//! `vet_app::ports::Store` and `ReviewStore`: a content directory plus one
//! SQLite file (WAL) accessed through SeaORM, both under the backend side's
//! data directory (§8).

mod codec;
mod content;
mod entity;
mod migration;
mod run;
mod servers;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, SqlxSqliteConnector, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use vet_app::pin::{BaseChoice, CommitId, Pin, PinId};
use vet_app::ports::{GcReport, ReviewStore, Store, StoreError};
use vet_app::review::{NonTextAck, RecordedSpan, Session, SessionId, SpanId, Version};
use vet_core::content::{BlobId, Changeset, Hash, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::merkle::{self, TreeNode};
use vet_core::platform::PlatformThread;
use vet_core::review::{Anchor, CommentTarget, Message, ReviewedSpan, Thread, ThreadId, ThreadStatus};

use codec::*;
use content::ContentStore;
use entity::kinds::{BaseKind, FilterKind};
use entity::*;

type Res<T> = Result<T, DbErr>;

#[derive(Clone, Copy, Debug)]
pub struct StoreConfig {
    /// How long a closed session is kept (§8).
    pub closed_retention: Duration,
    /// Content and snapshots younger than this are never collected, so a blob
    /// written just before the metadata that references it survives a GC.
    pub gc_grace: Duration,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            closed_retention: Duration::from_secs(30 * 24 * 3600),
            gc_grace: Duration::from_secs(3600),
        }
    }
}

#[derive(Clone)]
pub struct SqliteStore {
    db: DatabaseConnection,
    content: Arc<ContentStore>,
    config: StoreConfig,
    dir: PathBuf,
}

fn db_err(e: impl std::fmt::Display) -> DbErr {
    DbErr::Custom(e.to_string())
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
    let item = |kind: FilterKind, value: Vec<u8>| project_filter_item::ActiveModel {
        repo: Set(repo.to_owned()),
        kind: Set(kind),
        value: Set(value),
        ..Default::default()
    };
    let rows = filter
        .roots
        .iter()
        .map(|r| item(FilterKind::Root, r.0.clone()))
        .chain(
            filter
                .include
                .iter()
                .map(|s| item(FilterKind::Include, s.clone().into_bytes())),
        )
        .chain(
            filter
                .exclude
                .iter()
                .map(|s| item(FilterKind::Exclude, s.clone().into_bytes())),
        )
        .collect();
    insert_chunked(db, rows).await
}

fn read_filter(hide_binary: bool, items: Vec<project_filter_item::Model>) -> Res<ReviewFilter> {
    let mut filter = ReviewFilter {
        hide_binary,
        ..Default::default()
    };
    for item in items {
        let text = |v: Vec<u8>| String::from_utf8(v).map_err(db_err);
        match item.kind {
            FilterKind::Root => filter.roots.push(vet_core::content::RepoPath(item.value)),
            FilterKind::Include => filter.include.push(text(item.value)?),
            FilterKind::Exclude => filter.exclude.push(text(item.value)?),
        }
    }
    Ok(filter)
}

impl SqliteStore {
    /// The per-platform data directory; in WSL the Linux home, never `/mnt/*`.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("vet"))
    }

    pub async fn open(dir: &Path, config: StoreConfig) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir)?;
        let options = SqliteConnectOptions::new()
            .filename(dir.join("vet.sqlite"))
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
        let db = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
        migration::Migrator::up(&db, None).await?;
        let content = ContentStore::open(dir.join("content"))?;
        Ok(Self {
            db,
            content: Arc::new(content),
            config,
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
        let anchors: HashMap<i64, anchor::Model> = anchor::Entity::find()
            .filter(anchor::Column::Session.eq(sid))
            .all(&self.db)
            .await?
            .into_iter()
            .map(|a| (a.id, a))
            .collect();
        let anchor_of = |id: Option<i64>| id.and_then(|i| anchors.get(&i)).map(anchor_from).transpose();

        let spans = span::Entity::find()
            .filter(span::Column::Session.eq(sid))
            .order_by_asc(span::Column::Id)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|s| {
                let span = ReviewedSpan {
                    base: anchor_of(s.base_anchor)?,
                    target: anchor_of(s.target_anchor)?,
                    state: s.state.into(),
                };
                Ok(RecordedSpan {
                    id: SpanId(s.id as u64),
                    span,
                })
            })
            .collect::<Res<_>>()?;
        let acks = ack::Entity::find()
            .filter(ack::Column::Session.eq(sid))
            .order_by_asc(ack::Column::Id)
            .all(&self.db)
            .await?
            .into_iter()
            .map(ack_from)
            .collect::<Res<_>>()?;

        let mut messages: HashMap<i64, Vec<Message>> = HashMap::new();
        let thread_ids = thread::Entity::find()
            .filter(thread::Column::Session.eq(sid))
            .select_only()
            .column(thread::Column::Id);
        for m in message::Entity::find()
            .filter(message::Column::Thread.in_subquery(thread_ids.into_query()))
            .order_by_asc(message::Column::Id)
            .all(&self.db)
            .await?
        {
            messages.entry(m.thread).or_default().push(Message {
                author: m.author,
                body: m.body,
                at: m.at,
            });
        }
        let threads = thread::Entity::find()
            .filter(thread::Column::Session.eq(sid))
            .order_by_asc(thread::Column::Id)
            .all(&self.db)
            .await?
            .into_iter()
            .map(|t| {
                let platform = t.remote_id.clone().map(|remote_id| PlatformThread {
                    remote_id,
                    remote_status: t.remote_status.clone(),
                    resolvable: t.remote_resolvable.unwrap_or(false),
                    blocks_merge: t.remote_blocks_merge.unwrap_or(false),
                });
                Ok(Thread {
                    id: ThreadId(t.id as u64),
                    target: target_from(&t, t.anchor.and_then(|a| anchors.get(&a)))?,
                    messages: messages.remove(&t.id).unwrap_or_default(),
                    status: t.status.into(),
                    platform,
                })
            })
            .collect::<Res<_>>()?;

        let stored_base = session_base::Entity::find_by_id(sid).one(&self.db).await?;
        let base_commit = stored_base.as_ref().and_then(|b| b.commit.clone()).map(CommitId);
        let pin = pin::Entity::find_by_id(row.pin).one(&self.db).await?;
        let base = match stored_base {
            Some(b) => match b.kind {
                BaseKind::Branch => BaseChoice::Branch { name: b.value },
                BaseKind::MergeBase => BaseChoice::MergeBase { reference: b.value },
                BaseKind::Commit => BaseChoice::Commit { id: CommitId(b.value) },
            },
            // Stored before branches: the base was the pin's `HEAD`.
            None => BaseChoice::Commit {
                id: CommitId(pin.map(|p| p.head).unwrap_or_default()),
            },
        };

        Ok(Some(Session {
            id,
            pin: PinId(row.pin as u64),
            changeset: Changeset {
                base: SnapshotId(hash(&row.base)?),
                target: SnapshotId(hash(&row.target)?),
            },
            base,
            base_commit,
            spans,
            acks,
            threads,
            closed_at: row.closed_at,
            complete: row.complete,
        }))
    }

    async fn find_thread(db: &impl ConnectionTrait, id: ThreadId) -> Res<thread::Model> {
        thread::Entity::find_by_id(id.0 as i64)
            .one(db)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(format!("thread {}", id.0)))
    }

    async fn gc_metadata(&self, now: i64) -> Res<(GcReport, HashSet<BlobId>)> {
        let cutoff = now - self.config.closed_retention.as_secs() as i64;
        let grace = now - self.config.gc_grace.as_secs() as i64;
        let txn = self.db.begin().await?;

        // Expired closed sessions, unless an open thread keeps them.
        let expired: Vec<i64> = session::Entity::find()
            .filter(session::Column::ClosedAt.lte(cutoff))
            .select_only()
            .column(session::Column::Id)
            .into_tuple()
            .all(&txn)
            .await?;
        let kept: HashSet<i64> = thread::Entity::find()
            .filter(thread::Column::Session.is_in(expired.clone()))
            .filter(thread::Column::Status.eq(kinds::ThreadStatus::Open))
            .select_only()
            .column(thread::Column::Session)
            .into_tuple()
            .all(&txn)
            .await?
            .into_iter()
            .collect();
        let dead: Vec<i64> = expired.into_iter().filter(|id| !kept.contains(id)).collect();
        let sessions_deleted = session::Entity::delete_many()
            .filter(session::Column::Id.is_in(dead))
            .exec(&txn)
            .await?
            .rows_affected as usize;

        let pins: Vec<i64> = session::Entity::find()
            .select_only()
            .column(session::Column::Pin)
            .into_tuple()
            .all(&txn)
            .await?;
        pin::Entity::delete_many()
            .filter(pin::Column::Id.is_not_in(pins))
            .exec(&txn)
            .await?;

        // Snapshots nothing retained refers to.
        let mut live: HashSet<Vec<u8>> = HashSet::new();
        live.extend(
            session::Entity::find()
                .select_only()
                .column(session::Column::Base)
                .into_tuple::<Vec<u8>>()
                .all(&txn)
                .await?,
        );
        live.extend(
            session::Entity::find()
                .select_only()
                .column(session::Column::Target)
                .into_tuple::<Vec<u8>>()
                .all(&txn)
                .await?,
        );
        live.extend(
            pin::Entity::find()
                .select_only()
                .column(pin::Column::Tree)
                .into_tuple::<Vec<u8>>()
                .all(&txn)
                .await?,
        );
        live.extend(
            timeline::Entity::find()
                .select_only()
                .column(timeline::Column::Snapshot)
                .into_tuple::<Vec<u8>>()
                .all(&txn)
                .await?,
        );
        live.extend(
            anchor::Entity::find()
                .select_only()
                .column(anchor::Column::Snapshot)
                .into_tuple::<Vec<u8>>()
                .all(&txn)
                .await?,
        );
        live.extend(run::approved_baselines(&txn).await?);
        let candidates: Vec<Vec<u8>> = snapshot::Entity::find()
            .filter(snapshot::Column::Created.lte(grace))
            .select_only()
            .column(snapshot::Column::Id)
            .into_tuple()
            .all(&txn)
            .await?;
        let unreferenced: Vec<Vec<u8>> = candidates.into_iter().filter(|id| !live.contains(id)).collect();
        let mut snapshots_deleted = 0;
        for chunk in unreferenced.chunks(1000) {
            snapshots_deleted += snapshot::Entity::delete_many()
                .filter(snapshot::Column::Id.is_in(chunk.to_vec()))
                .exec(&txn)
                .await?
                .rows_affected as usize;
        }

        // Mark nodes reachable from the remaining roots, sweep the rest.
        let roots: Vec<Vec<u8>> = snapshot::Entity::find()
            .select_only()
            .column(snapshot::Column::Id)
            .into_tuple()
            .all(&txn)
            .await?;
        let edges: Vec<(Vec<u8>, Vec<u8>)> = tree_child::Entity::find()
            .filter(tree_child::Column::Kind.eq(kinds::ChildKind::Dir))
            .select_only()
            .column(tree_child::Column::Node)
            .column(tree_child::Column::Dir)
            .into_tuple()
            .all(&txn)
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
            .all(&txn)
            .await?;
        let unreachable: Vec<Vec<u8>> = nodes.into_iter().filter(|n| !reachable.contains(n)).collect();
        let mut nodes_deleted = 0;
        for chunk in unreachable.chunks(1000) {
            nodes_deleted += tree_node::Entity::delete_many()
                .filter(tree_node::Column::Id.is_in(chunk.to_vec()))
                .exec(&txn)
                .await?
                .rows_affected as usize;
        }

        let blobs: Vec<Vec<u8>> = tree_child::Entity::find()
            .filter(tree_child::Column::Blob.is_not_null())
            .select_only()
            .column(tree_child::Column::Blob)
            .distinct()
            .into_tuple()
            .all(&txn)
            .await?;
        let blobs = blobs.iter().map(|b| hash(b).map(BlobId)).collect::<Res<HashSet<_>>>()?;
        txn.commit().await?;
        Ok((
            GcReport {
                sessions_deleted,
                snapshots_deleted,
                nodes_deleted,
                blobs_deleted: 0,
            },
            blobs,
        ))
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
            at: Set(Some(at)),
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
                    at: r.at.unwrap_or(0),
                })
            })
            .collect::<Res<_>>()?)
    }

    async fn gc(&self, now: i64) -> Result<GcReport, StoreError> {
        let (mut report, live) = self.gc_metadata(now).await?;
        let at = UNIX_EPOCH + Duration::from_secs(now.max(0) as u64);
        let grace = self.config.gc_grace;
        report.blobs_deleted = self.blocking(move |c| c.sweep(&live, at, grace)).await?;
        Ok(report)
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

    async fn open_session(&self, repo: String, pin: PinId, changeset: Changeset) -> Result<SessionId, StoreError> {
        let row = session::ActiveModel {
            repo: Set(Some(repo)),
            pin: Set(pin.0 as i64),
            base: Set(changeset.base.0.0.to_vec()),
            target: Set(changeset.target.0.0.to_vec()),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;
        Ok(SessionId(row.id as u64))
    }

    async fn close_session(&self, id: SessionId, now: i64, complete: bool) -> Result<(), StoreError> {
        session::ActiveModel {
            id: Set(id.0 as i64),
            closed_at: Set(Some(now)),
            complete: Set(Some(complete)),
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
        Ok(read_filter(row.hide_binary, items)?)
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
        let (kind, value) = match choice {
            BaseChoice::Branch { name } => (BaseKind::Branch, name),
            BaseChoice::MergeBase { reference } => (BaseKind::MergeBase, reference),
            BaseChoice::Commit { id } => (BaseKind::Commit, id.0),
        };
        session_base::ActiveModel {
            session: Set(sid),
            kind: Set(kind),
            value: Set(value),
            commit: Set(Some(commit.0)),
        }
        .insert(&txn)
        .await?;
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

    async fn set_thread_platform(&self, thread: ThreadId, platform: Option<PlatformThread>) -> Result<(), StoreError> {
        thread::ActiveModel {
            id: Set(thread.0 as i64),
            remote_id: Set(platform.as_ref().map(|p| p.remote_id.clone())),
            remote_status: Set(platform.as_ref().and_then(|p| p.remote_status.clone())),
            remote_resolvable: Set(platform.as_ref().map(|p| p.resolvable)),
            remote_blocks_merge: Set(platform.as_ref().map(|p| p.blocks_merge)),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
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
