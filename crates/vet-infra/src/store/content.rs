//! The content of snapshots: one row per blob in `content.sqlite`, a SQLite file of its own so
//! that a large batch never holds the writer review commands need.
//!
//! A batch is one transaction. The file is in WAL mode with `synchronous=FULL`, so a commit is
//! durable before it is visible, and with `fullfsync` that is a flush of the drive's cache on
//! Apple systems. Rows in `vet.sqlite` refer to content only after the batch that wrote it
//! returned, which is what keeps a record from pointing at content that was not written.
//!
//! Blobs are compressed with zstd, unless that does not make them smaller. A binary file Vet
//! cannot show (anything but text and images) is held by hash and size only: its row has no data.
//!
//! A row read back is checked against its hash. One that decodes to other bytes (a torn or rotten
//! row) is deleted and reported missing, so the reader falls back to git or the worktree and the
//! next put writes it again. One that does not decode at all (a codec this Vet does not know, a
//! bug) is kept and read as missing: git cannot rebuild uncommitted content, and the row may read
//! again once the cause is fixed.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;
#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sea_orm::sqlx::sqlite::{
    SqliteAutoVacuum, SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sea_orm::sqlx::{self, Row};
use vet_app::ports::{StoreError, StoredBlob};
use vet_core::content::{BlobId, Hash, keeps_bytes};

use super::{BUSY_TIMEOUT, read_pool};
use crate::budget::Budget;

pub(super) const FILE: &str = "content.sqlite";

/// Bumped when the table changes. A file with a higher version was written by a newer Vet.
const SCHEMA: i64 = 1;

/// Large enough that almost every compressed blob is one page rather than an overflow chain.
const PAGE_SIZE: u32 = 16 * 1024;

/// How far the write-ahead log may grow before a checkpoint trims it on disk.
const JOURNAL_SIZE_LIMIT: u64 = 64 << 20;

const ZSTD_LEVEL: i32 = 3;

/// `codec` of a row whose `data` is the content itself.
const AS_IS: i64 = 0;
/// `codec` of a row whose `data` is the content compressed with zstd.
const ZSTD: i64 = 1;

/// SQLite allows 32766 bound variables per statement.
const IDS_PER_STATEMENT: usize = 1000;
const ROWS_PER_INSERT: usize = 500;

/// A blob as a row.
pub(super) struct Encoded {
    id: BlobId,
    size: usize,
    codec: i64,
    /// `None`: held by hash and size only.
    data: Option<Vec<u8>>,
}

impl Encoded {
    fn of(id: BlobId, bytes: Vec<u8>) -> Self {
        let size = bytes.len();
        if !keeps_bytes(&bytes) {
            return Self {
                id,
                size,
                codec: AS_IS,
                data: None,
            };
        }
        let (codec, data) = match zstd::bulk::compress(&bytes, ZSTD_LEVEL) {
            Ok(packed) if packed.len() < size => (ZSTD, packed),
            _ => (AS_IS, bytes),
        };
        Self {
            id,
            size,
            codec,
            data: Some(data),
        }
    }
}

/// A row as stored.
struct Stored {
    id: BlobId,
    size: i64,
    codec: i64,
    data: Option<Vec<u8>>,
}

enum Decoded {
    Held(StoredBlob),
    /// Decodes, but to other bytes than its hash says.
    Torn,
    /// Does not decode.
    Unreadable,
}

impl Stored {
    fn decode(self) -> Decoded {
        let Some(data) = self.data else {
            return match u64::try_from(self.size) {
                Ok(size) => Decoded::Held(StoredBlob::Binary { size }),
                Err(_) => Decoded::Unreadable,
            };
        };
        let bytes = match (self.codec, usize::try_from(self.size)) {
            (AS_IS, _) => data,
            (ZSTD, Ok(size)) => match zstd::bulk::decompress(&data, size) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!(blob = %self.id.0.to_hex(), error = %e, "a stored blob does not decompress");
                    return Decoded::Unreadable;
                }
            },
            (codec, _) => {
                tracing::warn!(blob = %self.id.0.to_hex(), codec, "a stored blob has a codec this version does not know");
                return Decoded::Unreadable;
            }
        };
        if BlobId::of(&bytes) == self.id {
            Decoded::Held(StoredBlob::Bytes(bytes))
        } else {
            Decoded::Torn
        }
    }
}

/// Runs `f` on each share of `items`, dealt out over the units of `io`.
async fn spread<I: Send + 'static, T: Send + 'static>(
    io: &Budget,
    items: Vec<I>,
    f: fn(Vec<I>) -> Vec<T>,
) -> Result<Vec<T>, StoreError> {
    let shares = io.limit().min(items.len());
    let mut dealt: Vec<Vec<I>> = (0..shares).map(|_| Vec::new()).collect();
    for (n, item) in items.into_iter().enumerate() {
        dealt[n % shares].push(item);
    }
    let done = futures::future::try_join_all(dealt.into_iter().map(|share| io.blocking(move || f(share))))
        .await
        .map_err(|e| StoreError(e.to_string()))?;
    Ok(done.into_iter().flatten().collect())
}

fn placeholders(rows: usize, columns: usize) -> String {
    let row = format!("({})", vec!["?"; columns].join(","));
    vec![row; rows].join(",")
}

/// `{head} id IN (…ids){tail}` with the ids bound.
fn by_ids<'q>(
    head: &str,
    ids: impl ExactSizeIterator<Item = &'q [u8]>,
    tail: &str,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
    let mut query = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{head} id IN {}{tail}",
        placeholders(1, ids.len())
    )));
    for id in ids {
        query = query.bind(id);
    }
    query
}

pub(super) struct ContentDb {
    /// One connection: the writers queue in the order they asked.
    pub(super) writer: SqlitePool,
    pub(super) readers: SqlitePool,
    /// How many transactions were committed.
    #[cfg(test)]
    pub commits: AtomicUsize,
    /// Runs after a batch's rows were inserted and before the transaction commits.
    #[cfg(test)]
    pub before_commit: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl ContentDb {
    pub(super) async fn open(dir: &Path) -> Result<Self, StoreError> {
        let path = dir.join(FILE);
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            // Both are fixed once the first table exists.
            .page_size(PAGE_SIZE)
            .auto_vacuum(SqliteAutoVacuum::Incremental)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(BUSY_TIMEOUT)
            // On Apple systems a commit and a checkpoint flush the drive's cache; elsewhere
            // SQLite ignores both.
            .pragma("fullfsync", "ON")
            .pragma("checkpoint_fullfsync", "ON")
            .pragma("journal_size_limit", JOURNAL_SIZE_LIMIT.to_string());
        let writer = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let version: i64 = sqlx::query("PRAGMA user_version").fetch_one(&writer).await?.get(0);
        if version > SCHEMA {
            return Err(StoreError(format!(
                "the content database in {} was written by a newer version of Vet",
                dir.display()
            )));
        }
        let mut tx = writer.begin().await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS blob (
                id     BLOB NOT NULL UNIQUE,
                size   INTEGER NOT NULL,
                codec  INTEGER NOT NULL,
                data   BLOB,
                stored INTEGER NOT NULL
            )",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS blob_stored ON blob(stored)")
            .execute(&mut *tx)
            .await?;
        sqlx::query("PRAGMA user_version = 1").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Self {
            writer,
            readers: read_pool(&path).await?,
            #[cfg(test)]
            commits: AtomicUsize::new(0),
            #[cfg(test)]
            before_commit: Mutex::new(None),
        })
    }

    /// Of `ids`, those no row exists for, in the order given. Answered from the index alone, so a
    /// row that does not decode still counts as held.
    pub(super) async fn missing(&self, ids: &[BlobId]) -> Result<Vec<BlobId>, StoreError> {
        let mut held = HashSet::new();
        for chunk in ids.chunks(IDS_PER_STATEMENT) {
            let query = by_ids(
                "SELECT id FROM blob WHERE",
                chunk.iter().map(|id| id.0.0.as_slice()),
                "",
            );
            for row in query.fetch_all(&self.readers).await? {
                held.extend(blob_id(row.get(0)));
            }
        }
        Ok(ids.iter().copied().filter(|id| !held.contains(id)).collect())
    }

    /// Writes each of `blobs` the store does not hold, in one transaction that is committed when
    /// this returns, with `stored` (unix seconds) as the time they were stored. Answers the ids of
    /// all of `blobs`, in order. A batch whose blobs are all held opens no transaction.
    pub(super) async fn put_blobs(
        &self,
        io: &Budget,
        blobs: Vec<Vec<u8>>,
        stored: i64,
    ) -> Result<Vec<BlobId>, StoreError> {
        let mut hashed = spread(io, blobs.into_iter().enumerate().collect(), |share| {
            (share.into_iter())
                .map(|(at, bytes): (usize, Vec<u8>)| (at, BlobId::of(&bytes), bytes))
                .collect()
        })
        .await?;
        hashed.sort_by_key(|(at, ..)| *at);
        let ids: Vec<BlobId> = hashed.iter().map(|(_, id, _)| *id).collect();
        let missing: HashSet<BlobId> = self.missing(&ids).await?.into_iter().collect();
        if missing.is_empty() {
            return Ok(ids);
        }
        let mut seen = HashSet::new();
        let fresh: Vec<(BlobId, Vec<u8>)> = hashed
            .into_iter()
            .filter(|(_, id, _)| missing.contains(id) && seen.insert(*id))
            .map(|(_, id, bytes)| (id, bytes))
            .collect();
        let rows = spread(io, fresh, |share| {
            share.into_iter().map(|(id, bytes)| Encoded::of(id, bytes)).collect()
        })
        .await?;
        self.insert(rows, stored).await?;
        Ok(ids)
    }

    async fn insert(&self, rows: Vec<Encoded>, stored: i64) -> Result<(), StoreError> {
        let mut tx = self.writer.begin().await?;
        for chunk in rows.chunks(ROWS_PER_INSERT) {
            let sql = format!(
                "INSERT INTO blob (id, size, codec, data, stored) VALUES {} ON CONFLICT(id) DO NOTHING",
                placeholders(chunk.len(), 5)
            );
            let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
            for row in chunk {
                query = query
                    .bind(row.id.0.0.as_slice())
                    .bind(row.size as i64)
                    .bind(row.codec)
                    .bind(row.data.as_deref())
                    .bind(stored);
            }
            query.execute(&mut *tx).await?;
        }
        #[cfg(test)]
        if let Some(hook) = &*crate::sync::lock(&self.before_commit) {
            hook();
        }
        tx.commit().await?;
        #[cfg(test)]
        self.commits.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// What the store holds of each of `ids`; those with no row, a torn one or one that does not
    /// decode are left out. A torn row is deleted.
    pub(super) async fn blobs(
        &self,
        io: &Budget,
        ids: Vec<BlobId>,
    ) -> Result<BTreeMap<BlobId, StoredBlob>, StoreError> {
        let ids: Vec<BlobId> = ids.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
        let mut rows = Vec::new();
        for chunk in ids.chunks(IDS_PER_STATEMENT) {
            let query = by_ids(
                "SELECT id, size, codec, data FROM blob WHERE",
                chunk.iter().map(|id| id.0.0.as_slice()),
                "",
            );
            for row in query.fetch_all(&self.readers).await? {
                let Some(id) = blob_id(row.get(0)) else { continue };
                rows.push(Stored {
                    id,
                    size: row.get(1),
                    codec: row.get(2),
                    data: row.get(3),
                });
            }
        }
        let decoded = spread(io, rows, |share| {
            share.into_iter().map(|row| (row.id, row.decode())).collect()
        })
        .await?;
        let mut found = BTreeMap::new();
        let mut torn = Vec::new();
        for (id, decoded) in decoded {
            match decoded {
                Decoded::Held(blob) => {
                    found.insert(id, blob);
                }
                Decoded::Torn => torn.push(id),
                Decoded::Unreadable => {}
            }
        }
        self.delete(&torn).await?;
        Ok(found)
    }

    /// Deletes the rows of `ids`, which do not match their hashes.
    async fn delete(&self, ids: &[BlobId]) -> Result<(), StoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut tx = self.writer.begin().await?;
        for id in ids {
            tracing::warn!(blob = %id.0.to_hex(), "a stored blob does not match its hash; removing it");
            sqlx::query("DELETE FROM blob WHERE id = ?")
                .bind(id.0.0.as_slice())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Deletes the rows not in `live` that were stored `grace` or longer before `now` (unix
    /// seconds; a row stored after `now` counts as stored just now), and gives the space back.
    /// Returns the number of rows deleted.
    pub(super) async fn sweep(&self, live: &HashSet<BlobId>, now: i64, grace: Duration) -> Result<usize, StoreError> {
        let cutoff = if grace.is_zero() {
            i64::MAX
        } else {
            now.saturating_sub(i64::try_from(grace.as_secs()).unwrap_or(i64::MAX))
        };
        let candidates: Vec<Vec<u8>> = sqlx::query("SELECT id FROM blob WHERE stored <= ?")
            .bind(cutoff)
            .fetch_all(&self.writer)
            .await?
            .into_iter()
            .map(|row| row.get(0))
            .collect();
        let doomed: Vec<Vec<u8>> = candidates
            .into_iter()
            .filter(|id| blob_id(id.clone()).is_none_or(|id| !live.contains(&id)))
            .collect();
        let mut tx = self.writer.begin().await?;
        for chunk in doomed.chunks(IDS_PER_STATEMENT) {
            by_ids(
                "DELETE FROM blob WHERE",
                chunk.iter().map(Vec::as_slice),
                " AND stored <= ?",
            )
            .bind(cutoff)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        if !doomed.is_empty() {
            sqlx::query("PRAGMA incremental_vacuum").execute(&self.writer).await?;
        }
        Ok(doomed.len())
    }

    /// Holds `id` with a codec no version knows: it counts as held and never reads.
    pub(super) async fn hold_unreadable(&self, id: BlobId) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO blob (id, size, codec, data, stored) VALUES (?, 0, 255, x'00', 0) ON CONFLICT(id) DO NOTHING",
        )
        .bind(id.0.0.as_slice())
        .execute(&self.writer)
        .await?;
        Ok(())
    }
}

fn blob_id(bytes: Vec<u8>) -> Option<BlobId> {
    <[u8; 32]>::try_from(bytes).ok().map(|b| BlobId(Hash(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::lock;

    const NOW: i64 = 1_700_000_000;

    fn io() -> Budget {
        Budget::new(2)
    }

    async fn open(dir: &Path) -> ContentDb {
        ContentDb::open(dir).await.unwrap()
    }

    async fn put(db: &ContentDb, blobs: &[&[u8]]) -> Vec<BlobId> {
        db.put_blobs(&io(), blobs.iter().map(|b| b.to_vec()).collect(), NOW)
            .await
            .unwrap()
    }

    async fn get(db: &ContentDb, id: BlobId) -> Option<StoredBlob> {
        db.blobs(&io(), vec![id]).await.unwrap().remove(&id)
    }

    /// Text that compresses well.
    fn text(n: usize) -> Vec<u8> {
        format!("line {n} of a source file\n").repeat(3000).into_bytes()
    }

    /// Bytes that do not compress and hold no NUL, so that they are kept.
    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state % 255) as u8 + 1
            })
            .collect()
    }

    /// `(codec, size, length of the data)` of the row of `id`.
    async fn row(db: &ContentDb, id: BlobId) -> Option<(i64, i64, Option<i64>)> {
        sqlx::query("SELECT codec, size, length(data) FROM blob WHERE id = ?")
            .bind(id.0.0.as_slice())
            .fetch_optional(&db.readers)
            .await
            .unwrap()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
    }

    async fn insert_raw(db: &ContentDb, id: BlobId, size: i64, codec: i64, data: &[u8]) {
        sqlx::query("INSERT INTO blob (id, size, codec, data, stored) VALUES (?, ?, ?, ?, 0)")
            .bind(id.0.0.as_slice())
            .bind(size)
            .bind(codec)
            .bind(data)
            .execute(&db.writer)
            .await
            .unwrap();
    }

    async fn pragma(db: &ContentDb, name: &str) -> i64 {
        let sql = format!("PRAGMA {name}");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .fetch_one(&db.writer)
            .await
            .unwrap()
            .get(0)
    }

    #[tokio::test]
    async fn a_text_is_stored_compressed_and_reads_back_whole() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let text = text(1);
        assert!(text.len() > 64 * 1024);
        let id = put(&db, &[&text]).await[0];

        let (codec, size, stored) = row(&db, id).await.unwrap();
        assert_eq!((codec, size), (ZSTD, text.len() as i64));
        assert!(stored.unwrap() < size / 4, "the row holds the compressed bytes");
        assert_eq!(get(&db, id).await, Some(StoredBlob::Bytes(text)));
    }

    #[tokio::test]
    async fn bytes_that_do_not_compress_are_stored_as_they_are() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let bytes = noise(4096, 7);
        let id = put(&db, &[&bytes]).await[0];

        assert_eq!(row(&db, id).await, Some((AS_IS, 4096, Some(4096))));
        assert_eq!(get(&db, id).await, Some(StoredBlob::Bytes(bytes)));
    }

    #[tokio::test]
    async fn a_binary_that_cannot_be_shown_is_held_by_hash_and_size_only() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let archive = b"!<arch>\n\0\0\0\0".to_vec();
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        let ids = put(&db, &[&archive, &png]).await;

        assert_eq!(row(&db, ids[0]).await, Some((AS_IS, archive.len() as i64, None)));
        assert_eq!(db.missing(&ids).await.unwrap(), []);
        assert_eq!(
            get(&db, ids[0]).await,
            Some(StoredBlob::Binary {
                size: archive.len() as u64
            })
        );
        assert_eq!(get(&db, ids[1]).await, Some(StoredBlob::Bytes(png)));
    }

    #[tokio::test]
    async fn a_row_that_decodes_to_other_bytes_is_deleted_and_written_again_by_the_next_put() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let id = BlobId::of(b"the real content");
        let other = zstd::bulk::compress(b"something else entirely", ZSTD_LEVEL).unwrap();
        insert_raw(&db, id, 23, ZSTD, &other).await;
        let plain = BlobId::of(b"whole");
        insert_raw(&db, plain, 3, AS_IS, b"who").await;
        assert_eq!(db.missing(&[id, plain]).await.unwrap(), []);

        assert_eq!(get(&db, id).await, None);
        assert_eq!(get(&db, plain).await, None);
        assert_eq!(db.missing(&[id, plain]).await.unwrap(), [id, plain]);

        put(&db, &[b"the real content", b"whole"]).await;
        assert_eq!(
            get(&db, id).await,
            Some(StoredBlob::Bytes(b"the real content".to_vec()))
        );
        assert_eq!(get(&db, plain).await, Some(StoredBlob::Bytes(b"whole".to_vec())));
    }

    #[tokio::test]
    async fn a_row_that_does_not_decode_is_kept_and_reads_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let garbage = BlobId::of(b"garbage");
        insert_raw(&db, garbage, 7, ZSTD, b"not a zstd frame").await;
        let unknown = BlobId::of(b"unknown");
        insert_raw(&db, unknown, 7, 255, b"unknown").await;

        assert_eq!(get(&db, garbage).await, None);
        assert_eq!(get(&db, unknown).await, None);
        assert!(row(&db, garbage).await.is_some() && row(&db, unknown).await.is_some());
        assert_eq!(db.missing(&[garbage, unknown]).await.unwrap(), []);
        put(&db, &[b"garbage"]).await;
        assert_eq!(get(&db, garbage).await, None, "a put does not overwrite what it holds");
    }

    #[tokio::test]
    async fn the_database_is_set_up_for_durable_commits_and_lasting_space() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;

        let mode: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&db.writer)
            .await
            .unwrap()
            .get(0);
        assert_eq!(mode, "wal");
        assert_eq!(pragma(&db, "synchronous").await, 2, "FULL");
        assert_eq!(pragma(&db, "page_size").await, i64::from(PAGE_SIZE));
        assert_eq!(pragma(&db, "auto_vacuum").await, 2, "incremental");
        assert_eq!(pragma(&db, "journal_size_limit").await, JOURNAL_SIZE_LIMIT as i64);
        #[cfg(target_vendor = "apple")]
        {
            assert_eq!(pragma(&db, "fullfsync").await, 1);
            assert_eq!(pragma(&db, "checkpoint_fullfsync").await, 1);
        }
    }

    #[tokio::test]
    async fn a_batch_is_one_commit_and_a_batch_of_blobs_held_already_commits_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let commits = || db.commits.load(Ordering::Relaxed);
        let blobs: Vec<Vec<u8>> = (0..40).map(text).collect();
        let refs: Vec<&[u8]> = blobs.iter().map(Vec::as_slice).collect();

        put(&db, &refs).await;
        assert_eq!(commits(), 1);
        put(&db, &refs).await;
        assert_eq!(commits(), 1);
        put(&db, &[b"one more"]).await;
        assert_eq!(commits(), 2);
    }

    #[tokio::test]
    async fn a_blob_given_twice_is_written_once_and_answers_both() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;

        let ids = put(&db, &[b"twin", b"other", b"twin"]).await;

        assert_eq!(ids[0], ids[2]);
        assert_eq!(db.missing(&ids).await.unwrap(), []);
        let rows: i64 = sqlx::query("SELECT count(*) FROM blob")
            .fetch_one(&db.readers)
            .await
            .unwrap()
            .get(0);
        assert_eq!(rows, 2);
    }

    #[tokio::test]
    async fn a_database_of_a_newer_version_is_not_opened_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let db = open(dir.path()).await;
            sqlx::query("PRAGMA user_version = 2")
                .execute(&db.writer)
                .await
                .unwrap();
            put(&db, &[b"kept"]).await[0]
        };

        let Err(refused) = ContentDb::open(dir.path()).await else {
            panic!("a newer database was opened")
        };
        assert!(refused.0.contains("newer version"), "{}", refused.0);

        let options = SqliteConnectOptions::new().filename(dir.path().join(FILE));
        let pool = SqlitePoolOptions::new().connect_with(options).await.unwrap();
        let rows: i64 = sqlx::query("SELECT count(*) FROM blob WHERE id = ?")
            .bind(id.0.0.as_slice())
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
        assert_eq!(rows, 1);
    }

    /// The blobs the crash tests write, as the child process and its parent both see them.
    fn crash_blobs() -> Vec<Vec<u8>> {
        (0..30).map(text).collect()
    }

    fn crash_ids() -> Vec<BlobId> {
        crash_blobs().iter().map(|b| BlobId::of(b)).collect()
    }

    /// Run by the crash tests, which have this process die in the middle of a put or right after it.
    #[test]
    #[ignore = "started by the crash tests"]
    fn crash_child() {
        let Some(dir) = std::env::var_os("VET_CONTENT_CRASH_DIR") else {
            return;
        };
        let before_commit = std::env::var("VET_CONTENT_CRASH_AT").is_ok_and(|at| at == "before_commit");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let db = open(Path::new(&dir)).await;
            if before_commit {
                // Nothing is dropped or flushed, as in a crash.
                *lock(&db.before_commit) = Some(Box::new(|| std::process::exit(3)));
            }
            db.put_blobs(&io(), crash_blobs(), NOW).await.unwrap();
            std::process::exit(4);
        });
    }

    /// The exit code of a child that puts the crash blobs into the database in `dir`.
    fn crash(dir: &Path, at: &str) -> Option<i32> {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::content::tests::crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("VET_CONTENT_CRASH_DIR", dir)
            .env("VET_CONTENT_CRASH_AT", at)
            .status()
            .unwrap()
            .code()
    }

    #[tokio::test]
    async fn a_crash_before_the_commit_leaves_none_of_the_batch_and_a_healthy_database() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            crash(dir.path(), "before_commit"),
            Some(3),
            "the child was to die mid-put"
        );

        let db = open(dir.path()).await;
        assert_eq!(db.missing(&crash_ids()).await.unwrap(), crash_ids());
        let check: String = sqlx::query("PRAGMA integrity_check")
            .fetch_one(&db.writer)
            .await
            .unwrap()
            .get(0);
        assert_eq!(check, "ok");
        let ids = db.put_blobs(&io(), crash_blobs(), NOW).await.unwrap();
        assert_eq!(db.missing(&ids).await.unwrap(), []);
        assert_eq!(get(&db, ids[0]).await, Some(StoredBlob::Bytes(crash_blobs().remove(0))));
    }

    #[tokio::test]
    async fn a_crash_right_after_a_put_returned_leaves_all_of_it() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            crash(dir.path(), "after_return"),
            Some(4),
            "the child was to die after the put"
        );

        let db = open(dir.path()).await;
        assert_eq!(db.missing(&crash_ids()).await.unwrap(), []);
        for (id, bytes) in crash_ids().into_iter().zip(crash_blobs()) {
            assert_eq!(get(&db, id).await, Some(StoredBlob::Bytes(bytes)));
        }
    }

    #[tokio::test]
    async fn a_sweep_deletes_what_nothing_refers_to_once_it_was_stored_for_the_grace() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let hour = Duration::from_secs(3600);
        let put_at =
            async |content: &[u8], stored| db.put_blobs(&io(), vec![content.to_vec()], stored).await.unwrap()[0];
        let old = put_at(b"old", NOW - 7200).await;
        let live = put_at(b"live and old", NOW - 7200).await;
        let fresh = put_at(b"fresh", NOW).await;
        let live_set = HashSet::from([live]);

        assert_eq!(db.sweep(&live_set, NOW, hour).await.unwrap(), 1);
        assert_eq!(db.missing(&[old, live, fresh]).await.unwrap(), [old]);

        // A row stored after `now` counts as stored just now: a grace keeps it, no grace does not.
        assert_eq!(db.sweep(&live_set, NOW - 1000, hour).await.unwrap(), 0);
        assert_eq!(db.sweep(&live_set, NOW - 1000, Duration::ZERO).await.unwrap(), 1);
        assert_eq!(db.missing(&[old, live, fresh]).await.unwrap(), [old, fresh]);
    }

    #[tokio::test]
    async fn a_sweep_gives_the_space_back() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(dir.path()).await;
        let blobs: Vec<Vec<u8>> = (0..48).map(|n| noise(50 * 1024, n + 1)).collect();
        db.put_blobs(&io(), blobs, NOW).await.unwrap();
        let before = pragma(&db, "page_count").await;
        assert!(before > 2 * 1024 * 1024 / i64::from(PAGE_SIZE));

        assert_eq!(db.sweep(&HashSet::new(), NOW, Duration::ZERO).await.unwrap(), 48);

        assert_eq!(pragma(&db, "freelist_count").await, 0);
        assert!(pragma(&db, "page_count").await < before);
    }
}
