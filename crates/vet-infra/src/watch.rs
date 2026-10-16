//! `Watcher` over the platform's native events with a polling fallback: `notify`
//! (FSEvents, inotify) on macOS and Linux, `ReadDirectoryChangesW` directly on Windows
//! (`notify` hides its overflow and handle errors). Polling is just a `MustRescan`
//! tick: reconciliation is metadata-first, so it is the poll.

use std::io;
use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Interval, MissedTickBehavior, interval_at};
use vet_app::ports::{Degraded, WatchEvent, Watcher};

use crate::paths;

#[cfg(not(windows))]
mod notify_backend;
#[cfg(windows)]
mod windows_backend;

/// Keeps a native watch alive; dropping it stops the watch.
type Guard = Box<dyn Send>;

enum StartError {
    // Only the notify backend runs into the inotify limit.
    #[cfg_attr(windows, allow(dead_code))]
    WatchLimit,
    Failed(io::Error),
}

#[cfg(not(windows))]
fn start_native(root: &Path, tx: UnboundedSender<WatchEvent>) -> Result<Guard, StartError> {
    notify_backend::start(root, tx)
}

#[cfg(windows)]
fn start_native(root: &Path, tx: UnboundedSender<WatchEvent>) -> Result<Guard, StartError> {
    let watcher = windows_backend::DirWatcher::open(root, tx).map_err(StartError::Failed)?;
    Ok(Box::new(watcher))
}

pub struct FsWatcher {
    native: Option<(Guard, UnboundedReceiver<WatchEvent>)>,
    tick: Option<Interval>,
    poll: Duration,
    degraded: Option<Degraded>,
}

impl FsWatcher {
    /// Watches `root` recursively. Polls every `poll` instead when the topology
    /// has no usable events or the inotify watch limit is exhausted.
    pub fn open(root: &Path, poll: Duration) -> io::Result<Self> {
        let osrelease = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok();
        let canonical = root.canonicalize()?;
        if let Some(reason) = topology_degraded(&canonical, osrelease.as_deref()) {
            return Ok(Self::polling(poll, reason));
        }
        let (tx, rx) = unbounded_channel();
        match start_native(root, tx) {
            Ok(guard) => Ok(Self {
                native: Some((guard, rx)),
                tick: None,
                poll,
                degraded: None,
            }),
            Err(StartError::WatchLimit) => Ok(Self::polling(poll, Degraded::WatchLimit)),
            Err(StartError::Failed(e)) => Err(e),
        }
    }

    fn polling(poll: Duration, reason: Degraded) -> Self {
        Self {
            native: None,
            tick: Some(ticker(poll)),
            poll,
            degraded: Some(reason),
        }
    }
}

fn ticker(period: Duration) -> Interval {
    let mut interval = interval_at(tokio::time::Instant::now() + period, period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval
}

/// A WSL worktree under `/mnt/*` is on 9P, where inotify does not see Windows-side writes.
fn topology_degraded(canonical_root: &Path, osrelease: Option<&str>) -> Option<Degraded> {
    let wsl = osrelease.is_some_and(|release| release.to_ascii_lowercase().contains("microsoft"));
    (wsl && canonical_root.starts_with("/mnt")).then_some(Degraded::MountedWindowsDrive)
}

async fn recv(native: &mut Option<(Guard, UnboundedReceiver<WatchEvent>)>) -> WatchEvent {
    match native {
        Some((_, rx)) => match rx.recv().await {
            Some(event) => event,
            None => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}

async fn tick(tick: &mut Option<Interval>) {
    match tick {
        Some(interval) => drop(interval.tick().await),
        None => std::future::pending().await,
    }
}

impl Watcher for FsWatcher {
    fn degraded(&self) -> Option<Degraded> {
        self.degraded
    }

    async fn next(&mut self) -> WatchEvent {
        tokio::select! {
            event = recv(&mut self.native) => {
                if let WatchEvent::Degraded(reason) = event {
                    self.native = None;
                    self.tick = Some(ticker(self.poll));
                    self.degraded = Some(reason);
                }
                event
            }
            () = tick(&mut self.tick) => WatchEvent::MustRescan,
        }
    }
}

/// Maps changed absolute paths to events: `Dirty` paths (root-relative, `/`-separated), empty
/// when nothing changed that matters. Paths outside the root or `.git/HEAD` and
/// `.git/index` (branch switches and commits change snapshot identity without
/// touching the worktree) force `MustRescan`; a change below `.git/refs` other than
/// `.git/refs/vet/`, to `.git/packed-refs` (a commit on any branch, a fetch) or to `.git/HEAD`
/// (a checkout, a commit on a detached HEAD) is `RefsMoved`, besides whatever else the paths
/// say; the rest of `.git` is ignored. `.git/refs/vet/` holds only the refs Vet writes for
/// its own bookkeeping, which never move the branch on view.
fn dirty_events<P: AsRef<Path>>(root: &Path, paths: impl IntoIterator<Item = P>) -> Vec<WatchEvent> {
    let mut dirty = Vec::new();
    let mut refs_moved = false;
    let mut rescan = false;
    for path in paths {
        let Ok(relative) = path.as_ref().strip_prefix(root) else {
            rescan = true;
            continue;
        };
        // A name that cannot be told apart from the repository's is a reason to look at everything.
        let Ok(path) = paths::from_relative(relative) else {
            rescan = true;
            continue;
        };
        let mut parts = path.0.split(|&b| b == b'/');
        if parts.next() == Some(b".git".as_slice()) {
            match parts.next() {
                Some(b"index") => rescan = true,
                // A detached HEAD moves without a ref being written.
                Some(b"HEAD") => {
                    rescan = true;
                    refs_moved = true;
                }
                Some(b"refs") => {
                    // Vet's own refs (changesets, fetched pull request heads) move without
                    // moving the branch on view.
                    if parts.next() != Some(b"vet".as_slice()) {
                        refs_moved = true;
                    }
                }
                Some(b"packed-refs") => refs_moved = true,
                _ => {}
            }
            continue;
        }
        dirty.push(path);
    }
    let mut events = Vec::new();
    if rescan {
        events.push(WatchEvent::MustRescan);
    } else if !dirty.is_empty() {
        events.push(WatchEvent::Dirty(dirty));
    }
    if refs_moved {
        events.push(WatchEvent::RefsMoved);
    }
    events
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use vet_core::content::RepoPath;

    use super::*;

    #[test]
    fn mnt_inside_wsl_is_degraded() {
        let wsl = Some("5.15.0-microsoft-standard-WSL2");
        assert_eq!(
            topology_degraded(Path::new("/mnt/c/work"), wsl),
            Some(Degraded::MountedWindowsDrive)
        );
        assert_eq!(topology_degraded(Path::new("/home/u/work"), wsl), None);
        assert_eq!(
            topology_degraded(Path::new("/mnt/data/work"), Some("6.1.0-generic")),
            None
        );
    }

    #[test]
    fn vet_ref_writes_do_not_move_the_branch_on_view() {
        let root = Path::new("/work/repo");
        for path in [
            "/work/repo/.git/refs/vet",
            "/work/repo/.git/refs/vet/pr/7/head",
            "/work/repo/.git/refs/vet/changesets/1/2/tip",
        ] {
            assert_eq!(dirty_events(root, [path]), vec![], "{path}");
        }
        assert_eq!(
            dirty_events(root, ["/work/repo/.git/refs/heads/main"]),
            vec![WatchEvent::RefsMoved]
        );
    }

    #[test]
    fn a_packed_refs_rewrite_moves_refs() {
        let root = Path::new("/work/repo");
        assert_eq!(
            dirty_events(root, ["/work/repo/.git/packed-refs"]),
            vec![WatchEvent::RefsMoved]
        );
    }

    #[tokio::test]
    async fn polling_ticks_a_rescan() {
        let mut watcher = FsWatcher::polling(Duration::from_millis(10), Degraded::MountedWindowsDrive);
        assert_eq!(watcher.degraded(), Some(Degraded::MountedWindowsDrive));
        assert_eq!(watcher.next().await, WatchEvent::MustRescan);
        assert_eq!(watcher.next().await, WatchEvent::MustRescan);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn inotify_reports_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut watcher = FsWatcher::open(dir.path(), Duration::from_millis(50)).unwrap();
        assert!(
            watcher.degraded().is_none(),
            "inotify is not usable here: {:?}",
            watcher.degraded()
        );
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let seen = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let WatchEvent::Dirty(paths) = watcher.next().await
                    && paths.contains(&RepoPath(b"a.txt".to_vec()))
                {
                    return;
                }
            }
        })
        .await;
        assert!(seen.is_ok(), "no event for a.txt");
    }
}
