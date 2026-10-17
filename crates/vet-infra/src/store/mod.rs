//! `vet_app::ports::Store`, `ChangesetStore`, `ReviewStore` and `ThreadStore` (reads) and
//! `ReviewWrites` (on [`ReviewWriter`]): two SQLite files (WAL) under the backend side's data
//! directory (§9). `vet.sqlite` holds the index and is accessed through SeaORM; `content.sqlite`
//! holds the content, one row per blob (see [`content`]).
//!
//! `vet.sqlite` is reached through two kinds of connection. Every write goes through the one
//! [`ReviewWriter`] connection, so writers queue in the order they asked. Every read goes
//! through a [`StoreReader`] pool of read-only connections. In WAL mode a reader never
//! waits for the writer, and the writer never waits for a reader, so a long read (a snapshot
//! loaded level by level, the listing of a project) and a mark are independent.

mod codec;
mod content;
mod entity;
mod gc;
mod migration;
mod projects;
mod recent;
mod servers;
mod settings;
#[cfg(test)]
mod tests;
mod writes;

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    DbErr, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, SqlxSqliteConnector,
    TransactionTrait,
};
use sea_orm_migration::MigratorTrait;
use vet_app::changeset::{ChangesetId, CommitId, Lens, Source, Version};
use vet_app::platform::{PullRequestHead, RemoteKey, StoredDiscussion};
use vet_app::ports::{
    ChangesetStore, LiveStore, ReviewRow, ReviewStore, SourceReview, Store, StoreError, StoredBlob, ThreadStore,
};
use vet_app::review::{ChangesetThread, Review, ReviewId, ReviewSummary};
use vet_app::trust::PullRequestTrust;
use vet_core::content::{BlobId, Hash, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::filter::ReviewFilter;
use vet_core::merkle::{self, TreeNode};
use vet_core::review::{Anchor, PendingVote, ThreadId};

use crate::budget::{Budget, REVIEW_IO};
use crate::clock::unix_now;
use codec::*;
use content::ContentDb;
use entity::*;

type Res<T> = Result<T, DbErr>;

#[derive(Clone)]
pub struct SqliteStore {
    writer: ReviewWriter,
    reader: StoreReader,
    content: Arc<ContentDb>,
    dir: PathBuf,
    /// What reads and writes of content draw on.
    io: Budget,
}

fn db_err(e: impl std::fmt::Display) -> DbErr {
    DbErr::Custom(e.to_string())
}

const DATABASE: &str = "vet.sqlite";

/// How long a connection waits for a lock SQLite holds before it gives up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A pool of read-only connections to the SQLite database `path`, which is in WAL mode already.
async fn read_pool(path: &Path) -> Result<SqlitePool, sea_orm::sqlx::Error> {
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .read_only(true)
                .busy_timeout(BUSY_TIMEOUT),
        )
        .await
}

/// The only connection that writes. One connection serializes the writers.
#[derive(Clone)]
pub struct ReviewWriter {
    db: DatabaseConnection,
}

impl ReviewWriter {
    async fn connect(dir: &Path) -> Result<Self, DbErr> {
        let options = SqliteConnectOptions::new()
            .filename(dir.join(DATABASE))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT)
            // A commit in WAL mode is an append; a checkpoint copies it into the database and
            // then truncates the log, so on Apple systems it flushes the drive (as content
            // batches do) before the log goes. Elsewhere SQLite ignores it.
            .pragma("checkpoint_fullfsync", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(db_err)?;
        Ok(Self {
            db: SqlxSqliteConnector::from_sqlx_sqlite_pool(pool),
        })
    }
}

/// A pool of read-only connections. The file is in WAL mode already, which the writer's
/// connection put it in.
#[derive(Clone)]
struct StoreReader {
    db: DatabaseConnection,
}

impl StoreReader {
    async fn connect(dir: &Path) -> Result<Self, DbErr> {
        let pool = read_pool(&dir.join(DATABASE)).await.map_err(db_err)?;
        Ok(Self {
            db: SqlxSqliteConnector::from_sqlx_sqlite_pool(pool),
        })
    }
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

/// Deletes the SQLite database `file` in `dir` with its write-ahead log and shared memory.
pub(crate) fn remove_database(dir: &Path, file: &str) -> io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        match std::fs::remove_file(dir.join(format!("{file}{suffix}"))) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// The bytes the two databases take in `dir`, the data directory, with their logs.
pub fn size_on_disk(dir: &Path) -> u64 {
    [DATABASE, content::FILE]
        .iter()
        .flat_map(|file| ["", "-wal", "-shm"].map(|suffix| format!("{file}{suffix}")))
        .filter_map(|name| std::fs::metadata(dir.join(name)).ok())
        .map(|m| m.len())
        .sum()
}

impl SqliteStore {
    /// The per-platform data directory; in WSL the Linux home, never `/mnt/*`.
    pub fn default_dir() -> Option<PathBuf> {
        vet_log::data_dir()
    }

    /// Brings the database up to date by its migrations. One that a release wrote before there
    /// were migrations is deleted and created anew, losing its contents; one written by a newer
    /// Vet is left alone and fails to open. Its content is read and written on `io`.
    pub async fn open(dir: &Path, io: Budget) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir)?;
        let mut writer = ReviewWriter::connect(dir).await?;
        match migration::origin(&writer.db).await? {
            migration::Origin::Known => {}
            migration::Origin::Unmigrated => {
                tracing::warn!(dir = %dir.display(), "the database predates migrations; creating it anew");
                writer.db.close().await?;
                remove_database(dir, DATABASE)?;
                writer = ReviewWriter::connect(dir).await?;
            }
            migration::Origin::Newer => {
                return Err(StoreError(format!(
                    "the database in {} was written by a newer version of Vet",
                    dir.display()
                )));
            }
        }
        migration::Migrator::up(&writer.db, None).await?;
        let reader = StoreReader::connect(dir).await?;
        let content = ContentDb::open(dir).await?;
        Ok(Self {
            writer,
            reader,
            content: Arc::new(content),
            dir: dir.to_owned(),
            io,
        })
    }

    /// The same store, its content read and written on `io`.
    pub fn budgeted(&self, io: Budget) -> Self {
        Self { io, ..self.clone() }
    }

    /// A store with a budget of its own, for tests.
    #[doc(hidden)]
    pub async fn for_test(dir: &Path) -> Result<Self, StoreError> {
        Self::open(dir, Budget::new(REVIEW_IO)).await
    }

    /// The data directory the store lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Holds `id` as content that is never readable, for tests that need the store to fail to
    /// read a blob it holds.
    #[doc(hidden)]
    pub async fn hold_unreadable_for_test(&self, id: BlobId) -> Result<(), StoreError> {
        self.content.hold_unreadable(id).await
    }

    /// Reads in one transaction, so that what takes several statements is one state of the file.
    async fn read(&self) -> Res<DatabaseTransaction> {
        self.reader.db.begin().await
    }

    async fn load_review(db: &impl ConnectionTrait, id: ReviewId) -> Res<Option<Review>> {
        let rid = id.0 as i64;
        let Some(row) = review::Entity::find_by_id(rid).one(db).await? else {
            return Ok(None);
        };
        let spans = span::Entity::find()
            .filter(span::Column::Review.eq(rid))
            .order_by_asc(span::Column::Id)
            .all(db)
            .await?;
        let anchors = find_anchors(
            db,
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
                .all(db)
                .await?,
        };
        review_from(rows).map(Some)
    }

    async fn load_threads(db: &impl ConnectionTrait, changeset: ChangesetId) -> Res<Vec<ChangesetThread>> {
        let cid = changeset.0 as i64;
        let threads = thread::Entity::find()
            .filter(thread::Column::Changeset.eq(cid))
            .order_by_asc(thread::Column::Id)
            .all(db)
            .await?;
        let anchors = find_anchors(db, threads.iter().filter_map(|t| t.anchor).collect()).await?;
        let thread_ids = thread::Entity::find()
            .filter(thread::Column::Changeset.eq(cid))
            .select_only()
            .column(thread::Column::Id);
        let messages = message::Entity::find()
            .filter(message::Column::Thread.in_subquery(thread_ids.into_query()))
            .order_by_asc(message::Column::Id)
            .all(db)
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
    /// The batch is one transaction of the content database, committed on return.
    async fn put_blobs(&self, blobs: Vec<Vec<u8>>) -> Result<Vec<BlobId>, StoreError> {
        self.content.put_blobs(&self.io, blobs, unix_now()).await
    }

    async fn missing_blobs(&self, ids: Vec<BlobId>) -> Result<Vec<BlobId>, StoreError> {
        self.content.missing(&ids).await
    }

    async fn blob(&self, id: BlobId) -> Result<Option<StoredBlob>, StoreError> {
        Ok(self.content.blobs(&self.io, vec![id]).await?.remove(&id))
    }

    async fn blobs(&self, ids: Vec<BlobId>) -> Result<BTreeMap<BlobId, StoredBlob>, StoreError> {
        self.content.blobs(&self.io, ids).await
    }

    async fn snapshot(&self, id: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
        let db = self.read().await?;
        let key = id.0.0.to_vec();
        if snapshot::Entity::find_by_id(key.clone()).one(&db).await?.is_none() {
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
                    .all(&db)
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
                .all(&db)
                .await?,
        );
        Ok(Some((Snapshot { id, entries }, status)))
    }
}

impl ChangesetStore for SqliteStore {
    async fn changeset(&self, repo: String, source: Source) -> Result<Option<ChangesetId>, StoreError> {
        Ok(find_changeset(&self.reader.db, repo, &source)
            .await?
            .map(|id| ChangesetId(id as u64)))
    }

    async fn source_reviews(&self, repo: String, sources: &[Source]) -> Result<Vec<SourceReview>, StoreError> {
        let db = &self.reader.db;
        let changesets = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo))
            .all(db)
            .await?;
        let ids: Vec<i64> = changesets.iter().map(|c| c.id).collect();
        // One query each, whatever the number of sources: the reviews newest first, the versions
        // (for the newest number), and the id of each open thread.
        let reviews = review::Entity::find()
            .filter(review::Column::Changeset.is_in(ids.clone()))
            .order_by_asc(review::Column::Changeset)
            .order_by_desc(review::Column::Id)
            .all(db)
            .await?;
        let versions = version::Entity::find()
            .filter(version::Column::Changeset.is_in(ids.clone()))
            .all(db)
            .await?;
        let open: Vec<i64> = thread::Entity::find()
            .filter(thread::Column::Changeset.is_in(ids))
            .filter(thread::Column::Status.eq(kinds::ThreadStatus::Open))
            .select_only()
            .column(thread::Column::Changeset)
            .into_tuple()
            .all(db)
            .await?;
        let mut latest: HashMap<i64, &review::Model> = HashMap::new();
        for review in &reviews {
            latest.entry(review.changeset).or_insert(review);
        }
        let mut newest: HashMap<i64, u32> = HashMap::new();
        for version in &versions {
            let at = newest.entry(version.changeset).or_insert(0);
            *at = (*at).max(u32::try_from(version.n).unwrap_or(u32::MAX));
        }
        let mut counting: HashMap<i64, u32> = HashMap::new();
        for id in open {
            *counting.entry(id).or_insert(0) += 1;
        }
        let summary =
            |review: &review::Model| match (review.summary_total, review.summary_seen, review.summary_old_base) {
                (Some(total), Some(seen), Some(old_base)) => Some(ReviewSummary {
                    total: u32::try_from(total).unwrap_or(0),
                    seen: u32::try_from(seen).unwrap_or(0),
                    old_base: u32::try_from(old_base).unwrap_or(0),
                }),
                _ => None,
            };
        Ok(sources
            .iter()
            .map(|source| {
                let (kind, key) = source_columns(source);
                let found = changesets.iter().find(|c| c.kind == kind && c.key == key);
                SourceReview {
                    source: source.clone(),
                    changeset: found.map(|c| ChangesetId(c.id as u64)),
                    open_threads: found.map_or(0, |c| counting.get(&c.id).copied().unwrap_or(0)),
                    latest: found.and_then(|c| latest.get(&c.id).copied()).map(|review| ReviewRow {
                        opened_at: review.opened_at,
                        closed_at: review.closed_at,
                        complete: review.complete,
                        last_reviewed: review.last_reviewed.and_then(|n| u32::try_from(n).ok()),
                        summary: summary(review),
                    }),
                    latest_version: found.map_or(0, |c| newest.get(&c.id).copied().unwrap_or(0)),
                }
            })
            .collect())
    }

    async fn lens(&self, repo: String, source: Source) -> Result<Option<Lens>, StoreError> {
        let (kind, key) = source_columns(&source);
        let row = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo))
            .filter(changeset::Column::Kind.eq(kind))
            .filter(changeset::Column::Key.eq(key))
            .one(&self.reader.db)
            .await?;
        Ok(row.and_then(|r| r.lens).map(Into::into))
    }

    async fn pull_request_head(&self, repo: String, source: Source) -> Result<Option<PullRequestHead>, StoreError> {
        let (kind, key) = source_columns(&source);
        let row = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo))
            .filter(changeset::Column::Kind.eq(kind))
            .filter(changeset::Column::Key.eq(key))
            .one(&self.reader.db)
            .await?;
        Ok(row.and_then(|r| {
            Some(PullRequestHead {
                title: r.title?,
                draft: r.draft,
                fork: r.fork,
                target_branch: r.target_branch?,
            })
        }))
    }

    async fn pull_request_trust(&self, repo: String, source: Source) -> Result<Option<PullRequestTrust>, StoreError> {
        let (kind, key) = source_columns(&source);
        let row = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo))
            .filter(changeset::Column::Kind.eq(kind))
            .filter(changeset::Column::Key.eq(key))
            .one(&self.reader.db)
            .await?;
        // The platform's data on the pull request is stored together with its title.
        Ok(row.filter(|r| r.title.is_some()).map(|r| PullRequestTrust {
            fork: r.fork,
            trusted_head: r.trusted_head.map(CommitId),
        }))
    }

    async fn pinned_base(&self, repo: String, source: Source) -> Result<Option<CommitId>, StoreError> {
        let (kind, key) = source_columns(&source);
        let row = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo))
            .filter(changeset::Column::Kind.eq(kind))
            .filter(changeset::Column::Key.eq(key))
            .one(&self.reader.db)
            .await?;
        Ok(row.and_then(|r| r.pinned_base).map(CommitId))
    }

    async fn pending_vote(&self, repo: String, source: Source) -> Result<Option<PendingVote>, StoreError> {
        let (kind, key) = source_columns(&source);
        let row = changeset::Entity::find()
            .filter(changeset::Column::Repo.eq(repo))
            .filter(changeset::Column::Kind.eq(kind))
            .filter(changeset::Column::Key.eq(key))
            .one(&self.reader.db)
            .await?;
        Ok(row.map(vote_from).transpose()?.flatten())
    }

    async fn open_changesets(&self) -> Result<Vec<ChangesetId>, StoreError> {
        let ids: Vec<i64> = review::Entity::find()
            .filter(review::Column::ClosedAt.is_null())
            .select_only()
            .column(review::Column::Changeset)
            .distinct()
            .order_by_asc(review::Column::Changeset)
            .into_tuple()
            .all(&self.reader.db)
            .await?;
        Ok(ids.into_iter().map(|id| ChangesetId(id as u64)).collect())
    }

    async fn stored_discussion(&self, remote: RemoteKey, number: u64) -> Result<Option<StoredDiscussion>, StoreError> {
        let row = pull_request::Entity::find()
            .filter(pull_request::Column::Host.eq(remote.host))
            .filter(pull_request::Column::Path.eq(remote.path))
            .filter(pull_request::Column::Number.eq(number as i64))
            .one(&self.reader.db)
            .await?;
        Ok(row.map(detail_from).transpose()?)
    }

    async fn versions(&self, changeset: ChangesetId) -> Result<Vec<Version>, StoreError> {
        let db = self.read().await?;
        let rows = version::Entity::find()
            .filter(version::Column::Changeset.eq(changeset.0 as i64))
            .order_by_asc(version::Column::N)
            .all(&db)
            .await?;
        let mut files: HashMap<i64, Vec<version_file::Model>> = HashMap::new();
        for file in version_file::Entity::find()
            .filter(version_file::Column::Version.is_in(rows.iter().map(|r| r.id)))
            .order_by_asc(version_file::Column::Id)
            .all(&db)
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
}

impl ReviewStore for SqliteStore {
    async fn open_review(&self, changeset: ChangesetId) -> Result<Option<ReviewId>, StoreError> {
        let id: Option<i64> = review::Entity::find()
            .filter(review::Column::Changeset.eq(changeset.0 as i64))
            .filter(review::Column::ClosedAt.is_null())
            .order_by_desc(review::Column::Id)
            .select_only()
            .column(review::Column::Id)
            .into_tuple()
            .one(&self.reader.db)
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
            .one(&self.reader.db)
            .await?;
        Ok(id.map(|i| ReviewId(i as u64)))
    }

    async fn review(&self, id: ReviewId) -> Result<Option<Review>, StoreError> {
        Ok(Self::load_review(&self.read().await?, id).await?)
    }

    async fn filter(&self, repo: String) -> Result<ReviewFilter, StoreError> {
        let db = self.read().await?;
        let Some(row) = project_filter::Entity::find_by_id(repo.clone()).one(&db).await? else {
            return Ok(ReviewFilter::default());
        };
        let items = project_filter_item::Entity::find()
            .filter(project_filter_item::Column::Repo.eq(repo))
            .order_by_asc(project_filter_item::Column::Id)
            .all(&db)
            .await?;
        Ok(filter_from(row.hide_binary, items)?)
    }

    async fn project_trusted(&self, repo: String) -> Result<bool, StoreError> {
        Ok(project_trust::Entity::find_by_id(repo)
            .one(&self.reader.db)
            .await?
            .is_some())
    }
}

impl ThreadStore for SqliteStore {
    async fn threads(&self, changeset: ChangesetId) -> Result<Vec<ChangesetThread>, StoreError> {
        Ok(Self::load_threads(&self.read().await?, changeset).await?)
    }
}

impl LiveStore for SqliteStore {
    type Writes = ReviewWriter;

    fn writes(&self) -> ReviewWriter {
        self.writer.clone()
    }
}
