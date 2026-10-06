//! `Watcher` over `notify` (FSEvents, `ReadDirectoryChangesW`, inotify) with a
//! polling fallback. Polling is just a `MustRescan` tick: reconciliation is
//! metadata-first, so it is the poll.
//!
//! What `notify` 8 surfaces (checked against its source):
//! - inotify `IN_Q_OVERFLOW`: `Flag::Rescan`. Watch-limit exhaustion:
//!   `ErrorKind::MaxFilesWatch`, from `watch()` and as an error event.
//! - FSEvents `MustScanSubDirs` (with `UserDropped`/`KernelDropped`): `Flag::Rescan`.
//!   `RootChanged` is not a rescan; it arrives as a rename-from event with info "root changed".
//! - `ReadDirectoryChangesW`: neither the zero-byte overflow nor handle invalidation
//!   is reported (the handler ignores the byte count and unwatches silently).
//!   Windows therefore also gets a periodic safety rescan.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::event::EventKind;
use notify::{Config, ErrorKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::{Interval, MissedTickBehavior, interval_at};
use vet_app::ports::{Degraded, WatchEvent, Watcher};
use vet_core::content::RepoPath;

type NotifyResult = notify::Result<notify::Event>;

const WINDOWS_SAFETY_RESCAN: Duration = Duration::from_secs(30);

pub struct FsWatcher {
    root: PathBuf,
    native: Option<(RecommendedWatcher, UnboundedReceiver<NotifyResult>)>,
    tick: Option<Interval>,
    poll: Duration,
    degraded: Option<Degraded>,
}

impl FsWatcher {
    /// Watches `root` recursively. Polls every `poll` instead when the topology
    /// has no usable events or the inotify watch limit is exhausted.
    pub fn open(root: &Path, poll: Duration) -> notify::Result<Self> {
        let osrelease = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok();
        let canonical = root.canonicalize().map_err(notify::Error::io)?;
        if let Some(reason) = topology_degraded(&canonical, osrelease.as_deref()) {
            return Ok(Self::polling(root, poll, reason));
        }
        let (tx, rx) = unbounded_channel();
        let started = RecommendedWatcher::new(move |res| drop(tx.send(res)), Config::default())
            .and_then(|mut watcher| watcher.watch(root, RecursiveMode::Recursive).map(|()| watcher));
        match started {
            Ok(watcher) => Ok(Self {
                root: root.to_owned(),
                native: Some((watcher, rx)),
                tick: cfg!(windows).then(|| ticker(WINDOWS_SAFETY_RESCAN)),
                poll,
                degraded: None,
            }),
            Err(e) if matches!(e.kind, ErrorKind::MaxFilesWatch) => Ok(Self::polling(root, poll, Degraded::WatchLimit)),
            Err(e) => Err(e),
        }
    }

    fn polling(root: &Path, poll: Duration, reason: Degraded) -> Self {
        Self {
            root: root.to_owned(),
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

async fn recv(native: &mut Option<(RecommendedWatcher, UnboundedReceiver<NotifyResult>)>) -> NotifyResult {
    match native {
        Some((_, rx)) => match rx.recv().await {
            Some(res) => res,
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
        loop {
            tokio::select! {
                res = recv(&mut self.native) => match translate(&self.root, res) {
                    Some(WatchEvent::Degraded(reason)) => {
                        self.native = None;
                        self.tick = Some(ticker(self.poll));
                        self.degraded = Some(reason);
                        return WatchEvent::Degraded(reason);
                    }
                    Some(event) => return event,
                    None => {}
                },
                () = tick(&mut self.tick) => return WatchEvent::MustRescan,
            }
        }
    }
}

/// Maps one `notify` result to a `WatchEvent`; `None` for events that cannot
/// change content. Anything unexpected becomes `MustRescan`, never silence.
fn translate(root: &Path, res: NotifyResult) -> Option<WatchEvent> {
    let event = match res {
        Ok(event) => event,
        Err(e) if matches!(e.kind, ErrorKind::MaxFilesWatch) => {
            return Some(WatchEvent::Degraded(Degraded::WatchLimit));
        }
        Err(_) => return Some(WatchEvent::MustRescan),
    };
    if event.need_rescan() || event.info() == Some("root changed") {
        return Some(WatchEvent::MustRescan);
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return None;
    }
    let mut dirty = Vec::new();
    for path in &event.paths {
        let Ok(relative) = path.strip_prefix(root) else {
            return Some(WatchEvent::MustRescan);
        };
        let parts: Vec<&[u8]> = relative
            .components()
            .map(|part| part.as_os_str().as_encoded_bytes())
            .collect();
        if parts.first() == Some(&b".git".as_slice()) {
            // Branch switches and commits change snapshot identity without touching the worktree.
            if matches!(parts.get(1).copied(), Some(b"HEAD" | b"index")) {
                return Some(WatchEvent::MustRescan);
            }
            continue;
        }
        dirty.push(RepoPath(parts.join(&b'/')));
    }
    (!dirty.is_empty()).then_some(WatchEvent::Dirty(dirty))
}

#[cfg(test)]
mod tests {
    use notify::event::{AccessKind, Flag, ModifyKind, RenameMode};

    use super::*;

    const ROOT: &str = "/work/repo";

    fn event(kind: EventKind, paths: &[&str]) -> notify::Event {
        let mut event = notify::Event::new(kind);
        event.paths = paths.iter().map(|path| Path::new(ROOT).join(path)).collect();
        event
    }

    fn translated(event: notify::Event) -> Option<WatchEvent> {
        translate(Path::new(ROOT), Ok(event))
    }

    #[test]
    fn rescan_flags_require_reconciliation() {
        let rescan = notify::Event::new(EventKind::Other).set_flag(Flag::Rescan);
        // inotify IN_Q_OVERFLOW
        assert_eq!(translated(rescan.clone()), Some(WatchEvent::MustRescan));
        // FSEvents MustScanSubDirs, UserDropped, KernelDropped
        for info in ["rescan: user dropped", "rescan: kernel dropped"] {
            assert_eq!(translated(rescan.clone().set_info(info)), Some(WatchEvent::MustRescan));
        }
    }

    #[test]
    fn fsevents_root_change_requires_reconciliation() {
        let root_changed =
            notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From))).set_info("root changed");
        assert_eq!(translated(root_changed), Some(WatchEvent::MustRescan));
    }

    #[test]
    fn errors_require_reconciliation_and_watch_limit_degrades() {
        let limit = Err(notify::Error::new(ErrorKind::MaxFilesWatch));
        assert_eq!(
            translate(Path::new(ROOT), limit),
            Some(WatchEvent::Degraded(Degraded::WatchLimit))
        );
        let other = Err(notify::Error::generic("handle invalidated"));
        assert_eq!(translate(Path::new(ROOT), other), Some(WatchEvent::MustRescan));
    }

    #[test]
    fn changes_become_repo_relative_paths() {
        let modified = event(EventKind::Modify(ModifyKind::Any), &["src/a.rs"]);
        assert_eq!(
            translated(modified),
            Some(WatchEvent::Dirty(vec![RepoPath(b"src/a.rs".to_vec())]))
        );
        assert_eq!(
            translated(event(EventKind::Access(AccessKind::Any), &["src/a.rs"])),
            None
        );
    }

    #[test]
    fn git_state_changes_rescan_and_the_rest_of_git_is_ignored() {
        assert_eq!(
            translated(event(EventKind::Any, &[".git/HEAD"])),
            Some(WatchEvent::MustRescan)
        );
        assert_eq!(
            translated(event(EventKind::Any, &[".git/index"])),
            Some(WatchEvent::MustRescan)
        );
        assert_eq!(translated(event(EventKind::Any, &[".git/objects/ab/cd"])), None);
    }

    #[test]
    fn paths_outside_the_root_require_reconciliation() {
        let outside = notify::Event::new(EventKind::Any).add_path("/elsewhere/a".into());
        assert_eq!(translated(outside), Some(WatchEvent::MustRescan));
    }

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

    #[tokio::test]
    async fn polling_ticks_a_rescan() {
        let mut watcher = FsWatcher::polling(
            Path::new(ROOT),
            Duration::from_millis(10),
            Degraded::MountedWindowsDrive,
        );
        assert_eq!(watcher.degraded(), Some(Degraded::MountedWindowsDrive));
        assert_eq!(watcher.next().await, WatchEvent::MustRescan);
        assert_eq!(watcher.next().await, WatchEvent::MustRescan);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn inotify_reports_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut watcher = FsWatcher::open(dir.path(), Duration::from_millis(50)).unwrap();
        if watcher.degraded().is_some() {
            return;
        }
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
