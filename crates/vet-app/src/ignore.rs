//! Drops watcher events for git-ignored paths before they reach settling.

use vet_core::content::RepoPath;

use crate::ports::{Degraded, IgnoreRules, WatchEvent, Watcher};

pub struct IgnoringWatcher<W, I> {
    watcher: W,
    rules: I,
    /// Paths whose filtering was cancelled by a dropped `next` future; they are
    /// reported unfiltered next time so no event is lost.
    interrupted: Option<Vec<RepoPath>>,
}

impl<W, I> IgnoringWatcher<W, I> {
    pub fn new(watcher: W, rules: I) -> Self {
        Self {
            watcher,
            rules,
            interrupted: None,
        }
    }
}

impl<W: Watcher + Send, I: IgnoreRules + Send + Sync> Watcher for IgnoringWatcher<W, I> {
    fn degraded(&self) -> Option<Degraded> {
        self.watcher.degraded()
    }

    async fn next(&mut self) -> WatchEvent {
        if let Some(paths) = self.interrupted.take() {
            return WatchEvent::Dirty(paths);
        }
        loop {
            let paths = match self.watcher.next().await {
                WatchEvent::Dirty(paths) => paths,
                other => return other,
            };
            self.interrupted = Some(paths.clone());
            let kept = self.rules.unignored(paths).await;
            self.interrupted = None;
            match kept {
                Ok(kept) if kept.is_empty() => {}
                Ok(kept) => return WatchEvent::Dirty(kept),
                // Unknown is never silence: reconcile everything.
                Err(_) => return WatchEvent::MustRescan,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::ports::GitError;

    struct Script(VecDeque<WatchEvent>);

    impl Watcher for Script {
        fn degraded(&self) -> Option<Degraded> {
            None
        }
        async fn next(&mut self) -> WatchEvent {
            match self.0.pop_front() {
                Some(event) => event,
                None => std::future::pending().await,
            }
        }
    }

    /// Ignores everything under `target/`.
    struct Rules;

    impl IgnoreRules for Rules {
        async fn unignored(&self, paths: Vec<RepoPath>) -> Result<Vec<RepoPath>, GitError> {
            Ok(paths.into_iter().filter(|p| !p.0.starts_with(b"target/")).collect())
        }
    }

    fn dirty(paths: &[&str]) -> WatchEvent {
        WatchEvent::Dirty(paths.iter().map(|p| RepoPath(p.as_bytes().to_vec())).collect())
    }

    #[tokio::test]
    async fn ignored_paths_are_dropped_and_the_rest_passes() {
        let script = [
            dirty(&["target/debug/x"]),
            dirty(&["target/a", "src/a.rs"]),
            WatchEvent::MustRescan,
        ];
        let mut watcher = IgnoringWatcher::new(Script(script.into()), Rules);
        assert_eq!(watcher.next().await, dirty(&["src/a.rs"]));
        assert_eq!(watcher.next().await, WatchEvent::MustRescan);
    }

    #[tokio::test]
    async fn a_cancelled_filter_does_not_lose_paths() {
        struct Never;
        impl IgnoreRules for Never {
            async fn unignored(&self, _: Vec<RepoPath>) -> Result<Vec<RepoPath>, GitError> {
                std::future::pending().await
            }
        }
        let mut watcher = IgnoringWatcher::new(Script([dirty(&["src/a.rs"])].into()), Never);
        let cancelled = tokio::time::timeout(std::time::Duration::from_millis(10), watcher.next()).await;
        assert!(cancelled.is_err());
        assert_eq!(watcher.next().await, dirty(&["src/a.rs"]));
    }
}
