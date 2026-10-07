//! `vet_app::ports::GcStore`: the SQL behind garbage collection (§8). What to
//! delete is decided by `vet_app::gc`.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, UNIX_EPOCH};

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, TransactionTrait};
use vet_app::gc::{GcFacts, GcPlan, SessionFacts};
use vet_app::pin::PinId;
use vet_app::ports::{GcReport, GcStore, StoreError};
use vet_app::review::SessionId;
use vet_core::content::{BlobId, SnapshotId};

use super::codec::hash;
use super::entity::*;
use super::{Res, SqliteStore};

/// SQLite allows 32766 bound variables per statement.
const CHUNK: usize = 1000;

fn snapshot_id(bytes: &[u8]) -> Res<SnapshotId> {
    hash(bytes).map(SnapshotId)
}

async fn facts(db: &impl ConnectionTrait) -> Res<GcFacts> {
    let mut snapshots_of: HashMap<i64, Vec<SnapshotId>> = HashMap::new();
    let pairs: Vec<(i64, Vec<u8>)> = timeline::Entity::find()
        .select_only()
        .columns([timeline::Column::Session, timeline::Column::Snapshot])
        .into_tuple()
        .all(db)
        .await?;
    let anchors: Vec<(i64, Vec<u8>)> = anchor::Entity::find()
        .select_only()
        .columns([anchor::Column::Session, anchor::Column::Snapshot])
        .into_tuple()
        .all(db)
        .await?;
    for (session, snapshot) in pairs.into_iter().chain(anchors) {
        snapshots_of.entry(session).or_default().push(snapshot_id(&snapshot)?);
    }
    let with_open_thread: HashSet<i64> = thread::Entity::find()
        .filter(thread::Column::Status.eq(kinds::ThreadStatus::Open))
        .select_only()
        .column(thread::Column::Session)
        .into_tuple()
        .all(db)
        .await?
        .into_iter()
        .collect();

    let mut sessions = Vec::new();
    for row in session::Entity::find().all(db).await? {
        let mut snapshots = snapshots_of.remove(&row.id).unwrap_or_default();
        snapshots.extend([snapshot_id(&row.base)?, snapshot_id(&row.target)?]);
        sessions.push(SessionFacts {
            id: SessionId(row.id as u64),
            pin: PinId(row.pin as u64),
            closed_at: row.closed_at,
            has_open_thread: with_open_thread.contains(&row.id),
            snapshots,
        });
    }
    let pins = pin::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|p| Ok((PinId(p.id as u64), snapshot_id(&p.tree)?)))
        .collect::<Res<_>>()?;
    let snapshots = snapshot::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|s| Ok((snapshot_id(&s.id)?, s.created)))
        .collect::<Res<_>>()?;
    Ok(GcFacts {
        sessions,
        pins,
        snapshots,
    })
}

/// Deletes the rows of `E` whose `key` is in `ids`; returns how many went.
async fn delete_in<E, V>(db: &impl ConnectionTrait, key: E::Column, ids: Vec<V>) -> Res<usize>
where
    E: EntityTrait,
    V: Into<sea_orm::Value> + Clone,
{
    let mut deleted = 0;
    for chunk in ids.chunks(CHUNK) {
        deleted += E::delete_many()
            .filter(key.is_in(chunk.to_vec()))
            .exec(db)
            .await?
            .rows_affected as usize;
    }
    Ok(deleted)
}

/// Deletes the tree nodes no snapshot reaches; returns how many went.
async fn sweep_nodes(db: &impl ConnectionTrait) -> Res<usize> {
    let roots: Vec<Vec<u8>> = snapshot::Entity::find()
        .select_only()
        .column(snapshot::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    let edges: Vec<(Vec<u8>, Vec<u8>)> = tree_child::Entity::find()
        .filter(tree_child::Column::Kind.eq(kinds::ChildKind::Dir))
        .select_only()
        .column(tree_child::Column::Node)
        .column(tree_child::Column::Dir)
        .into_tuple()
        .all(db)
        .await?;
    let mut children: HashMap<Vec<u8>, Vec<Vec<u8>>> = HashMap::new();
    for (node, dir) in edges {
        children.entry(node).or_default().push(dir);
    }
    let mut reachable: HashSet<Vec<u8>> = HashSet::new();
    let mut stack = roots;
    while let Some(n) = stack.pop() {
        if reachable.insert(n.clone()) {
            stack.extend(children.remove(&n).unwrap_or_default());
        }
    }
    let nodes: Vec<Vec<u8>> = tree_node::Entity::find()
        .select_only()
        .column(tree_node::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    let unreachable = nodes.into_iter().filter(|n| !reachable.contains(n)).collect();
    delete_in::<tree_node::Entity, _>(db, tree_node::Column::Id, unreachable).await
}

/// The content the remaining trees refer to.
async fn referenced_blobs(db: &impl ConnectionTrait) -> Res<HashSet<BlobId>> {
    let blobs: Vec<Vec<u8>> = tree_child::Entity::find()
        .filter(tree_child::Column::Blob.is_not_null())
        .select_only()
        .column(tree_child::Column::Blob)
        .distinct()
        .into_tuple()
        .all(db)
        .await?;
    blobs.iter().map(|b| hash(b).map(BlobId)).collect()
}

impl SqliteStore {
    /// Applies `decide` to the facts and deletes, in one transaction. Returns
    /// the report so far, the content to keep, and the content grace period.
    async fn gc_metadata(
        &self,
        decide: &(dyn Fn(&GcFacts) -> GcPlan + Send + Sync),
    ) -> Res<(GcReport, HashSet<BlobId>, Duration)> {
        let txn = self.db.begin().await?;
        let plan = decide(&facts(&txn).await?);
        let sessions_deleted = delete_in::<session::Entity, _>(
            &txn,
            session::Column::Id,
            plan.sessions.iter().map(|s| s.0 as i64).collect(),
        )
        .await?;
        delete_in::<pin::Entity, _>(&txn, pin::Column::Id, plan.pins.iter().map(|p| p.0 as i64).collect()).await?;
        let snapshots_deleted = delete_in::<snapshot::Entity, _>(
            &txn,
            snapshot::Column::Id,
            plan.snapshots.iter().map(|s| s.0.0.to_vec()).collect(),
        )
        .await?;
        let nodes_deleted = sweep_nodes(&txn).await?;
        let live = referenced_blobs(&txn).await?;
        txn.commit().await?;
        let report = GcReport {
            sessions_deleted,
            snapshots_deleted,
            nodes_deleted,
            blobs_deleted: 0,
        };
        Ok((report, live, plan.content_grace))
    }
}

impl GcStore for SqliteStore {
    async fn gc(&self, now: i64, decide: &(dyn Fn(&GcFacts) -> GcPlan + Send + Sync)) -> Result<GcReport, StoreError> {
        let (mut report, live, grace) = self.gc_metadata(decide).await?;
        let at = UNIX_EPOCH + Duration::from_secs(now.max(0) as u64);
        report.blobs_deleted = self.blocking(move |c| c.sweep(&live, at, grace)).await?;
        Ok(report)
    }
}
