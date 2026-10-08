//! Materialized trees kept between launches (§7.2, §8).
//!
//! ```text
//! <base>/
//!   .staging/<launch>.lock        held for the life of a launch
//!   .staging/<launch>-<n>/        a tree being built: tree/ and meta/
//!   .locks/<key>                  shared by every user of the tree, exclusive for the GC
//!   .locks/<key>.restore          exclusive while a language restores the tree
//!   .trash/<n>-<key>/             removed from service, being deleted
//!   <key>/                        <snapshot prefix>-<policy>
//!     tree/                       the root servers are started on
//!     meta/complete               written last, before the rename
//!     meta/restore-<language>.json  see `restore::record`
//!     meta/size, meta/last-used
//! ```
//!
//! A tree is built in a staging directory and renamed to its key in one step, so a
//! directory under a key is always complete. A crash leaves staging, which the next
//! start removes once the launch that owned it is gone. The locks are outside the
//! tree so that renaming and deleting never meet an open file, which Windows refuses.
//! The OS drops a lock with its process, so there is nothing to clean up after a crash.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vet_core::content::SnapshotId;
use vet_semantic::policy::AnalysisPolicy;

use crate::budget::{Budget, SEMANTIC_IO};

/// How often a restore waits for the lock of its tree to be free.
const LOCK_POLL: Duration = Duration::from_millis(100);

const STAGING: &str = ".staging";
const LOCKS: &str = ".locks";
const TRASH: &str = ".trash";
const TREE: &str = "tree";
const META: &str = "meta";
const COMPLETE: &str = "complete";
const LAST_USED: &str = "last-used";
const SIZE: &str = "size";
/// Bumped when a kept tree's layout changes: older trees are then built again.
const FORMAT: u32 = 1;
/// Hex digits of the snapshot in a directory name: long names break Windows paths
/// (260 characters) inside `node_modules`. The whole id is in `meta/complete`.
const NAME_HEX: usize = 20;
/// A launch's lock file younger than this is not judged dead: it may not be locked yet.
const NEW_LAUNCH: Duration = Duration::from_secs(10);

pub const DEFAULT_BUDGET: u64 = 20 << 30;
pub const DEFAULT_MAX_IDLE: Duration = Duration::from_secs(14 * 24 * 3600);

/// How much is kept, and for how long.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Bytes the kept trees may use together.
    pub budget: u64,
    /// A tree unused for this long is removed.
    pub max_idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            budget: DEFAULT_BUDGET,
            max_idle: DEFAULT_MAX_IDLE,
        }
    }
}

/// What keeps a tree from being collected besides being in use: the base of an
/// open changeset, or a repository's remembered base. Implemented by whoever
/// knows the changesets; this module never looks at them.
pub trait Referenced: Send + Sync {
    fn snapshots(&self) -> Pin<Box<dyn Future<Output = HashSet<SnapshotId>> + Send + '_>>;
}

impl<F: Fn() -> HashSet<SnapshotId> + Send + Sync> Referenced for F {
    fn snapshots(&self) -> Pin<Box<dyn Future<Output = HashSet<SnapshotId>> + Send + '_>> {
        Box::pin(std::future::ready(self()))
    }
}

/// What a collection did.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Collected {
    pub removed: usize,
    pub freed: u64,
    /// Trees that would have gone but are in use.
    pub in_use: usize,
}

/// A tree in use. Dropping it releases the tree to the collector; nothing is deleted.
pub struct Lease {
    _lock: File,
    tree: PathBuf,
}

impl Lease {
    pub fn tree(&self) -> &Path {
        &self.tree
    }
}

pub struct Acquired {
    pub lease: Lease,
    /// Whether it was built now, not found kept.
    pub built: bool,
}

#[derive(Debug)]
pub enum Error<E> {
    /// The caller's build failed.
    Build(E),
    Io(String),
}

struct Launch {
    id: String,
    _lock: File,
}

pub struct KeptTrees {
    base: PathBuf,
    limits: Limits,
    referenced: Arc<dyn Referenced>,
    launch: OnceLock<Result<Launch, String>>,
    staged: AtomicU64,
    /// Builds of one key in this process, so two languages starting on one tree build it once.
    gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    io: Budget,
}

struct Key {
    snapshot: SnapshotId,
    name: String,
}

impl Key {
    fn new(snapshot: SnapshotId, policy: AnalysisPolicy) -> Self {
        let hex = snapshot.0.to_hex();
        let policy = if policy.allow_executing { "exec" } else { "noexec" };
        Self {
            snapshot,
            name: format!("{}-{policy}", &hex[..NAME_HEX]),
        }
    }

    /// The content of `meta/complete`: the whole snapshot id guards against a name collision.
    fn mark(&self) -> String {
        format!("vet-tree {FORMAT} {}", self.snapshot.0.to_hex())
    }
}

fn io_message(path: &Path, e: &io::Error) -> String {
    format!("{}: {e}", path.display())
}

async fn blocking<T: Send + 'static>(io: &Budget, work: impl FnOnce() -> T + Send + 'static) -> T {
    io.blocking(work).await.expect("a blocking task")
}

impl KeptTrees {
    /// Trees below `base`, which is created on first use.
    pub fn new(base: PathBuf, limits: Limits, referenced: Arc<dyn Referenced>) -> Self {
        Self {
            base,
            limits,
            referenced,
            launch: OnceLock::new(),
            staged: AtomicU64::new(0),
            gates: Mutex::default(),
            io: Budget::new(SEMANTIC_IO),
        }
    }

    /// Reads and writes the trees on `io`.
    pub fn budgeted(mut self, io: Budget) -> Self {
        self.io = io;
        self
    }

    /// The directories, and this launch's lock, which a staging directory is judged dead by.
    fn launch(&self) -> Result<&Launch, String> {
        let launch = self.launch.get_or_init(|| {
            static LAUNCHES: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let id = format!(
                "{}-{nanos}-{}",
                std::process::id(),
                LAUNCHES.fetch_add(1, Ordering::Relaxed)
            );
            let make = || -> io::Result<File> {
                for dir in [STAGING, LOCKS, TRASH] {
                    std::fs::create_dir_all(self.base.join(dir))?;
                }
                let mut lock = open_lock(&self.base.join(STAGING).join(format!("{id}.lock")))?;
                lock.lock()?;
                // A lock file with content has been locked: when nobody holds it any more, its launch is dead.
                lock.write_all(b"1")?;
                Ok(lock)
            };
            make()
                .map(|_lock| Launch { id: id.clone(), _lock })
                .map_err(|e| io_message(&self.base, &e))
        });
        launch.as_ref().map_err(Clone::clone)
    }

    fn gate(&self, name: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self.gates.lock().unwrap_or_else(|p| p.into_inner());
        gates.entry(name.to_owned()).or_default().clone()
    }

    /// The tree of `snapshot` under `policy`, locked against collection: the kept
    /// one if there is one, else built by `build` into the directory it is given.
    pub async fn acquire<E, B, Fut>(
        &self,
        snapshot: SnapshotId,
        policy: AnalysisPolicy,
        mut build: B,
    ) -> Result<Acquired, Error<E>>
    where
        E: Send,
        B: FnMut(PathBuf) -> Fut + Send,
        Fut: Future<Output = Result<(), E>> + Send,
    {
        let launch = self.launch().map_err(Error::Io)?.id.clone();
        let key = Arc::new(Key::new(snapshot, policy));
        let gate = self.gate(&key.name);
        let _building = gate.lock().await;
        let mut built = false;
        // A collection can take a tree between its being built and being locked.
        for _ in 0..3 {
            let (base, k) = (self.base.clone(), key.clone());
            if let Some(lease) = blocking(&self.io, move || lease(&base, &k)).await.map_err(Error::Io)? {
                return Ok(Acquired { lease, built });
            }
            let staging = self
                .base
                .join(STAGING)
                .join(format!("{launch}-{}", self.staged.fetch_add(1, Ordering::Relaxed)));
            let mut staged = Staged(Some(staging.clone()));
            let dir = staging.clone();
            blocking(&self.io, move || create_staging(&dir))
                .await
                .map_err(|e| Error::Io(io_message(&staging, &e)))?;
            build(staging.join(TREE)).await.map_err(Error::Build)?;
            let (base, k, dir) = (self.base.clone(), key.clone(), staging.clone());
            blocking(&self.io, move || publish(&base, &k, &dir))
                .await
                .map_err(Error::Io)?;
            // Published, or removed by `publish`.
            staged.0 = None;
            built = true;
        }
        Err(Error::Io(
            "the tree was removed again and again while it was taken into use".into(),
        ))
    }

    /// The directory of the kept tree of `snapshot` under `policy`, if there is one.
    /// Nothing is built and nothing is locked: a collection may take the tree at any time.
    pub async fn kept(&self, snapshot: SnapshotId, policy: AnalysisPolicy) -> Option<PathBuf> {
        let (base, key) = (self.base.clone(), Key::new(snapshot, policy));
        blocking(&self.io, move || {
            let dir = base.join(&key.name);
            is_complete(&dir, &key).then(|| dir.join(TREE))
        })
        .await
    }

    /// Removes what a dead launch left and trees that are not complete. Returns how many were removed.
    pub async fn sweep(&self) -> usize {
        let Ok(launch) = self.launch().map(|l| l.id.clone()) else {
            return 0;
        };
        let base = self.base.clone();
        blocking(&self.io, move || sweep_in(&base, &launch)).await
    }

    /// Removes trees that are not in use and not referenced when they have been
    /// unused longer than the limit, and the least recently used while the total is over budget.
    pub async fn collect(&self, now: SystemTime) -> Collected {
        let referenced = self.referenced.snapshots().await;
        let (base, limits) = (self.base.clone(), self.limits);
        blocking(&self.io, move || {
            collect_in(
                &base,
                &Plan::Retention {
                    limits,
                    now,
                    referenced,
                },
            )
        })
        .await
    }

    /// Removes every tree that is not in use, referenced or not.
    pub async fn clear(&self) -> Collected {
        let base = self.base.clone();
        blocking(&self.io, move || collect_in(&base, &Plan::Everything)).await
    }
}

/// A staging directory that has not been published: removed when the build fails or is cancelled.
/// A crash leaves it to the next start's sweep.
struct Staged(Option<PathBuf>);

impl Drop for Staged {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn open_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn create_staging(staging: &Path) -> io::Result<()> {
    std::fs::create_dir_all(staging.join(TREE))?;
    std::fs::create_dir_all(staging.join(META))
}

fn touch(path: &Path) {
    if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(false).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

fn is_complete(dir: &Path, key: &Key) -> bool {
    std::fs::read_to_string(dir.join(META).join(COMPLETE)).is_ok_and(|mark| mark == key.mark())
}

/// Moves `dir` out of service and deletes it. Gives up on a tree in use.
fn retire(base: &Path, name: &str) -> io::Result<Retired> {
    let lock = open_lock(&base.join(LOCKS).join(name))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Retired::InUse),
        Err(TryLockError::Error(e)) => return Err(e),
    }
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let trash = base.join(TRASH).join(format!("{nanos}-{name}"));
    std::fs::rename(base.join(name), &trash)?;
    drop(lock);
    // Whatever is left is removed by the next sweep.
    let _ = std::fs::remove_dir_all(trash);
    Ok(Retired::Removed)
}

enum Retired {
    Removed,
    InUse,
}

/// Takes the kept tree into use, or returns `None` if there is none (anymore).
fn lease(base: &Path, key: &Key) -> Result<Option<Lease>, String> {
    let dir = base.join(&key.name);
    let fail = |e: io::Error| io_message(&dir, &e);
    if !is_complete(&dir, key) {
        if dir.exists() {
            retire(base, &key.name).map_err(fail)?;
        }
        return Ok(None);
    }
    let lock = open_lock(&base.join(LOCKS).join(&key.name)).map_err(fail)?;
    lock.lock_shared().map_err(fail)?;
    // The collector may have had the lock first.
    if !is_complete(&dir, key) {
        return Ok(None);
    }
    touch(&dir.join(META).join(LAST_USED));
    Ok(Some(Lease {
        _lock: lock,
        tree: dir.join(TREE),
    }))
}

/// Completes the staged tree and renames it to its key.
fn publish(base: &Path, key: &Key, staging: &Path) -> Result<(), String> {
    let meta = staging.join(META);
    let dir = base.join(&key.name);
    std::fs::write(meta.join(SIZE), tree_size(&staging.join(TREE)).to_string())
        .and_then(|()| std::fs::write(meta.join(LAST_USED), []))
        .and_then(|()| std::fs::write(meta.join(COMPLETE), key.mark()))
        .map_err(|e| io_message(staging, &e))?;
    // Windows refuses to rename a directory that something has just had open (an indexer, a virus scanner).
    let attempts = if cfg!(windows) { 10 } else { 1 };
    let mut failure = None;
    for attempt in 0..attempts {
        match std::fs::rename(staging, &dir) {
            Ok(()) => return Ok(()),
            // Another launch published the same tree first.
            Err(_) if is_complete(&dir, key) => {
                let _ = std::fs::remove_dir_all(staging);
                return Ok(());
            }
            Err(e) => failure = Some(e),
        }
        if attempt + 1 < attempts {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = std::fs::remove_dir_all(staging);
    Err(failure.map_or_else(String::new, |e| io_message(&dir, &e)))
}

/// Bytes of the files below `dir`; links are not followed.
pub(crate) fn tree_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(_) => total += entry.metadata().map_or(0, |m| m.len()),
                Err(_) => {}
            }
        }
    }
    total
}

enum Plan {
    Retention {
        limits: Limits,
        now: SystemTime,
        referenced: HashSet<SnapshotId>,
    },
    Everything,
}

struct Kept {
    name: String,
    snapshot: SnapshotId,
    size: u64,
    last_used: SystemTime,
}

/// The complete trees below `base`.
fn kept_trees(base: &Path) -> Vec<Kept> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let meta = entry.path().join(META);
            let mark = std::fs::read_to_string(meta.join(COMPLETE)).ok()?;
            let hex = mark.strip_prefix(&format!("vet-tree {FORMAT} "))?;
            let snapshot = SnapshotId(vet_core::content::Hash::from_hex(hex)?);
            let modified = |file: &str| std::fs::metadata(meta.join(file)).and_then(|m| m.modified()).ok();
            let size = std::fs::read_to_string(meta.join(SIZE))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or_else(|| tree_size(&entry.path().join(TREE)));
            Some(Kept {
                name,
                snapshot,
                size,
                last_used: modified(LAST_USED).or_else(|| modified(COMPLETE)).unwrap_or(UNIX_EPOCH),
            })
        })
        .collect()
}

fn collect_in(base: &Path, plan: &Plan) -> Collected {
    let mut trees = kept_trees(base);
    trees.sort_by_key(|t| t.last_used);
    let mut remaining: u64 = trees.iter().map(|t| t.size).sum();
    let mut done = Collected::default();
    for tree in &trees {
        let evict = match plan {
            Plan::Everything => true,
            Plan::Retention {
                limits,
                now,
                referenced,
            } => {
                let idle = now.duration_since(tree.last_used).unwrap_or_default();
                !referenced.contains(&tree.snapshot) && (idle > limits.max_idle || remaining > limits.budget)
            }
        };
        if !evict {
            continue;
        }
        match retire(base, &tree.name) {
            Ok(Retired::Removed) => {
                remaining = remaining.saturating_sub(tree.size);
                done.removed += 1;
                done.freed += tree.size;
            }
            Ok(Retired::InUse) => done.in_use += 1,
            Err(_) => {}
        }
    }
    done
}

/// What a launch that is gone left behind.
fn sweep_in(base: &Path, own: &str) -> usize {
    let mut removed = 0;
    let list = |dir: &str| -> Vec<PathBuf> {
        std::fs::read_dir(base.join(dir))
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    for trash in list(TRASH) {
        removed += usize::from(std::fs::remove_dir_all(trash).is_ok());
    }

    let staging = list(STAGING);
    let mut live = HashSet::from([own.to_owned()]);
    let mut dead = Vec::new();
    for path in &staging {
        let Some(id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".lock"))
        else {
            continue;
        };
        if id == own {
            continue;
        }
        // Created but not yet locked, or crashed doing so.
        let young = std::fs::metadata(path).is_ok_and(|m| {
            m.len() == 0
                && m.modified()
                    .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < NEW_LAUNCH))
        });
        match open_lock(path).map(|lock| lock.try_lock()) {
            Ok(Ok(())) if !young => dead.push((id.to_owned(), path.clone())),
            _ => {
                live.insert(id.to_owned());
            }
        }
    }
    for path in &staging {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let launch = name.rsplit_once('-').map_or("", |(launch, _)| launch);
        if path.is_dir() && !live.contains(launch) {
            removed += usize::from(std::fs::remove_dir_all(path).is_ok());
        }
    }
    for (_, lock) in dead {
        let _ = std::fs::remove_file(lock);
    }

    // Directories that no complete tree is under, and locks of trees that are gone.
    for path in list("") {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !name.starts_with('.') && path.is_dir() && !path.join(META).join(COMPLETE).is_file() {
            removed += usize::from(matches!(retire(base, name), Ok(Retired::Removed)));
        }
    }
    for lock in list(LOCKS) {
        let name = lock.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let tree = name.strip_suffix(".restore").unwrap_or(name);
        if !base.join(tree).is_dir() && open_lock(&lock).is_ok_and(|f| f.try_lock().is_ok()) {
            let _ = std::fs::remove_file(&lock);
        }
    }
    removed
}

/// Where a tree under `root` keeps its records, if `root` is the `tree/` of a kept tree.
#[derive(Clone)]
pub(crate) struct Layout {
    dir: PathBuf,
}

impl Layout {
    pub(crate) fn of_tree(root: &Path) -> Option<Self> {
        let dir = root.parent()?;
        (root.file_name()? == TREE && dir.join(META).join(COMPLETE).is_file()).then(|| Self { dir: dir.to_owned() })
    }

    pub(crate) fn meta(&self) -> PathBuf {
        self.dir.join(META)
    }

    /// Held while a language restores the tree: the restores of one tree run one at a time.
    /// Waiting for the lock takes no thread: a restore can take minutes, and a thread of
    /// `io` that waited would be missing for every other read in that time.
    pub(crate) async fn lock_restore(&self, io: &Budget) -> io::Result<File> {
        let name = self.dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let path = self
            .dir
            .parent()
            .unwrap_or(&self.dir)
            .join(LOCKS)
            .join(format!("{name}.restore"));
        loop {
            let at = path.clone();
            let taken = blocking(io, move || {
                if let Some(locks) = at.parent() {
                    std::fs::create_dir_all(locks)?;
                }
                let lock = open_lock(&at)?;
                match lock.try_lock() {
                    Ok(()) => Ok(Some(lock)),
                    Err(TryLockError::WouldBlock) => Ok(None),
                    Err(TryLockError::Error(e)) => Err(e),
                }
            })
            .await?;
            match taken {
                Some(lock) => return Ok(lock),
                None => tokio::time::sleep(LOCK_POLL).await,
            }
        }
    }

    /// Re-measures the tree after a restore added to it.
    pub(crate) async fn measure(&self, io: &Budget) {
        let (tree, size) = (self.dir.join(TREE), self.meta().join(SIZE));
        blocking(io, move || {
            let _ = std::fs::write(size, tree_size(&tree).to_string());
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use vet_core::content::Hash;

    use super::*;

    const POLICY: AnalysisPolicy = AnalysisPolicy { allow_executing: false };
    const DAY: Duration = Duration::from_secs(24 * 3600);

    fn snapshot(n: u8) -> SnapshotId {
        SnapshotId(Hash([n; 32]))
    }

    fn trees_with(base: &Path, limits: Limits, referenced: &[u8]) -> KeptTrees {
        let referenced: HashSet<_> = referenced.iter().map(|n| snapshot(*n)).collect();
        KeptTrees::new(base.to_owned(), limits, Arc::new(move || referenced.clone()))
    }

    fn trees(base: &Path) -> KeptTrees {
        trees_with(base, Limits::default(), &[])
    }

    /// Writes `bytes` bytes into the tree being built.
    fn fill(bytes: usize) -> impl FnMut(PathBuf) -> std::future::Ready<Result<(), String>> {
        move |dest| std::future::ready(std::fs::write(dest.join("file"), vec![b'x'; bytes]).map_err(|e| e.to_string()))
    }

    async fn acquire(trees: &KeptTrees, n: u8, bytes: usize) -> Acquired {
        trees.acquire(snapshot(n), POLICY, fill(bytes)).await.unwrap()
    }

    /// Makes the tree of `n` look as if it was last used `ago` before `now`.
    fn used_at(base: &Path, n: u8, now: SystemTime, ago: Duration) {
        let key = Key::new(snapshot(n), POLICY);
        let file = OpenOptions::new()
            .write(true)
            .open(base.join(&key.name).join(META).join(LAST_USED))
            .unwrap();
        file.set_modified(now - ago).unwrap();
    }

    fn exists(base: &Path, n: u8) -> bool {
        base.join(Key::new(snapshot(n), POLICY).name).join(TREE).is_dir()
    }

    fn staging_dirs(base: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(base.join(STAGING))
            .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn a_kept_tree_is_reused_by_the_next_launch_without_building() {
        let data = tempfile::tempdir().unwrap();
        let first = trees(data.path());
        let built = acquire(&first, 1, 5).await;
        assert!(built.built);
        std::fs::write(built.lease.tree().join("restored"), "node_modules").unwrap();
        let root = built.lease.tree().to_owned();
        drop(built);
        drop(first);

        let restarted = trees(data.path());
        let builds = AtomicUsize::new(0);
        let again = restarted
            .acquire(snapshot(1), POLICY, |_| {
                builds.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok::<(), String>(()))
            })
            .await
            .unwrap();
        assert!(!again.built);
        assert_eq!(builds.load(Ordering::SeqCst), 0);
        assert_eq!(again.lease.tree(), root);
        assert_eq!(std::fs::read_to_string(root.join("restored")).unwrap(), "node_modules");
    }

    #[tokio::test]
    async fn a_tree_is_found_without_being_built_or_locked_only_while_it_is_kept() {
        let data = tempfile::tempdir().unwrap();
        let trees = trees(data.path());
        assert_eq!(trees.kept(snapshot(1), POLICY).await, None);

        let built = acquire(&trees, 1, 1).await;
        let root = built.lease.tree().to_owned();
        drop(built);
        assert_eq!(trees.kept(snapshot(1), POLICY).await, Some(root));
        assert_eq!(
            trees.kept(snapshot(1), AnalysisPolicy { allow_executing: true }).await,
            None
        );
        assert_eq!(trees.kept(snapshot(2), POLICY).await, None);
        assert_eq!(trees.clear().await.removed, 1, "looking took no lock");
        assert_eq!(trees.kept(snapshot(1), POLICY).await, None);
    }

    #[tokio::test]
    async fn the_policy_is_part_of_the_key() {
        let data = tempfile::tempdir().unwrap();
        let trees = trees(data.path());
        let plain = acquire(&trees, 1, 1).await;
        let executing = trees
            .acquire(snapshot(1), AnalysisPolicy { allow_executing: true }, fill(1))
            .await
            .unwrap();
        assert!(executing.built);
        assert_ne!(plain.lease.tree(), executing.lease.tree());
    }

    /// Run by `a_crash_during_the_build_leaves_no_usable_tree`, which has this process die mid-build.
    #[test]
    #[ignore = "started by a_crash_during_the_build_leaves_no_usable_tree"]
    fn crash_child() {
        let Some(base) = std::env::var_os("VET_KEPT_CRASH_BASE") else {
            return;
        };
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(async {
            let trees = trees(Path::new(&base));
            let _ = trees
                .acquire(snapshot(1), POLICY, |dest| async move {
                    std::fs::write(dest.join("half"), "written").unwrap();
                    // Nothing is dropped or cleaned up, as in a crash.
                    if dest.exists() {
                        std::process::exit(3);
                    }
                    Ok::<(), String>(())
                })
                .await;
        });
    }

    #[tokio::test]
    async fn a_crash_during_the_build_leaves_no_usable_tree() {
        let data = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "materialize::kept::tests::crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("VET_KEPT_CRASH_BASE", data.path())
            .status()
            .unwrap();
        assert!(!status.success(), "the child was to die");
        assert!(!exists(data.path(), 1));
        assert_eq!(staging_dirs(data.path()).len(), 1, "the half-built tree is in staging");

        let restarted = trees(data.path());
        assert!(restarted.sweep().await >= 1);
        assert!(staging_dirs(data.path()).is_empty());
        let built = acquire(&restarted, 1, 3).await;
        assert!(built.built);
        assert!(!built.lease.tree().join("half").exists());
    }

    #[tokio::test]
    async fn a_failed_or_cancelled_build_leaves_nothing_behind() {
        let data = tempfile::tempdir().unwrap();
        let trees = Arc::new(trees(data.path()));
        let failed = trees
            .acquire(snapshot(1), POLICY, |dest| {
                std::fs::write(dest.join("half"), "x").unwrap();
                std::future::ready(Err::<(), _>("no blobs"))
            })
            .await;
        assert!(matches!(failed, Err(Error::Build("no blobs"))));

        let started = Arc::new(tokio::sync::Notify::new());
        let task = tokio::spawn({
            let (trees, started) = (trees.clone(), started.clone());
            async move {
                let _ = trees
                    .acquire(snapshot(2), POLICY, |dest| {
                        std::fs::write(dest.join("half"), "x").unwrap();
                        started.notify_one();
                        std::future::pending::<Result<(), String>>()
                    })
                    .await;
            }
        });
        started.notified().await;
        task.abort();
        let _ = task.await;
        assert!(staging_dirs(data.path()).is_empty());
        assert!(!exists(data.path(), 1) && !exists(data.path(), 2));
    }

    #[tokio::test]
    async fn a_sweep_leaves_the_build_of_a_running_launch_alone() {
        let data = tempfile::tempdir().unwrap();
        let building = Arc::new(trees(data.path()));
        let (started, release) = (
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(tokio::sync::Notify::new()),
        );
        let task = tokio::spawn({
            let (building, started, release) = (building.clone(), started.clone(), release.clone());
            async move {
                building
                    .acquire(snapshot(1), POLICY, |dest| {
                        let (started, release) = (started.clone(), release.clone());
                        async move {
                            std::fs::write(dest.join("file"), "x").unwrap();
                            started.notify_one();
                            release.notified().await;
                            Ok::<(), String>(())
                        }
                    })
                    .await
                    .unwrap()
                    .built
            }
        });
        started.notified().await;

        // Another Vet starting up.
        assert_eq!(trees(data.path()).sweep().await, 0);
        release.notify_one();
        assert!(task.await.unwrap());
        assert!(exists(data.path(), 1));
    }

    #[tokio::test]
    async fn two_languages_starting_together_build_the_tree_once() {
        let data = tempfile::tempdir().unwrap();
        let trees = trees(data.path());
        let builds = AtomicUsize::new(0);
        let start = || {
            trees.acquire(snapshot(1), POLICY, |dest| {
                builds.fetch_add(1, Ordering::SeqCst);
                async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    std::fs::write(dest.join("file"), "x").map_err(|e| e.to_string())
                }
            })
        };
        let (a, b) = tokio::join!(start(), start());
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(a.lease.tree(), b.lease.tree());
        assert!(a.built != b.built, "one built it, the other found it");
    }

    #[tokio::test]
    async fn two_launches_building_one_tree_end_with_one_tree() {
        let data = tempfile::tempdir().unwrap();
        let (one, other) = (trees(data.path()), trees(data.path()));
        let slow = |dest: PathBuf| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            std::fs::write(dest.join("file"), "x").map_err(|e| e.to_string())
        };
        let (a, b) = tokio::join!(
            one.acquire(snapshot(1), POLICY, slow),
            other.acquire(snapshot(1), POLICY, slow)
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.lease.tree(), b.lease.tree());
        assert!(a.lease.tree().join("file").is_file());
        assert!(staging_dirs(data.path()).is_empty());
    }

    #[tokio::test]
    async fn a_tree_in_use_is_neither_collected_nor_cleared() {
        let data = tempfile::tempdir().unwrap();
        let limits = Limits {
            budget: 0,
            max_idle: Duration::ZERO,
        };
        let trees = trees_with(data.path(), limits, &[]);
        let held = acquire(&trees, 1, 10).await;
        let now = SystemTime::now() + 365 * DAY;

        let collected = trees.collect(now).await;
        assert_eq!((collected.removed, collected.in_use), (0, 1));
        assert_eq!(trees.clear().await.removed, 0);
        assert!(held.lease.tree().join("file").is_file());

        drop(held);
        assert_eq!(trees.collect(now).await.removed, 1);
        assert!(!exists(data.path(), 1));
    }

    #[tokio::test]
    async fn a_tree_that_a_collection_took_is_built_again_when_asked_for() {
        let data = tempfile::tempdir().unwrap();
        let trees = trees(data.path());
        drop(acquire(&trees, 1, 1).await);
        assert_eq!(trees.clear().await.removed, 1);
        assert!(acquire(&trees, 1, 1).await.built);
    }

    #[tokio::test]
    async fn trees_unused_for_too_long_are_removed_unless_something_refers_to_them() {
        let data = tempfile::tempdir().unwrap();
        let trees = trees_with(data.path(), Limits::default(), &[2]);
        let now = SystemTime::now();
        for n in 1..=3 {
            drop(acquire(&trees, n, 10).await);
        }
        used_at(data.path(), 1, now, 20 * DAY);
        used_at(data.path(), 2, now, 20 * DAY);
        used_at(data.path(), 3, now, 13 * DAY);

        let collected = trees.collect(now).await;
        assert_eq!(collected.removed, 1);
        assert!(!exists(data.path(), 1), "unused and not referred to");
        assert!(exists(data.path(), 2), "old but referred to");
        assert!(exists(data.path(), 3), "not yet old");
    }

    #[tokio::test]
    async fn over_the_budget_the_least_recently_used_unreferenced_trees_go_first() {
        let data = tempfile::tempdir().unwrap();
        let limits = Limits {
            budget: 25,
            max_idle: 100 * DAY,
        };
        let trees = trees_with(data.path(), limits, &[1]);
        let now = SystemTime::now();
        for n in 1..=3 {
            drop(acquire(&trees, n, 10).await);
        }
        // 1 is the oldest but referred to.
        used_at(data.path(), 1, now, 3 * DAY);
        used_at(data.path(), 2, now, 2 * DAY);
        used_at(data.path(), 3, now, DAY);

        let collected = trees.collect(now).await;
        assert_eq!((collected.removed, collected.freed), (1, 10));
        assert!(exists(data.path(), 1));
        assert!(!exists(data.path(), 2));
        assert!(exists(data.path(), 3));

        // Within the budget nothing more goes.
        assert_eq!(trees.collect(now).await.removed, 0);
    }

    #[tokio::test]
    async fn using_a_tree_makes_it_the_most_recently_used() {
        let data = tempfile::tempdir().unwrap();
        let limits = Limits {
            budget: 15,
            max_idle: 100 * DAY,
        };
        let trees = trees_with(data.path(), limits, &[]);
        let now = SystemTime::now();
        for n in 1..=2 {
            drop(acquire(&trees, n, 10).await);
        }
        used_at(data.path(), 1, now, 5 * DAY);
        used_at(data.path(), 2, now, 4 * DAY);
        // Reopened: 1 is used now, so 2 is the one to go.
        drop(acquire(&trees, 1, 10).await);

        trees.collect(now + Duration::from_secs(60)).await;
        assert!(exists(data.path(), 1));
        assert!(!exists(data.path(), 2));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn removing_a_tree_leaves_what_its_links_lead_to() {
        let data = tempfile::tempdir().unwrap();
        let donor = tempfile::tempdir().unwrap();
        std::fs::create_dir(donor.path().join("node_modules")).unwrap();
        std::fs::write(donor.path().join("node_modules/pkg"), "x").unwrap();
        let trees = trees(data.path());
        let linked = acquire(&trees, 1, 1).await;
        std::os::unix::fs::symlink(
            donor.path().join("node_modules"),
            linked.lease.tree().join("node_modules"),
        )
        .unwrap();
        drop(linked);

        assert_eq!(trees.clear().await.removed, 1);
        assert!(donor.path().join("node_modules/pkg").is_file());
    }
}
