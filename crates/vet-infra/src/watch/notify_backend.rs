//! `notify` (FSEvents, inotify) for macOS and Linux. Checked against notify 8's source:
//! - inotify `IN_Q_OVERFLOW`: `Flag::Rescan`. Watch-limit exhaustion:
//!   `ErrorKind::MaxFilesWatch`, from `watch()` and as an error event.
//! - FSEvents `MustScanSubDirs` (with `UserDropped`/`KernelDropped`): `Flag::Rescan`.
//!   `RootChanged` is not a rescan; it arrives as a rename-from event with info "root changed".

use std::path::Path;

use notify::event::EventKind;
use notify::{Config, ErrorKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc::UnboundedSender;
use vet_app::ports::{Degraded, WatchEvent};

use super::{Guard, StartError, dirty_events};

type NotifyResult = notify::Result<notify::Event>;

pub fn start(root: &Path, tx: UnboundedSender<WatchEvent>) -> Result<Guard, StartError> {
    let event_root = root.to_owned();
    let on_event = move |res| {
        for event in translate(&event_root, res) {
            drop(tx.send(event));
        }
    };
    let mut watcher = RecommendedWatcher::new(on_event, Config::default()).map_err(failed)?;
    watcher.watch(root, RecursiveMode::Recursive).map_err(failed)?;
    Ok(Box::new(watcher))
}

fn failed(e: notify::Error) -> StartError {
    match e.kind {
        ErrorKind::MaxFilesWatch => StartError::WatchLimit,
        _ => StartError::Failed(std::io::Error::other(e)),
    }
}

/// Maps one `notify` result to `WatchEvent`s; none for events that cannot
/// change content. Anything unexpected becomes `MustRescan`, never silence.
fn translate(root: &Path, res: NotifyResult) -> Vec<WatchEvent> {
    let event = match res {
        Ok(event) => event,
        Err(e) if matches!(e.kind, ErrorKind::MaxFilesWatch) => {
            return vec![WatchEvent::Degraded(Degraded::WatchLimit)];
        }
        Err(_) => return vec![WatchEvent::MustRescan],
    };
    if event.need_rescan() || event.info() == Some("root changed") {
        return vec![WatchEvent::MustRescan];
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return Vec::new();
    }
    dirty_events(root, &event.paths)
}

#[cfg(test)]
mod tests {
    use notify::event::{AccessKind, Flag, ModifyKind, RenameMode};
    use vet_core::content::RepoPath;

    use super::*;

    const ROOT: &str = "/work/repo";

    fn event(kind: EventKind, paths: &[&str]) -> notify::Event {
        let mut event = notify::Event::new(kind);
        event.paths = paths.iter().map(|path| Path::new(ROOT).join(path)).collect();
        event
    }

    fn translated(event: notify::Event) -> Vec<WatchEvent> {
        translate(Path::new(ROOT), Ok(event))
    }

    #[test]
    fn rescan_flags_require_reconciliation() {
        let rescan = notify::Event::new(EventKind::Other).set_flag(Flag::Rescan);
        // inotify IN_Q_OVERFLOW
        assert_eq!(translated(rescan.clone()), vec![WatchEvent::MustRescan]);
        // FSEvents MustScanSubDirs, UserDropped, KernelDropped
        for info in ["rescan: user dropped", "rescan: kernel dropped"] {
            assert_eq!(translated(rescan.clone().set_info(info)), vec![WatchEvent::MustRescan]);
        }
    }

    #[test]
    fn fsevents_root_change_requires_reconciliation() {
        let root_changed =
            notify::Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From))).set_info("root changed");
        assert_eq!(translated(root_changed), vec![WatchEvent::MustRescan]);
    }

    #[test]
    fn errors_require_reconciliation_and_watch_limit_degrades() {
        let limit = Err(notify::Error::new(ErrorKind::MaxFilesWatch));
        assert_eq!(
            translate(Path::new(ROOT), limit),
            vec![WatchEvent::Degraded(Degraded::WatchLimit)]
        );
        let other = Err(notify::Error::generic("handle invalidated"));
        assert_eq!(translate(Path::new(ROOT), other), vec![WatchEvent::MustRescan]);
    }

    #[test]
    fn changes_become_repo_relative_paths() {
        let modified = event(EventKind::Modify(ModifyKind::Any), &["src/a.rs"]);
        assert_eq!(
            translated(modified),
            vec![WatchEvent::Dirty(vec![RepoPath(b"src/a.rs".to_vec())])]
        );
        assert_eq!(
            translated(event(EventKind::Access(AccessKind::Any), &["src/a.rs"])),
            vec![]
        );
    }

    #[test]
    fn git_state_changes_rescan_and_the_rest_of_git_is_ignored() {
        assert_eq!(
            translated(event(EventKind::Any, &[".git/HEAD"])),
            vec![WatchEvent::MustRescan]
        );
        assert_eq!(
            translated(event(EventKind::Any, &[".git/index"])),
            vec![WatchEvent::MustRescan]
        );
        assert_eq!(translated(event(EventKind::Any, &[".git/objects/ab/cd"])), vec![]);
    }

    #[test]
    fn ref_writes_are_reported_even_alongside_a_rescan() {
        for path in [
            ".git/refs/heads/main",
            ".git/refs/remotes/origin/main",
            ".git/packed-refs",
        ] {
            assert_eq!(translated(event(EventKind::Any, &[path])), vec![WatchEvent::RefsMoved]);
        }
        assert_eq!(
            translated(event(EventKind::Any, &[".git/index", ".git/refs/heads/main"])),
            vec![WatchEvent::MustRescan, WatchEvent::RefsMoved]
        );
    }

    #[test]
    fn paths_outside_the_root_require_reconciliation() {
        let outside = notify::Event::new(EventKind::Any).add_path("/elsewhere/a".into());
        assert_eq!(translated(outside), vec![WatchEvent::MustRescan]);
    }
}
