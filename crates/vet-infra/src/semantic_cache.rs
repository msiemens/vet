//! `vet_semantic::reuse::SemanticCache`: the answers of language servers kept between
//! launches (I2, SPEC §7.6, §8), in one SQLite file (WAL) of their own, apart from
//! the review database.
//!
//! The file is disposable. Deleting it, or a format this Vet does not know, costs
//! speed and nothing else; a row that does not decode is a miss and is deleted.
//! Writes are queued and applied in batches by one task, so a query never waits
//! for the disk. A crash loses the batch that was not applied yet. Several Vets
//! may share the file: rows are replaced as a whole, under SQLite's own locking,
//! so one launch never loses what another wrote.
//!
//! The file holds at most a budget of bytes. Beyond it the rows least recently used
//! are removed, down to four fifths of it.

mod codec;

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sea_orm::sqlx::{self, Row};
use tokio::sync::{mpsc, oneshot};
use vet_semantic::reuse::{Key, SemanticCache, Value};

use crate::clock::unix_now;
use codec::{Address, address, decode, encode};

const FILE: &str = "semantic-cache.sqlite";

/// Bumped when the table changes. A file with another version is recreated.
const SCHEMA: i64 = 1;
const SET_SCHEMA: &str = "PRAGMA user_version = 1";

/// Default size of the file's content (SPEC §8).
pub const BUDGET: u64 = 256 << 20;

/// How long writes are gathered into one transaction, and how many at most.
const BATCH_WINDOW: Duration = Duration::from_millis(200);
const BATCH_ROWS: usize = 100;

/// Rows written between two checks of the budget.
const CHECK_EVERY: usize = 1000;

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

enum Job {
    Put {
        address: Address,
        kind: i64,
        bytes: Vec<u8>,
    },
    Used(Address),
    /// Answered when everything queued before it is applied, and the budget is kept.
    Flush(oneshot::Sender<()>),
    /// Removes every row and answers with the bytes it held.
    Clear(oneshot::Sender<u64>),
}

pub struct SqliteSemanticCache {
    pool: SqlitePool,
    queue: mpsc::UnboundedSender<Job>,
}

fn message(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A file that is not a database, or one written by a version of Vet that kept something else.
enum Unusable {
    Foreign,
    Failed(String),
}

impl From<sqlx::Error> for Unusable {
    fn from(e: sqlx::Error) -> Self {
        // SQLITE_CORRUPT, SQLITE_NOTADB.
        let broken = matches!(&e, sqlx::Error::Database(d) if matches!(d.code().as_deref(), Some("11" | "26")));
        if broken {
            Self::Foreign
        } else {
            Self::Failed(message(e))
        }
    }
}

async fn connect(path: &Path) -> Result<SqlitePool, Unusable> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(10));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    let version: i64 = sqlx::query("PRAGMA user_version").fetch_one(&pool).await?.get(0);
    if version != 0 && version != SCHEMA {
        pool.close().await;
        return Err(Unusable::Foreign);
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS answer (
            key       BLOB PRIMARY KEY,
            kind      INTEGER NOT NULL,
            value     BLOB NOT NULL,
            bytes     INTEGER NOT NULL,
            last_used INTEGER NOT NULL
        ) WITHOUT ROWID",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS answer_lru ON answer(last_used)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(SET_SCHEMA).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(pool)
}

impl SqliteSemanticCache {
    /// Opens the cache in `dir`, the data directory, with the default budget.
    pub async fn open(dir: &Path) -> Result<Self, String> {
        Self::start(dir, BUDGET, Arc::new(unix_now)).await
    }

    async fn start(dir: &Path, budget: u64, clock: Clock) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(message)?;
        let path = dir.join(FILE);
        let pool = match connect(&path).await {
            Ok(pool) => pool,
            Err(Unusable::Failed(why)) => return Err(why),
            Err(Unusable::Foreign) => {
                for suffix in ["", "-wal", "-shm"] {
                    match std::fs::remove_file(dir.join(format!("{FILE}{suffix}"))) {
                        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(message(e)),
                        _ => {}
                    }
                }
                match connect(&path).await {
                    Ok(pool) => pool,
                    Err(Unusable::Foreign) => return Err("the semantic cache cannot be created".into()),
                    Err(Unusable::Failed(why)) => return Err(why),
                }
            }
        };
        let (queue, jobs) = mpsc::unbounded_channel();
        tokio::spawn(write_all(pool.clone(), jobs, budget, clock));
        let cache = Self { pool, queue };
        cache.flush().await;
        Ok(cache)
    }

    /// Returns when everything put so far is in the file and the file is within its budget.
    pub async fn flush(&self) {
        let (done, wait) = oneshot::channel();
        if self.queue.send(Job::Flush(done)).is_ok() {
            let _ = wait.await;
        }
    }

    /// "Clear semantic caches": removes every answer. Returns the bytes freed.
    pub async fn clear(&self) -> u64 {
        let (done, wait) = oneshot::channel();
        if self.queue.send(Job::Clear(done)).is_err() {
            return 0;
        }
        wait.await.unwrap_or(0)
    }

    async fn read(&self, address: Address, kind: codec::Kind) -> Option<Value> {
        let row = sqlx::query("SELECT value FROM answer WHERE key = ?")
            .bind(&address[..])
            .fetch_optional(&self.pool)
            .await
            .ok()??;
        let stored: Vec<u8> = row.get(0);
        match decode(kind, &stored) {
            Some(value) => {
                let _ = self.queue.send(Job::Used(address));
                Some(value)
            }
            None => {
                let _ = sqlx::query("DELETE FROM answer WHERE key = ?")
                    .bind(&address[..])
                    .execute(&self.pool)
                    .await;
                None
            }
        }
    }
}

impl SemanticCache for SqliteSemanticCache {
    fn get<'a>(&'a self, key: &'a Key) -> Pin<Box<dyn Future<Output = Option<Value>> + Send + 'a>> {
        Box::pin(async move {
            let (address, kind) = address(key)?;
            self.read(address, kind).await
        })
    }

    fn put(&self, key: &Key, value: &Value) {
        let Some((address, _)) = address(key) else {
            return;
        };
        let (kind, bytes) = encode(value);
        let _ = self.queue.send(Job::Put {
            address,
            kind: kind.id(),
            bytes,
        });
    }
}

/// Applies the queued jobs in batches until every sender is gone.
async fn write_all(pool: SqlitePool, mut jobs: mpsc::UnboundedReceiver<Job>, budget: u64, clock: Clock) {
    let mut unchecked = 0;
    loop {
        let Some(first) = jobs.recv().await else {
            return;
        };
        let mut batch = vec![first];
        let window = tokio::time::sleep(BATCH_WINDOW);
        tokio::pin!(window);
        while !matches!(batch.last(), Some(Job::Flush(_) | Job::Clear(_))) && batch.len() < BATCH_ROWS {
            tokio::select! {
                job = jobs.recv() => match job {
                    Some(job) => batch.push(job),
                    None => break,
                },
                () = &mut window => break,
            }
        }
        let puts = batch.iter().filter(|j| matches!(j, Job::Put { .. })).count();
        let flushed = batch.iter().any(|j| matches!(j, Job::Flush(_)));
        let acks = apply(&pool, batch, &clock).await;
        unchecked += puts;
        if unchecked >= CHECK_EVERY || flushed {
            unchecked = 0;
            if let Err(e) = keep_within(&pool, budget).await {
                eprintln!("vet: semantic cache: {e}");
            }
        }
        for ack in acks {
            match ack {
                Ack::Flushed(done) => {
                    let _ = done.send(());
                }
                Ack::Cleared(done, freed) => {
                    let _ = done.send(freed);
                }
            }
        }
    }
}

enum Ack {
    Flushed(oneshot::Sender<()>),
    Cleared(oneshot::Sender<u64>, u64),
}

/// Writes the jobs in one transaction; what could not be written is lost, as the cache may lose it.
async fn apply(pool: &SqlitePool, batch: Vec<Job>, clock: &Clock) -> Vec<Ack> {
    let mut acks = Vec::new();
    let result: Result<(), sqlx::Error> = async {
        let mut tx = pool.begin().await?;
        let now = clock();
        for job in batch {
            match job {
                Job::Put { address, kind, bytes } => {
                    sqlx::query(
                        "INSERT INTO answer (key, kind, value, bytes, last_used) VALUES (?, ?, ?, ?, ?)
                         ON CONFLICT(key) DO UPDATE SET
                           value = excluded.value, bytes = excluded.bytes, last_used = excluded.last_used",
                    )
                    .bind(&address[..])
                    .bind(kind)
                    .bind(&bytes[..])
                    .bind(bytes.len() as i64)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                }
                Job::Used(address) => {
                    sqlx::query("UPDATE answer SET last_used = ? WHERE key = ?")
                        .bind(now)
                        .bind(&address[..])
                        .execute(&mut *tx)
                        .await?;
                }
                Job::Flush(done) => acks.push(Ack::Flushed(done)),
                Job::Clear(done) => {
                    let held: i64 = sqlx::query("SELECT COALESCE(SUM(bytes), 0) FROM answer")
                        .fetch_one(&mut *tx)
                        .await?
                        .get(0);
                    sqlx::query("DELETE FROM answer").execute(&mut *tx).await?;
                    acks.push(Ack::Cleared(done, held.max(0) as u64));
                }
            }
        }
        tx.commit().await
    }
    .await;
    if let Err(e) = result {
        eprintln!("vet: semantic cache: {e}");
    }
    if acks.iter().any(|a| matches!(a, Ack::Cleared(..))) {
        // Hands the space back to the file system.
        let _ = sqlx::query("VACUUM").execute(pool).await;
    }
    acks
}

/// Removes the rows least recently used while the file holds more than `budget` bytes,
/// down to four fifths of it.
async fn keep_within(pool: &SqlitePool, budget: u64) -> Result<(), sqlx::Error> {
    let held: i64 = sqlx::query("SELECT COALESCE(SUM(bytes), 0) FROM answer")
        .fetch_one(pool)
        .await?
        .get(0);
    let held = held.max(0) as u64;
    if held <= budget {
        return Ok(());
    }
    let mut excess = held - budget / 5 * 4;
    while excess > 0 {
        let oldest = sqlx::query("SELECT key, bytes FROM answer ORDER BY last_used ASC LIMIT 200")
            .fetch_all(pool)
            .await?;
        if oldest.is_empty() {
            break;
        }
        let mut tx = pool.begin().await?;
        for row in oldest {
            if excess == 0 {
                break;
            }
            let (key, bytes): (Vec<u8>, i64) = (row.get(0), row.get(1));
            sqlx::query("DELETE FROM answer WHERE key = ?")
                .bind(&key[..])
                .execute(&mut *tx)
                .await?;
            excess = excess.saturating_sub(bytes.max(0) as u64);
        }
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
