//! `vet_app::ports::Store`, `ChangesetStore`, `ReviewStore` and `ThreadStore`: a
//! content directory plus one SQLite file (WAL) accessed through SeaORM, both
//! under the backend side's data directory (§8).

mod codec;
mod content;
mod entity;
mod gc;
mod migration;
mod recent;
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
use vet_app::changeset::{BaseChoice, ChangesetId, Lens, Source, Version};
use vet_app::ports::{ChangesetStore, ReviewStore, Store, StoreError, ThreadStore};
use vet_app::review::{ChangesetThread, NonTextAck, Review, ReviewId, SpanId, SpanOrigin};
use vet_core::content::{BlobId, Hash, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::merkle::{self, TreeNode};
use vet_core::review::{
    Anchor, CommentTarget, Message, MessageId, PlatformThread, ReviewedSpan, ThreadId, ThreadStatus,
};

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

/// Writes a version and the files it changed.
async fn insert_version(db: &impl ConnectionTrait, changeset: ChangesetId, version: &Version) -> Res<()> {
    let id = version_row(changeset, version).insert(db).await?.id;
    insert_chunked(db, version_file_rows(id, &version.change)).await
}

async fn put_anchor(db: &impl ConnectionTrait, changeset: i64, anchor: Option<&Anchor>) -> Res<Option<i64>> {
    match anchor {
        Some(a) => Ok(Some(anchor_row(changeset, a).insert(db).await?.id)),
        None => Ok(None),
    }
}

async fn find_changeset(db: &impl ConnectionTrait, repo: String, source: &Source) -> Res<Option<i64>> {
    let (kind, key) = source_columns(source);
    changeset::Entity::find()
        .filter(changeset::Column::Repo.eq(repo))
        .filter(changeset::Column::Kind.eq(kind))
        .filter(changeset::Column::Key.eq(key))
        .select_only()
        .column(changeset::Column::Id)
        .into_tuple()
        .one(db)
        .await
}

async fn insert_changeset(db: &impl ConnectionTrait, repo: String, source: &Source) -> Res<i64> {
    let (kind, key) = source_columns(source);
    Ok(changeset::ActiveModel {
        repo: Set(repo),
        kind: Set(kind),
        key: Set(key),
        ..Default::default()
    }
    .insert(db)
    .await?
    .id)
}

/// The anchors with these ids.
async fn find_anchors(db: &impl ConnectionTrait, ids: Vec<i64>) -> Res<Vec<anchor::Model>> {
    let mut found = Vec::new();
    for chunk in ids.chunks(1000) {
        found.extend(
            anchor::Entity::find()
                .filter(anchor::Column::Id.is_in(chunk.to_vec()))
                .all(db)
                .await?,
        );
    }
    Ok(found)
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

    async fn load_review(&self, id: ReviewId) -> Res<Option<Review>> {
        let rid = id.0 as i64;
        let Some(row) = review::Entity::find_by_id(rid).one(&self.db).await? else {
            return Ok(None);
        };
        let spans = span::Entity::find()
            .filter(span::Column::Review.eq(rid))
            .order_by_asc(span::Column::Id)
            .all(&self.db)
            .await?;
        let anchors = find_anchors(
            &self.db,
            spans
                .iter()
                .flat_map(|s| [s.base_anchor, s.target_anchor])
                .flatten()
                .collect(),
        )
        .await?;
        let rows = ReviewRows {
            review: row,
            anchors,
            spans,
            acks: ack::Entity::find()
                .filter(ack::Column::Review.eq(rid))
                .order_by_asc(ack::Column::Id)
                .all(&self.db)
                .await?,
        };
        review_from(rows).map(Some)
    }

    async fn load_threads(&self, changeset: ChangesetId) -> Res<Vec<ChangesetThread>> {
        let cid = changeset.0 as i64;
        let threads = thread::Entity::find()
            .filter(thread::Column::Changeset.eq(cid))
            .order_by_asc(thread::Column::Id)
            .all(&self.db)
            .await?;
        let anchors = find_anchors(&self.db, threads.iter().filter_map(|t| t.anchor).collect()).await?;
        let thread_ids = thread::Entity::find()
            .filter(thread::Column::Changeset.eq(cid))
            .select_only()
            .column(thread::Column::Id);
        let messages = message::Entity::find()
            .filter(message::Column::Thread.in_subquery(thread_ids.into_query()))
            .order_by_asc(message::Column::Id)
            .all(&self.db)
            .await?;
        threads_from(threads, anchors, messages)
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
}

impl ChangesetStore for SqliteStore {
    async fn changeset(&self, repo: String, source: Source) -> Result<Option<ChangesetId>, StoreError> {
        Ok(find_changeset(&self.db, repo, &source)
            .await?
            .map(|id| ChangesetId(id as u64)))
    }

    async fn lens(&self, repo: String, source: Source) -> Result<Option<Lens>, StoreError> {
        let (kind, key) = source_columns(&source);
        let row = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo))
            .filter(changeset::Column::Kind.eq(kind))
            .filter(changeset::Column::Key.eq(key))
            .one(&self.db)
            .await?;
        Ok(row.and_then(|r| r.lens).map(Into::into))
    }

    async fn set_lens(&self, repo: String, source: Source, lens: Lens) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => {
                changeset::ActiveModel {
                    id: Set(id),
                    lens: Set(Some(lens.into())),
                    ..Default::default()
                }
                .update(&txn)
                .await?;
            }
            None => {
                let (kind, key) = source_columns(&source);
                changeset::ActiveModel {
                    repo: Set(repo),
                    kind: Set(kind),
                    key: Set(key),
                    lens: Set(Some(lens.into())),
                    ..Default::default()
                }
                .insert(&txn)
                .await?;
            }
        }
        Ok(txn.commit().await?)
    }

    async fn ensure_changeset(&self, repo: String, source: Source) -> Result<ChangesetId, StoreError> {
        let txn = self.db.begin().await?;
        let id = match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => id,
            None => insert_changeset(&txn, repo, &source).await?,
        };
        txn.commit().await?;
        Ok(ChangesetId(id as u64))
    }

    async fn open_changesets(&self) -> Result<Vec<ChangesetId>, StoreError> {
        let ids: Vec<i64> = review::Entity::find()
            .filter(review::Column::ClosedAt.is_null())
            .select_only()
            .column(review::Column::Changeset)
            .distinct()
            .order_by_asc(review::Column::Changeset)
            .into_tuple()
            .all(&self.db)
            .await?;
        Ok(ids.into_iter().map(|id| ChangesetId(id as u64)).collect())
    }

    async fn versions(&self, changeset: ChangesetId) -> Result<Vec<Version>, StoreError> {
        let rows = version::Entity::find()
            .filter(version::Column::Changeset.eq(changeset.0 as i64))
            .order_by_asc(version::Column::N)
            .all(&self.db)
            .await?;
        let mut files: HashMap<i64, Vec<version_file::Model>> = HashMap::new();
        for file in version_file::Entity::find()
            .filter(version_file::Column::Version.is_in(rows.iter().map(|r| r.id)))
            .order_by_asc(version_file::Column::Id)
            .all(&self.db)
            .await?
        {
            files.entry(file.version).or_default().push(file);
        }
        let versions = rows.into_iter().map(|r| {
            let files = files.remove(&r.id).unwrap_or_default();
            version_from(r, files)
        });
        Ok(versions.collect::<Res<_>>()?)
    }

    async fn append_version(&self, changeset: ChangesetId, version: Version) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        insert_version(&txn, changeset, &version).await?;
        Ok(txn.commit().await?)
    }
}

impl ReviewStore for SqliteStore {
    async fn start_review(
        &self,
        repo: String,
        source: Source,
        base: BaseChoice,
        first: Version,
        now: i64,
    ) -> Result<(ChangesetId, ReviewId), StoreError> {
        let txn = self.db.begin().await?;
        let changeset = match find_changeset(&txn, repo.clone(), &source).await? {
            Some(id) => id,
            None => insert_changeset(&txn, repo, &source).await?,
        };
        version::Entity::delete_many()
            .filter(version::Column::Changeset.eq(changeset))
            .exec(&txn)
            .await?;
        insert_version(&txn, ChangesetId(changeset as u64), &first).await?;
        let (base_kind, base_value) = base_columns(base);
        let review = review::ActiveModel {
            changeset: Set(changeset),
            base_kind: Set(base_kind),
            base_value: Set(base_value),
            opened_at: Set(now),
            complete: Set(false),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        txn.commit().await?;
        Ok((ChangesetId(changeset as u64), ReviewId(review.id as u64)))
    }

    async fn close_review(&self, id: ReviewId, now: i64, complete: bool) -> Result<(), StoreError> {
        review::ActiveModel {
            id: Set(id.0 as i64),
            closed_at: Set(Some(now)),
            complete: Set(complete),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn open_review(&self, changeset: ChangesetId) -> Result<Option<ReviewId>, StoreError> {
        let id: Option<i64> = review::Entity::find()
            .filter(review::Column::Changeset.eq(changeset.0 as i64))
            .filter(review::Column::ClosedAt.is_null())
            .order_by_desc(review::Column::Id)
            .select_only()
            .column(review::Column::Id)
            .into_tuple()
            .one(&self.db)
            .await?;
        Ok(id.map(|i| ReviewId(i as u64)))
    }

    async fn latest_review(&self, changeset: ChangesetId) -> Result<Option<ReviewId>, StoreError> {
        let id: Option<i64> = review::Entity::find()
            .filter(review::Column::Changeset.eq(changeset.0 as i64))
            .order_by_desc(review::Column::Id)
            .select_only()
            .column(review::Column::Id)
            .into_tuple()
            .one(&self.db)
            .await?;
        Ok(id.map(|i| ReviewId(i as u64)))
    }

    async fn review(&self, id: ReviewId) -> Result<Option<Review>, StoreError> {
        Ok(self.load_review(id).await?)
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

    async fn browsing_base(&self, repo: String) -> Result<Option<BaseChoice>, StoreError> {
        let row = project_base::Entity::find_by_id(repo).one(&self.db).await?;
        Ok(row.map(|r| base_from(r.base_kind, r.base_value)))
    }

    async fn set_browsing_base(&self, repo: String, choice: BaseChoice) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        project_base::Entity::delete_by_id(repo.clone()).exec(&txn).await?;
        let (base_kind, base_value) = base_columns(choice);
        project_base::ActiveModel {
            repo: Set(repo),
            base_kind: Set(base_kind),
            base_value: Set(base_value),
        }
        .insert(&txn)
        .await?;
        Ok(txn.commit().await?)
    }

    async fn set_base(&self, id: ReviewId, choice: BaseChoice) -> Result<(), StoreError> {
        let (kind, value) = base_columns(choice);
        review::ActiveModel {
            id: Set(id.0 as i64),
            base_kind: Set(kind),
            base_value: Set(value),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn set_last_reviewed(&self, id: ReviewId, version: u32) -> Result<(), StoreError> {
        review::ActiveModel {
            id: Set(id.0 as i64),
            last_reviewed: Set(Some(version.into())),
            ..Default::default()
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    async fn edit_spans(
        &self,
        id: ReviewId,
        remove: Vec<SpanId>,
        add: Vec<(ReviewedSpan, SpanOrigin)>,
    ) -> Result<Vec<SpanId>, StoreError> {
        let txn = self.db.begin().await?;
        let rid = id.0 as i64;
        let changeset = review::Entity::find_by_id(rid)
            .one(&txn)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(format!("review {}", id.0)))?
            .changeset;
        // Anchors are owned by their spans, so they go with them.
        let remove: Vec<i64> = remove.iter().map(|s| s.0 as i64).collect();
        let doomed = span::Entity::find()
            .filter(span::Column::Review.eq(rid))
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
        for (s, origin) in &add {
            let row = span::ActiveModel {
                review: Set(rid),
                state: Set(s.state.into()),
                base_anchor: Set(put_anchor(&txn, changeset, s.base.as_ref()).await?),
                target_anchor: Set(put_anchor(&txn, changeset, s.target.as_ref()).await?),
                origin_commit: Set(match origin {
                    SpanOrigin::Squash => None,
                    SpanOrigin::Commit(c) => Some(c.0.clone()),
                }),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            ids.push(SpanId(row.id as u64));
        }
        txn.commit().await?;
        Ok(ids)
    }

    async fn acknowledge(&self, id: ReviewId, ack: NonTextAck) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let rid = id.0 as i64;
        let existing = ack::Entity::find()
            .filter(ack::Column::Review.eq(rid))
            .filter(ack_filter(&ack))
            .one(&txn)
            .await?;
        if existing.is_none() {
            ack_row(rid, &ack).insert(&txn).await?;
        }
        Ok(txn.commit().await?)
    }

    async fn unacknowledge(&self, id: ReviewId, ack: NonTextAck) -> Result<(), StoreError> {
        ack::Entity::delete_many()
            .filter(ack::Column::Review.eq(id.0 as i64))
            .filter(ack_filter(&ack))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}

impl ThreadStore for SqliteStore {
    async fn threads(&self, changeset: ChangesetId) -> Result<Vec<ChangesetThread>, StoreError> {
        Ok(self.load_threads(changeset).await?)
    }

    async fn add_thread(
        &self,
        changeset: ChangesetId,
        review: Option<ReviewId>,
        target: CommentTarget,
        opener: Message,
        platform: Option<PlatformThread>,
        status: ThreadStatus,
    ) -> Result<(ThreadId, MessageId), StoreError> {
        let txn = self.db.begin().await?;
        let cid = changeset.0 as i64;
        let anchor = match &target {
            CommentTarget::Range(a) => put_anchor(&txn, cid, Some(a)).await?,
            _ => None,
        };
        let mut row = thread::ActiveModel {
            changeset: Set(cid),
            review: Set(review.map(|r| r.0 as i64)),
            status: Set(status.into()),
            ..Default::default()
        };
        set_target(&mut row, &target, anchor);
        set_platform(&mut row, platform.as_ref());
        let thread = row.insert(&txn).await?.id;
        let message = message_row(thread, &opener).insert(&txn).await?.id;
        txn.commit().await?;
        Ok((ThreadId(thread as u64), MessageId(message as u64)))
    }

    async fn add_message(&self, thread: ThreadId, m: Message) -> Result<MessageId, StoreError> {
        Self::find_thread(&self.db, thread).await?;
        let row = message_row(thread.0 as i64, &m).insert(&self.db).await?;
        Ok(MessageId(row.id as u64))
    }

    async fn replace_message(&self, m: Message) -> Result<(), StoreError> {
        let old = message::Entity::find_by_id(m.id.0 as i64)
            .one(&self.db)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(format!("message {}", m.id.0)))?;
        let mut row = message_row(old.thread, &m);
        row.id = Set(old.id);
        row.update(&self.db).await?;
        Ok(())
    }

    async fn delete_message(&self, message: MessageId) -> Result<(), StoreError> {
        message::Entity::delete_by_id(message.0 as i64).exec(&self.db).await?;
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

    async fn set_thread_platform(&self, thread: ThreadId, platform: PlatformThread) -> Result<(), StoreError> {
        let mut row = thread::ActiveModel {
            id: Set(thread.0 as i64),
            ..Default::default()
        };
        set_platform(&mut row, Some(&platform));
        row.update(&self.db).await?;
        Ok(())
    }

    async fn set_thread_target(&self, thread: ThreadId, target: CommentTarget) -> Result<(), StoreError> {
        let txn = self.db.begin().await?;
        let old = Self::find_thread(&txn, thread).await?;
        if let Some(a) = old.anchor {
            anchor::Entity::delete_by_id(a).exec(&txn).await?;
        }
        let anchor = match &target {
            CommentTarget::Range(a) => put_anchor(&txn, old.changeset, Some(a)).await?,
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
