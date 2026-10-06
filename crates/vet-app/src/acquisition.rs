//! Acquisition (§6): maps git and filesystem state into immutable snapshots.
//! Provenance lives here, keyed by `SnapshotId`, and nowhere else (I1).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vet_core::content::{EntryKind, Snapshot, SnapshotId, SnapshotStatus};
use vet_semantic::ports::{Materialized, Materializer, NotMaterializable};

use crate::pin::CommitId;
use crate::ports::{CheckoutError, Dirty, GitError, GitRepo, Scan, Store};

/// Where a snapshot came from. Never visible to `vet-semantic`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Origin {
    DirtyTree { base_head: CommitId },
    Commit(CommitId),
    PullRequest { head: CommitId, from_fork: bool },
}

/// An evolving tip: its settled snapshots, in order, are the edit timeline.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LogicalTip {
    pub timeline: Vec<SnapshotId>,
}

/// Deletes the materialized directory when the pool drops its server (§7.2).
pub struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const MATERIALIZED: &str = "vet-materialized";
const LOCK: &str = ".lock";

fn materialized_base() -> PathBuf {
    std::env::temp_dir().join(MATERIALIZED)
}

/// This process's directory for materialized snapshots. It holds an exclusive
/// lock on `.lock` for the life of the process; the OS releases it on exit or
/// crash, which is how `sweep_materialized` tells a dead launch from a live one.
fn launch_dir() -> Result<&'static Path, String> {
    struct Launch {
        dir: PathBuf,
        _lock: File,
    }
    static LAUNCH: OnceLock<Result<Launch, String>> = OnceLock::new();
    let launch = LAUNCH.get_or_init(|| {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = materialized_base().join(format!("{}-{nanos}", std::process::id()));
        let make = || -> std::io::Result<File> {
            std::fs::create_dir_all(&dir)?;
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(dir.join(LOCK))?;
            lock.lock()?;
            Ok(lock)
        };
        make()
            .map(|_lock| Launch {
                dir: dir.clone(),
                _lock,
            })
            .map_err(|e| format!("{}: {e}", dir.display()))
    });
    match launch {
        Ok(l) => Ok(&l.dir),
        Err(e) => Err(e.clone()),
    }
}

/// Removes materialized directories left by launches that are no longer
/// running (crashed or killed). Call once at startup. Returns how many were removed.
pub fn sweep_materialized() -> usize {
    sweep_in(&materialized_base(), launch_dir().ok())
}

fn sweep_in(base: &Path, own: Option<&Path>) -> usize {
    let Ok(entries) = std::fs::read_dir(base) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let dir = entry.path();
        if own == Some(dir.as_path()) || !dir.is_dir() {
            continue;
        }
        let stale = match OpenOptions::new().read(true).write(true).open(dir.join(LOCK)) {
            // Unlocked: nobody owns it. The handle is closed before deleting.
            Ok(lock) => lock.try_lock().is_ok(),
            // Still being created by a launch that has not locked it yet.
            Err(_) => entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > Duration::from_secs(60))),
        };
        if stale && std::fs::remove_dir_all(&dir).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub struct Acquisition<G, S> {
    pub git: G,
    pub store: S,
    /// The commit whose objects back each snapshot's committed blobs.
    commits: Mutex<HashMap<SnapshotId, CommitId>>,
}

/// The initial changeset of a dirty-tree review: `HEAD` to dirty-at-open.
pub struct Opened {
    pub head: CommitId,
    pub base: Snapshot,
    pub target: Snapshot,
    pub status: SnapshotStatus,
}

impl<G, S> Acquisition<G, S> {
    pub fn new(git: G, store: S) -> Self {
        Self {
            git,
            store,
            commits: Mutex::default(),
        }
    }

    /// Remember that git can reconstruct `snapshot`'s committed blobs from
    /// `commit`; everything else must be in the content store. Needed before
    /// `materialize` can serve the snapshot (§7.2).
    pub fn record_commit(&self, snapshot: SnapshotId, commit: CommitId) {
        self.commits.lock().expect("not poisoned").insert(snapshot, commit);
    }
}

impl<G: GitRepo, S: Store> Acquisition<G, S> {
    /// Review open (§6.1): `HEAD`, dirty-at-open, and a copy of every blob git
    /// cannot be assumed to keep (anything not already in `HEAD`) in the store.
    /// Recording the pin is up to the caller.
    pub async fn open_pin(&self) -> Result<Opened, GitError> {
        let head = self.git.head().await?;
        let base = self.git.commit_snapshot(&head).await?;
        let Scan {
            snapshot: target,
            status,
        } = self.git.scan(Dirty::All).await?;
        self.retain(&[&base], &target, &status).await?;
        self.record_commit(base.id, head.clone());
        self.record_commit(target.id, head.clone());
        Ok(Opened {
            head,
            base,
            target,
            status,
        })
    }

    /// Copies the blobs of `target` that `known` snapshots do not already have
    /// (they are in git or were retained before) into the store, so the
    /// snapshot stays reconstructible (§6.1).
    pub async fn retain(
        &self,
        known: &[&Snapshot],
        target: &Snapshot,
        status: &SnapshotStatus,
    ) -> Result<(), GitError> {
        for (path, entry) in &target.entries {
            let is_known = known
                .iter()
                .any(|s| s.entries.get(path).is_some_and(|k| k.blob == entry.blob));
            if is_known || entry.kind == EntryKind::Submodule || status.content_unavailable.contains(path) {
                continue;
            }
            let bytes = self.git.read_blob(path, entry.blob).await?.ok_or_else(|| {
                GitError(format!(
                    "{} changed while being copied",
                    String::from_utf8_lossy(&path.0)
                ))
            })?;
            self.store.put_blob(bytes).await.map_err(|e| GitError(e.0))?;
        }
        Ok(())
    }

    /// Authoritative reconciliation (§6.2): before declaring a snapshot settled,
    /// at review open, on rescan signals, and before every launch.
    pub async fn reconcile(&self, dirty: Dirty) -> Result<Scan, GitError> {
        self.git.scan(dirty).await
    }
}

impl<G: GitRepo + Sync, S: Store + Sync> Materializer for Acquisition<G, S> {
    type Guard = DirGuard;

    /// Builds the snapshot in a fresh temporary directory. Refuses unless every
    /// blob is available (§6.1); the semantic layer only ever sees the directory (I1).
    async fn materialize(&self, id: SnapshotId) -> Result<Materialized<DirGuard>, NotMaterializable> {
        let other = |why: String| NotMaterializable::Other(why);
        let commit = self
            .commits
            .lock()
            .expect("not poisoned")
            .get(&id)
            .cloned()
            .ok_or_else(|| other("the snapshot has no recorded origin".into()))?;
        let stored = self.store.snapshot(id).await.map_err(|e| other(e.0))?;
        let (snapshot, status) = match stored {
            Some(found) => found,
            None => {
                // A commit's own tree is not necessarily in the store.
                let snapshot = self.git.commit_snapshot(&commit).await.map_err(|e| other(e.0))?;
                if snapshot.id != id {
                    return Err(other("the snapshot is not in the store".into()));
                }
                (snapshot, SnapshotStatus::default())
            }
        };
        if !status.path_collisions.is_empty() && cfg!(any(windows, target_os = "macos")) {
            return Err(other(
                "paths that differ only in case would merge on this filesystem".into(),
            ));
        }

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let launch = launch_dir().map_err(|e| other(format!("materialize directory: {e}")))?;
        let root = launch.join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
        std::fs::create_dir_all(&root).map_err(|e| other(format!("{}: {e}", root.display())))?;
        let guard = DirGuard(root.clone());
        match self.git.checkout(&self.store, &commit, &snapshot, &root).await {
            Ok(()) => Ok(Materialized { root, guard }),
            Err(CheckoutError::MissingBlobs(blobs)) => Err(NotMaterializable::MissingBlobs(blobs)),
            Err(CheckoutError::Git(e)) => Err(other(e.0)),
        }
    }
}

/// An acquisition that a pool can own while the session keeps using it.
pub struct SharedAcquisition<G, S>(pub Arc<Acquisition<G, S>>);

impl<G: GitRepo + Send + Sync, S: Store + Send + Sync> Materializer for SharedAcquisition<G, S> {
    type Guard = DirGuard;

    async fn materialize(&self, id: SnapshotId) -> Result<Materialized<DirGuard>, NotMaterializable> {
        self.0.materialize(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_removes_only_directories_of_dead_launches() {
        let base = std::env::temp_dir().join(format!("vet-sweep-test-{}", std::process::id()));
        let launch = |name: &str, locked: bool| {
            let dir = base.join(name);
            std::fs::create_dir_all(dir.join("0")).unwrap();
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(dir.join(LOCK))
                .unwrap();
            if locked {
                lock.lock().unwrap();
            }
            (dir, lock)
        };
        let (dead, dead_lock) = launch("dead", false);
        drop(dead_lock);
        let (alive, _alive_lock) = launch("alive", true);
        let (own, _own_lock) = launch("own", false);
        // Being created: no lock file yet, too new to judge.
        let fresh = base.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();

        assert_eq!(sweep_in(&base, Some(&own)), 1);
        assert!(!dead.exists());
        assert!(alive.exists() && own.exists() && fresh.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn this_launch_directory_is_locked_and_survives_a_sweep() {
        let dir = launch_dir().unwrap();
        // Even without being excluded by name, the held lock protects it.
        sweep_in(dir.parent().unwrap(), None);
        assert!(dir.exists());
    }
}
