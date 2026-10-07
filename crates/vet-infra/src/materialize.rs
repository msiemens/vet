//! `vet_semantic::ports::Materializer`: a snapshot written out as a temporary
//! directory, which is all the semantic layer ever sees of it (I1, §7.2).

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vet_app::acquisition::Acquisition;
use vet_app::ports::{Checkout, CheckoutError, History, Store};
use vet_core::content::SnapshotId;
use vet_semantic::ports::{Materialized, Materializer, NotMaterializable};

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

/// Serves the snapshots an acquisition has recorded.
pub struct Materialize<G, S>(Arc<Acquisition<G, S>>);

impl<G, S> Materialize<G, S> {
    pub fn new(acquisition: Arc<Acquisition<G, S>>) -> Self {
        Self(acquisition)
    }
}

impl<G: History + Checkout + Send + Sync, S: Store + Send + Sync> Materializer for Materialize<G, S> {
    type Guard = DirGuard;

    /// Builds the snapshot in a fresh temporary directory. Refuses unless every
    /// blob is available (§6.1).
    async fn materialize(&self, id: SnapshotId) -> Result<Materialized<DirGuard>, NotMaterializable> {
        let other = |why: String| NotMaterializable::Other(why);
        let (commit, snapshot, status) = self.0.recorded(id).await.map_err(other)?;
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
        match self.0.git.checkout(&self.0.store, &commit, &snapshot, &root).await {
            Ok(()) => Ok(Materialized { root, guard }),
            Err(CheckoutError::MissingBlobs(blobs)) => Err(NotMaterializable::MissingBlobs(blobs)),
            Err(CheckoutError::Git(e)) => Err(other(e.0)),
        }
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
