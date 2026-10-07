//! `vet_app::ports::Store` and `ReviewStore`: a content directory plus one
//! SQLite file (WAL) accessed through SeaORM, both under the backend side's
//! data directory (§8).

mod codec;
mod content;
mod entity;
mod gc;
mod migration;
mod recent;
mod run;
mod servers;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, SqlxSqliteConnector, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use vet_app::pin::{BaseChoice, CommitId, Pin, PinId};
use vet_app::ports::{ReviewStore, Store, StoreError};
use vet_app::review::{NonTextAck, Session, SessionId, SpanId, Version};
use vet_core::content::{BlobId, Changeset, Hash, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::merkle::{self, TreeNode};
use vet_core::review::{Anchor, CommentTarget, Message, ReviewedSpan, ThreadId, ThreadStatus};

use codec::*;
use content::ContentStore;
use entity::*;

type Res<T> = Result<T, DbErr>;

#[derive(Clone)]
pub struct SqliteStore {
    db: DatabaseConnection,
    content: Arc<ContentStore>,
    dir: PathBuf,
}

fn db_err(e: impl std::fmt::Display) -> DbErr {
    DbErr::Custom(e.to_string())
}

const DATABASE: &str = "vet.sqlite";

async fn connect(dir: &Path) -> Result<DatabaseConnection, DbErr> {
    let options = SqliteConnectOptions::new()
        .filename(dir.join(DATABASE))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    // One connection serializes writers; WAL readers would not need it, but
    // nothing here is read-heavy.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(db_err)?;
    Ok(SqlxSqliteConnector::from_sqlx_sqlite_pool(pool))
}

/// SQLite allows 32766 bound variables per statement.
async fn insert_chunked<A>(db: &impl ConnectionTrait, rows: Vec<A>) -> Res<()>
where
    A: ActiveModelTrait + ActiveModelBehavior + Clone + Send,
    <A::Entity as EntityTrait>::Model: IntoActiveModel<A>,
{
    for chunk in rows.chunks(1000) {
        A::Entity::insert_many(chunk.to_vec()).exec(db).await?;
    }
    Ok(())
}

async fn put_anchor(db: &impl ConnectionTrait, session: i64, anchor: Option<&Anchor>) -> Res<Option<i64>> {
    match anchor {
        Some(a) => Ok(Some(anchor_row(session, a).insert(db).await?.id)),
        None => Ok(None),
    }
}

async fn write_filter(db: &impl ConnectionTrait, repo: &str, filter: &ReviewFilter) -> Res<()> {
    project_filter::ActiveModel {
        repo: Set(repo.to_owned()),
        hide_binary: Set(filter.hide_binary),
    }
    .insert(db)
    .await?;
    insert_chunked(db, filter_rows(repo, filter)).await
}

impl SqliteStore {
    /// The per-platform data directory; in WSL the Linux home, never `/mnt/*`.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("vet"))
    }

    /// A database written by another version of Vet (it applied migrations
    /// this one does not have) is deleted and created anew, losing its reviews.
    pub async fn open(dir: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir)?;
        let mut db = connect(dir).await?;
        if migration::is_foreign(&db).await? {
            db.close().await?;
            for suffix in ["", "-wal", "-shm"] {
                match std::fs::remove_file(dir.join(format!("{DATABASE}{suffix}"))) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
            db = connect(dir).await?;
        }
        migration::Migrator::up(&db, None).await?;
        let content = ContentStore::open(dir.join("content"))?;
        Ok(Self {
            db,
            content: Arc::new(content),
            dir: dir.to_owned(),
        })
    }

    /// The data directory the store lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&ContentStore) -> io::Result<T> + Send + 'static,
    ) -> Result<T, StoreError> {
        let content = self.content.clone();
        Ok(tokio::task::spawn_blocking(move || f(&content))
            .await
            .map_err(db_err)??)
    }

    async fn load_session(&self, id: SessionId) -> Res<Option<Session>> {
        let sid = id.0 as i64;
        let Some(row) = session::Entity::find_by_id(sid).one(&self.db).await? else {
            return Ok(None);
        };
        let thread_ids = thread::Entity::find()
            .filter(thread::Column::Session.eq(sid))
            .select_only()
            .column(thread::Column::Id);
        let rows = SessionRows {
            anchors: anchor::Entity::find()
                .filter(anchor::Column::Session.eq(sid))
                .all(&self.db)
                .await?,
            spans: span::Entity::find()
                .filter(span::Column::Session.eq(sid))
                .order_by_asc(span::Column::Id)
                .all(&self.db)
                .await?,
            acks: ack::Entity::find()
                .filter(ack::Column::Session.eq(sid))
                .order_by_asc(ack::Column::Id)
                .all(&self.db)
                .await?,
            messages: message::Entity::find()
                .filter(message::Column::Thread.in_subquery(thread_ids.into_query()))
                .order_by_asc(message::Column::Id)
                .all(&self.db)
                .await?,
            threads: thread::Entity::find()
                .filter(thread::Column::Session.eq(sid))
                .order_by_asc(thread::Column::Id)
                .all(&self.db)
                .await?,
            base: session_base::Entity::find_by_id(sid)
                .one(&self.db)
                .await?
                .ok_or_else(|| corrupt("session without a base"))?,
            session: row,
        };
        session_from(id, rows).map(Some)
    }

    async fn find_thread(db: &impl ConnectionTrait, id: ThreadId) -> Res<thread::Model> {
        thread::Entity::find_by_id(id.0 as i64)
            .one(db)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(format!("thread {}", id.0)))
    }
}

impl Store for SqliteStore {
    async fn put_blob(&self, bytes: Vec<u8>) -> Result<BlobId, StoreError> {
        self.blocking(move |c| c.put(&bytes)).await
    }

    async fn blob(&self, id: BlobId) -> Result<Option<Vec<u8>>, StoreError> {
        self.blocking(move |c| c.get(id)).await
    }

    async fn put_snapshot(&self, snapshot: Snapshot, status: SnapshotStatus, at: i64) -> Result<(), StoreError> {
        let (root, nodes) = merkle::build(&snapshot.entries);
        if root != snapshot.id {
            return Err(StoreError("snapshot id does not match its entries".into()));
        }
        let txn = self.db.begin().await?;
        let key = root.0.0.to_vec();
        if snapshot::Entity::find_by_id(key.clone()).one(&txn).await?.is_none() {
            snapshot::ActiveModel {
                id: Set(key.clone()),
                created: Set(at),
            }
            .insert(&txn)
            .await?;
            insert_chunked(&txn, status_rows(&key, &status)).await?;

            let hashes: Vec<Vec<u8>> = nodes.keys().map(|h| h.0.to_vec()).collect();
            let mut present = HashSet::new();
            for chunk in hashes.chunks(1000) {
                present.extend(
                    tree_node::Entity::find()
                        .filter(tree_node::Column::Id.is_in(chunk.to_vec()))
                        .all(&txn)
                        .await?
                        .into_iter()
                        .map(|n| n.id),
                );
            }
            let missing: Vec<_> = nodes
                .iter()
                .filter(|(h, _)| !present.contains(h.0.as_slice()))
                .collect();
            insert_chunked(
                &txn,
                missing
                    .iter()
                    .map(|(h, _)| tree_node::ActiveModel { id: Set(h.0.to_vec()) })
                    .collect(),
            )
            .await?;
            insert_chunked(&txn, missing.iter().flat_map(|(h, n)| child_rows(**h, n)).collect()).await?;
        }
        Ok(txn.commit().await?)
    }

    async fn snapshot(&self, id: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
        let key = id.0.0.to_vec();
        if snapshot::Entity::find_by_id(key.clone()).one(&self.db).await?.is_none() {
            return Ok(None);
        }
        // Load the tree level by level.
        let mut nodes: HashMap<Hash, TreeNode> = HashMap::new();
        let mut level = vec![id.0];
        while !level.is_empty() {
            let keys: Vec<Vec<u8>> = level.iter().map(|h| h.0.to_vec()).collect();
            let mut rows: HashMap<Vec<u8>, Vec<tree_child::Model>> = HashMap::new();
            for chunk in keys.chunks(1000) {
                for row in tree_child::Entity::find()
                    .filter(tree_child::Column::Node.is_in(chunk.to_vec()))
                    .all(&self.db)
                    .await?
                {
                    rows.entry(row.node.clone()).or_default().push(row);
                }
            }
            let mut next = Vec::new();
            for h in level {
                let node = node_from(rows.remove(h.0.as_slice()).unwrap_or_default())?;
                next.extend(node.dirs.values().copied());
                nodes.insert(h, node);
            }
            level = next;
        }
        let entries = merkle::entries(id, &|h| nodes.get(&h).cloned())
            .ok_or_else(|| StoreError("snapshot tree is incomplete".into()))?;
        let status = status_from(
            snapshot_path::Entity::find()
                .filter(snapshot_path::Column::Snapshot.eq(key))
                .order_by_asc(snapshot_path::Column::Id)
                .all(&self.db)
                .await?,
        );
        Ok(Some((Snapshot { id, entries }, status)))
    }

    async fn append_timeline(&self, session: SessionId, snapshot: SnapshotId, at: i64) -> Result<(), StoreError> {
        timeline::ActiveModel {
            session: Set(session.0 as i64),
            snapshot: Set(snapshot.0.0.to_vec()),
            at: Set(at),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    async fn timeline(&self, session: SessionId) -> Result<Vec<Version>, StoreError> {
        let rows = timeline::Entity::find()
            .filter(timeline::Column::Session.eq(session.0 as i64))
            .order_by_asc(timeline::Column::Id)
            .all(&self.db)
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                Ok(Version {
                    snapshot: SnapshotId(hash(&r.snapshot)?),
                    at: r.at,
                })
            })
            .collect::<Res<_>>()?)
    }
}

impl ReviewStore for SqliteStore {
    async fn put_pin(&self, head: CommitId, tree: SnapshotId) -> Result<Pin, StoreError> {
        let row = pin::ActiveModel {
            head: Set(head.0.clone()),
            tree: Set(tree.0.0.to_vec()),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;
        Ok(Pin {
            id: PinId(row.id as u64),
            head,
            tree,
        })
    }

    async fn pin(&self, id: PinId) -> Result<Option<Pin>, StoreError> {
        let Some(row) = pin::Entity::find_by_id(id.0 as i64).one(&self.db).await? else {
            return Ok(None);
        };
        Ok(Some(Pin {
            id,
            head: CommitId(row.head),
            tree: SnapshotId(hash(&row.tree)?),
        }))
    }

    async fn open_session(
        &self,
        repo: String,
        pin: PinId,
        changeset: Changeset,
        base: BaseChoice,
        base_commit: CommitId,
    ) -> Result<SessionId, StoreError> {
        let txn = self.db.begin().await?;
        let row = session::ActiveModel {
            repo: Set(repo),
            pin: Set(pin.0 as i64),
            base: Set(changeset.base.0.0.to_vec()),
            target: Set(changeset.target.0.0.to_vec()),
            complete: Set(false),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        base_row(row.id, base, base_commit).insert(&txn).await?;
        txn.commit().await?;
        Ok(SessionId(row.id as u64))
    }

    async fn close_session(&self, id: SessionId, now: i64, complete: bool) -> Result<(), StoreError> {
        session::ActiveModel {
            id: Set(id.0 as i64),
            closed_at: Set(Some(now)),
            complete: Set(complete),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn open_sessions(&self, repo: String) -> Result<Vec<SessionId>, StoreError> {
        let ids: Vec<i64> = session::Entity::find()
            .filter(session::Column::Repo.eq(repo))
            .filter(session::Column::ClosedAt.is_null())
            .order_by_asc(session::Column::Id)
            .select_only()
            .column(session::Column::Id)
            .into_tuple()
            .all(&self.db)
            .await?;
        Ok(ids.into_iter().map(|i| SessionId(i as u64)).collect())
    }

    async fn session(&self, id: SessionId) -> Result<Option<Session>, StoreError> {
        Ok(self.load_session(id).await?)
    }

    async fn filter(&self, repo: String) -> Result<ReviewFilter, StoreError> {
        let Some(row) = project_filter::Entity::find_by_id(repo.clone()).one(&self.db).await? else {
            return Ok(ReviewFilter::default());
        };
        let items = project_filter_item::Entity::find()
            .filter(project_filter_item::Column::Repo.eq(repo))
            .order_by_asc(project_filter_item::Column::Id)
            .all(&self.db)
            .await?;
        Ok(filter_from(row.hide_binary, items)?)
    }

    async fn set_filter(&self, repo: String, filter: ReviewFilter) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        project_filter::Entity::delete_by_id(repo.clone()).exec(&txn).await?;
        write_filter(&txn, &repo, &filter).await?;
        Ok(txn.commit().await?)
    }

    async fn set_base(
        &self,
        id: SessionId,
        base: SnapshotId,
        choice: BaseChoice,
        commit: CommitId,
    ) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let sid = id.0 as i64;
        session::ActiveModel {
            id: Set(sid),
            base: Set(base.0.0.to_vec()),
            ..Default::default()
        }
        .update(&txn)
        .await?;
        session_base::Entity::delete_by_id(sid).exec(&txn).await?;
        base_row(sid, choice, commit).insert(&txn).await?;
        Ok(txn.commit().await?)
    }

    async fn edit_spans(
        &self,
        id: SessionId,
        remove: Vec<SpanId>,
        add: Vec<ReviewedSpan>,
    ) -> Result<Vec<SpanId>, StoreError> {
        let txn = self.db.begin().await?;
        let sid = id.0 as i64;
        // Anchors are owned by their spans, so they go with them.
        let remove: Vec<i64> = remove.iter().map(|s| s.0 as i64).collect();
        let doomed = span::Entity::find()
            .filter(span::Column::Session.eq(sid))
            .filter(span::Column::Id.is_in(remove))
            .all(&txn)
            .await?;
        span::Entity::delete_many()
            .filter(span::Column::Id.is_in(doomed.iter().map(|s| s.id)))
            .exec(&txn)
            .await?;
        anchor::Entity::delete_many()
            .filter(anchor::Column::Id.is_in(doomed.iter().flat_map(|s| [s.base_anchor, s.target_anchor]).flatten()))
            .exec(&txn)
            .await?;
        let mut ids = Vec::new();
        for s in &add {
            let row = span::ActiveModel {
                session: Set(sid),
                state: Set(s.state.into()),
                base_anchor: Set(put_anchor(&txn, sid, s.base.as_ref()).await?),
                target_anchor: Set(put_anchor(&txn, sid, s.target.as_ref()).await?),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            ids.push(SpanId(row.id as u64));
        }
        txn.commit().await?;
        Ok(ids)
    }

    async fn acknowledge(&self, id: SessionId, ack: NonTextAck) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let sid = id.0 as i64;
        let existing = ack::Entity::find()
            .filter(ack::Column::Session.eq(sid))
            .filter(ack_filter(&ack))
            .one(&txn)
            .await?;
        if existing.is_none() {
            ack_row(sid, &ack).insert(&txn).await?;
        }
        Ok(txn.commit().await?)
    }

    async fn unacknowledge(&self, id: SessionId, ack: NonTextAck) -> Result<(), StoreError> {
        ack::Entity::delete_many()
            .filter(ack::Column::Session.eq(id.0 as i64))
            .filter(ack_filter(&ack))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    async fn add_thread(&self, id: SessionId, target: CommentTarget, opener: Message) -> Result<ThreadId, StoreError> {
        let txn = self.db.begin().await?;
        let sid = id.0 as i64;
        let anchor = match &target {
            CommentTarget::Range(a) => put_anchor(&txn, sid, Some(a)).await?,
            _ => None,
        };
        let mut row = thread::ActiveModel {
            session: Set(sid),
            status: Set(ThreadStatus::Open.into()),
            ..Default::default()
        };
        set_target(&mut row, &target, anchor);
        let thread = row.insert(&txn).await?.id;
        message::ActiveModel {
            thread: Set(thread),
            author: Set(opener.author),
            body: Set(opener.body),
            at: Set(opener.at),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        txn.commit().await?;
        Ok(ThreadId(thread as u64))
    }

    async fn add_message(&self, thread: ThreadId, m: Message) -> Result<(), StoreError> {
        Self::find_thread(&self.db, thread).await?;
        message::ActiveModel {
            thread: Set(thread.0 as i64),
            author: Set(m.author),
            body: Set(m.body),
            at: Set(m.at),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    async fn set_thread_status(&self, thread: ThreadId, status: ThreadStatus) -> Result<(), StoreError> {
        thread::ActiveModel {
            id: Set(thread.0 as i64),
            status: Set(status.into()),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn set_thread_target(&self, thread: ThreadId, target: CommentTarget) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let old = Self::find_thread(&txn, thread).await?;
        if let Some(a) = old.anchor {
            anchor::Entity::delete_by_id(a).exec(&txn).await?;
        }
        let anchor = match &target {
            CommentTarget::Range(a) => put_anchor(&txn, old.session, Some(a)).await?,
            _ => None,
        };
        let mut row = thread::ActiveModel {
            id: Set(old.id),
            ..Default::default()
        };
        set_target(&mut row, &target, anchor);
        row.update(&txn).await?;
        Ok(txn.commit().await?)
    }

    async fn delete_thread(&self, thread: ThreadId) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let old = Self::find_thread(&txn, thread).await?;
        thread::Entity::delete_by_id(old.id).exec(&txn).await?;
        if let Some(a) = old.anchor {
            anchor::Entity::delete_by_id(a).exec(&txn).await?;
        }
        Ok(txn.commit().await?)
    }
}
