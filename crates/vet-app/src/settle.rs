//! Settling (§6.2). `Settler` is the debounce state machine over an injected
//! clock reading. `Settling` drives it from watcher events and reconciles, so
//! size and mtime stability is judged by the scanner (metadata first), not by
//! the watcher: dropped events and polling-only topologies behave the same.

use std::collections::BTreeSet;
use std::pin::pin;
use std::task::Poll;

use vet_core::content::{RepoPath, SnapshotId};

use crate::acquisition::Acquisition;
use crate::ports::{Clock, Degraded, Dirty, GitError, GitRepo, Millis, Scan, Store, WatchEvent, Watcher};

pub struct Settler {
    debounce_ms: u64,
    all: bool,
    paths: BTreeSet<RepoPath>,
    last_change: Option<Millis>,
}

impl Settler {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            debounce_ms,
            all: false,
            paths: BTreeSet::new(),
            last_change: None,
        }
    }

    pub fn record(&mut self, dirty: Dirty, now: Millis) {
        match dirty {
            Dirty::All => self.all = true,
            Dirty::Paths(paths) => self.paths.extend(paths),
        }
        self.last_change = Some(now);
    }

    /// When the caller should next call [`Settler::take_settled`], if anything is pending.
    pub fn deadline(&self) -> Option<Millis> {
        self.last_change.map(|at| Millis(at.0 + self.debounce_ms))
    }

    /// The touched paths, once the debounce window has passed with no changes.
    pub fn take_settled(&mut self, now: Millis) -> Option<Dirty> {
        if now < self.deadline()? {
            return None;
        }
        self.last_change = None;
        let paths = std::mem::take(&mut self.paths);
        Some(if std::mem::take(&mut self.all) {
            Dirty::All
        } else {
            Dirty::Paths(paths.into_iter().collect())
        })
    }
}

enum Wake {
    Event(WatchEvent),
    Deadline,
}

/// Watcher events and the debounce deadline in, settled snapshots out. Every
/// rescan signal and every settle goes through [`Acquisition::reconcile`].
pub struct Settling<W, C> {
    watcher: W,
    clock: C,
    settler: Settler,
    /// Last snapshot returned by `next_settled`.
    settled: Option<SnapshotId>,
    /// Latest reconciled snapshot that differs from `settled` and is awaiting a quiet window.
    candidate: Option<SnapshotId>,
}

impl<W: Watcher, C: Clock> Settling<W, C> {
    pub fn new(watcher: W, clock: C, debounce_ms: u64, settled: Option<SnapshotId>) -> Self {
        Self {
            watcher,
            clock,
            settler: Settler::new(debounce_ms),
            settled,
            candidate: None,
        }
    }

    pub fn degraded(&self) -> Option<Degraded> {
        self.watcher.degraded()
    }

    /// Resolves with the next settled scan whose snapshot differs from the previous one.
    pub async fn next_settled<G: GitRepo, S: Store>(
        &mut self,
        acquisition: &Acquisition<G, S>,
    ) -> Result<Scan, GitError> {
        loop {
            match self.wake().await {
                Wake::Event(WatchEvent::Dirty(paths)) => self.settler.record(Dirty::Paths(paths), self.clock.now()),
                Wake::Event(WatchEvent::MustRescan | WatchEvent::Degraded(_)) => {
                    let snapshot = acquisition.reconcile(Dirty::All).await?.snapshot;
                    if Some(snapshot.id) != self.candidate.or(self.settled) {
                        self.candidate = Some(snapshot.id);
                        self.settler.record(Dirty::All, self.clock.now());
                    }
                }
                Wake::Deadline => {
                    let Some(dirty) = self.settler.take_settled(self.clock.now()) else {
                        continue;
                    };
                    let scan = match acquisition.reconcile(dirty.clone()).await {
                        Ok(scan) => scan,
                        Err(error) => {
                            // A later call retries after another quiet window.
                            self.settler.record(dirty, self.clock.now());
                            return Err(error);
                        }
                    };
                    let snapshot = &scan.snapshot;
                    if self.candidate.is_some_and(|candidate| candidate != snapshot.id) {
                        self.candidate = Some(snapshot.id);
                        self.settler.record(dirty, self.clock.now());
                    } else {
                        self.candidate = None;
                        if Some(snapshot.id) != self.settled {
                            self.settled = Some(snapshot.id);
                            return Ok(scan);
                        }
                    }
                }
            }
        }
    }

    async fn wake(&mut self) -> Wake {
        let mut event = pin!(self.watcher.next());
        let mut timer = self
            .settler
            .deadline()
            .map(|deadline| Box::pin(self.clock.sleep_until(deadline)));
        std::future::poll_fn(|cx| {
            if let Poll::Ready(event) = event.as_mut().poll(cx) {
                return Poll::Ready(Wake::Event(event));
            }
            match &mut timer {
                Some(timer) => timer.as_mut().poll(cx).map(|()| Wake::Deadline),
                None => Poll::Pending,
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Waker};

    use vet_core::content::{BlobId, Hash, Snapshot, SnapshotDiff, SnapshotStatus};

    use super::*;
    use crate::pin::CommitId;
    use crate::ports::{CheckoutError, GcReport, Scan, StoreError};
    use crate::review::SessionId;

    fn path(name: &str) -> RepoPath {
        RepoPath(name.as_bytes().to_vec())
    }

    fn id(n: u8) -> SnapshotId {
        SnapshotId(Hash([n; 32]))
    }

    #[test]
    fn a_burst_settles_once_after_the_window() {
        let mut settler = Settler::new(1500);
        settler.record(Dirty::Paths(vec![path("a.rs")]), Millis(0));
        settler.record(Dirty::Paths(vec![path("b.rs")]), Millis(1000));
        assert_eq!(settler.take_settled(Millis(2000)), None, "b.rs touched 1000 ms ago");
        assert_eq!(settler.deadline(), Some(Millis(2500)));
        assert_eq!(
            settler.take_settled(Millis(2500)),
            Some(Dirty::Paths(vec![path("a.rs"), path("b.rs")]))
        );
        assert_eq!(settler.take_settled(Millis(9999)), None, "nothing pending");
    }

    #[test]
    fn a_rescan_widens_the_dirty_set() {
        let mut settler = Settler::new(100);
        settler.record(Dirty::Paths(vec![path("a.rs")]), Millis(0));
        settler.record(Dirty::All, Millis(10));
        assert_eq!(settler.take_settled(Millis(110)), Some(Dirty::All));
    }

    /// Fake clock and watcher script sharing one timeline; time only moves when
    /// the next scripted event or the pending deadline is reached.
    #[derive(Default)]
    struct Sim {
        now: u64,
        script: VecDeque<(u64, WatchEvent)>,
        deadline: Option<u64>,
        scans: VecDeque<SnapshotId>,
        scanned: Vec<Dirty>,
    }

    type Shared = Arc<Mutex<Sim>>;

    struct FakeClock(Shared);
    struct FakeWatcher(Shared);
    struct FakeGit(Shared);
    struct FakeStore;

    struct Sleep {
        sim: Shared,
        deadline: u64,
    }

    impl Future for Sleep {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let mut sim = self.sim.lock().unwrap();
            if sim.now >= self.deadline {
                return Poll::Ready(());
            }
            if sim.script.front().is_none_or(|(at, _)| *at >= self.deadline) {
                sim.now = self.deadline;
                return Poll::Ready(());
            }
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    impl Drop for Sleep {
        fn drop(&mut self) {
            self.sim.lock().unwrap().deadline = None;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Millis {
            Millis(self.0.lock().unwrap().now)
        }
        fn sleep_until(&self, deadline: Millis) -> impl Future<Output = ()> + Send {
            self.0.lock().unwrap().deadline = Some(deadline.0);
            Sleep {
                sim: self.0.clone(),
                deadline: deadline.0,
            }
        }
    }

    impl Watcher for FakeWatcher {
        fn degraded(&self) -> Option<Degraded> {
            None
        }
        fn next(&mut self) -> impl Future<Output = WatchEvent> + Send {
            let sim = self.0.clone();
            std::future::poll_fn(move |_| {
                let mut sim = sim.lock().unwrap();
                let due = sim
                    .script
                    .front()
                    .is_some_and(|(at, _)| sim.deadline.is_none_or(|deadline| *at < deadline));
                if !due {
                    return Poll::Pending;
                }
                let (at, event) = sim.script.pop_front().unwrap();
                sim.now = sim.now.max(at);
                Poll::Ready(event)
            })
        }
    }

    impl GitRepo for FakeGit {
        async fn head(&self) -> Result<CommitId, GitError> {
            unreachable!()
        }
        async fn scan(&self, dirty: Dirty) -> Result<Scan, GitError> {
            let mut sim = self.0.lock().unwrap();
            sim.scanned.push(dirty);
            let id = sim.scans.pop_front().expect("unexpected scan");
            Ok(Scan {
                snapshot: Snapshot {
                    id,
                    entries: Default::default(),
                },
                status: Default::default(),
            })
        }
        async fn checkout<S: Store + Sync>(
            &self,
            _: &S,
            _: &CommitId,
            _: &Snapshot,
            _: &Path,
        ) -> Result<(), CheckoutError> {
            unreachable!()
        }
        async fn commit_snapshot(&self, _: &CommitId) -> Result<Snapshot, GitError> {
            unreachable!()
        }
        async fn read_blob(&self, _: &RepoPath, _: BlobId) -> Result<Option<Vec<u8>>, GitError> {
            unreachable!()
        }
        async fn submodule_commit(&self, _: BlobId) -> Result<Option<String>, GitError> {
            unreachable!()
        }
        async fn merge_base(&self, _: &str) -> Result<CommitId, GitError> {
            unreachable!()
        }
        async fn resolve_commit(&self, _: &str) -> Result<CommitId, GitError> {
            unreachable!()
        }
        async fn base_candidates(&self, _: usize) -> Result<crate::ports::BaseCandidates, GitError> {
            unreachable!()
        }
        async fn set_session_ref(&self, _: SessionId, _: &str, _: &CommitId) -> Result<(), GitError> {
            unreachable!()
        }
        async fn delete_session_refs(&self, _: SessionId) -> Result<(), GitError> {
            unreachable!()
        }
        async fn diff<S: Store + Sync>(&self, _: &S, _: &Snapshot, _: &Snapshot) -> Result<SnapshotDiff, GitError> {
            unreachable!()
        }
    }

    impl Store for FakeStore {
        async fn put_blob(&self, _: Vec<u8>) -> Result<BlobId, StoreError> {
            unreachable!()
        }
        async fn blob(&self, _: BlobId) -> Result<Option<Vec<u8>>, StoreError> {
            unreachable!()
        }
        async fn put_snapshot(&self, _: Snapshot, _: SnapshotStatus) -> Result<(), StoreError> {
            unreachable!()
        }
        async fn snapshot(&self, _: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
            unreachable!()
        }
        async fn append_timeline(&self, _: SessionId, _: SnapshotId) -> Result<(), StoreError> {
            unreachable!()
        }
        async fn timeline(&self, _: SessionId) -> Result<Vec<SnapshotId>, StoreError> {
            unreachable!()
        }
        async fn gc(&self, _: i64) -> Result<GcReport, StoreError> {
            unreachable!()
        }
    }

    struct Harness {
        sim: Shared,
        settling: Settling<FakeWatcher, FakeClock>,
        acquisition: Acquisition<FakeGit, FakeStore>,
    }

    fn harness(script: Vec<(u64, WatchEvent)>, scans: Vec<SnapshotId>, settled: Option<SnapshotId>) -> Harness {
        let sim = Arc::new(Mutex::new(Sim {
            script: script.into(),
            scans: scans.into(),
            ..Sim::default()
        }));
        Harness {
            settling: Settling::new(FakeWatcher(sim.clone()), FakeClock(sim.clone()), 1500, settled),
            acquisition: Acquisition::new(FakeGit(sim.clone()), FakeStore),
            sim,
        }
    }

    /// Polls until ready; `None` if the future stays pending with nothing left to happen.
    fn run<T>(future: impl Future<Output = T>) -> Option<T> {
        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        (0..100).find_map(|_| match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => Some(value),
            Poll::Pending => None,
        })
    }

    fn dirty(name: &str) -> WatchEvent {
        WatchEvent::Dirty(vec![path(name)])
    }

    #[test]
    fn a_write_burst_settles_after_the_quiet_window() {
        let mut h = harness(vec![(0, dirty("a.rs")), (1000, dirty("b.rs"))], vec![id(1)], None);
        let snapshot = run(h.settling.next_settled(&h.acquisition)).unwrap().unwrap().snapshot;
        assert_eq!(snapshot.id, id(1));
        let sim = h.sim.lock().unwrap();
        assert_eq!(sim.now, 2500);
        assert_eq!(sim.scanned, vec![Dirty::Paths(vec![path("a.rs"), path("b.rs")])]);
    }

    #[test]
    fn a_snapshot_that_keeps_changing_does_not_settle() {
        let script = vec![(0, WatchEvent::MustRescan), (1000, WatchEvent::MustRescan)];
        let mut h = harness(script, vec![id(1), id(2), id(2)], None);
        let snapshot = run(h.settling.next_settled(&h.acquisition)).unwrap().unwrap().snapshot;
        assert_eq!(snapshot.id, id(2));
        assert_eq!(h.sim.lock().unwrap().now, 2500, "second rescan restarted the window");
    }

    #[test]
    fn an_unchanged_snapshot_is_not_reported() {
        let mut h = harness(
            vec![(0, dirty("a.rs")), (5000, WatchEvent::MustRescan)],
            vec![id(1), id(1)],
            Some(id(1)),
        );
        assert!(run(h.settling.next_settled(&h.acquisition)).is_none());
        assert!(h.sim.lock().unwrap().scans.is_empty());
    }

    #[test]
    fn a_rescan_signal_reconciles_immediately() {
        let mut h = harness(
            vec![(0, WatchEvent::Degraded(Degraded::WatchLimit))],
            vec![id(1), id(1)],
            None,
        );
        run(h.settling.next_settled(&h.acquisition)).unwrap().unwrap();
        assert_eq!(h.sim.lock().unwrap().scanned, vec![Dirty::All, Dirty::All]);
    }
}
