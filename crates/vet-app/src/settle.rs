//! Settling (§6.2). `Settler` is the debounce state machine over an injected
//! clock reading. `Settling` drives it from watcher events and reconciles, so
//! size and mtime stability is judged by the scanner (metadata first), not by
//! the watcher: dropped events and polling-only topologies behave the same.

use std::collections::BTreeSet;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

use tokio::sync::Notify;
use vet_core::content::{RepoPath, SnapshotId};

use crate::acquisition::Acquisition;
use crate::ports::{Clock, Degraded, Dirty, GitError, Millis, Scan, Scanner, WatchEvent, Watcher};

/// A request to reconcile the working tree now, for a client that must not miss an edit the
/// watcher has not delivered yet. The settle loop takes it when it next looks for work.
#[derive(Default)]
pub struct SettleFlush {
    requested: AtomicBool,
    wake: Notify,
}

/// Whether the settled tree is wanted: while it is not, file changes only mark it changed and
/// are not scanned, and the settle loop waits for it to want again. The session flips it as the
/// working tree goes off and on view or its review opens and ends; a flip wakes the loop.
#[derive(Debug)]
pub struct Wanted {
    wanted: AtomicBool,
    wake: Notify,
}

impl Wanted {
    pub fn new(wanted: bool) -> Self {
        Self {
            wanted: AtomicBool::new(wanted),
            wake: Notify::new(),
        }
    }

    /// Whether the settled tree is wanted now.
    pub fn get(&self) -> bool {
        self.wanted.load(Ordering::SeqCst)
    }

    /// Says whether the settled tree is wanted now; a change wakes the settle loop.
    pub fn set(&self, wanted: bool) {
        if self.wanted.swap(wanted, Ordering::SeqCst) != wanted {
            self.wake.notify_one();
        }
    }

    fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.wake.notified()
    }
}

impl SettleFlush {
    /// Asks the settle loop to reconcile now.
    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// Whether a reconcile was asked for and not taken yet.
    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    /// Takes the request; whether there was one.
    fn take(&self) -> bool {
        self.requested.swap(false, Ordering::SeqCst)
    }

    fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.wake.notified()
    }
}

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

/// What [`Settling::next_settled`] resolved with.
pub enum Settled {
    /// The working tree settled.
    Tree(Scan),
    /// A branch ref was written; the branch on view may have a new tip.
    RefsMoved,
    /// The settled tree has stayed as it is for the quiet period of [`Settling::quiet_after`].
    Quiet,
    /// Whether edits wait for their window to pass, as [`Settling::reporting_edits`] asked to be
    /// told: `true` when the first touch started one, `false` when it ended.
    Edits(bool),
}

enum Wake {
    Event(WatchEvent),
    Deadline,
    Quiet,
    /// The settled tree came to be wanted, or stopped being wanted; look at it again.
    Wanted,
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
    /// How long the settled tree must stay untouched before it counts as quiet.
    quiet_ms: Option<u64>,
    /// While the settled tree is owed a [`Settled::Quiet`]: when it was last touched, or settled.
    untouched_since: Option<Millis>,
    /// Whether to resolve with [`Settled::Edits`] when edits start and stop waiting.
    reporting_edits: bool,
    /// What [`Settled::Edits`] last said.
    edits_reported: bool,
    /// A client's request to reconcile now, taken on the next look for work.
    flush: Option<Arc<SettleFlush>>,
    /// Whether the settled tree is wanted; none is always. While it is not, edits mark it
    /// changed and are not reconciled until it is wanted again.
    wanted: Option<Arc<Wanted>>,
    /// The tree was touched while it was not wanted, so reconciling it whole is owed.
    owed: bool,
}

impl<W: Watcher, C: Clock> Settling<W, C> {
    pub fn new(watcher: W, clock: C, debounce_ms: u64, settled: Option<SnapshotId>) -> Self {
        Self {
            watcher,
            clock,
            settler: Settler::new(debounce_ms),
            settled,
            candidate: None,
            quiet_ms: None,
            untouched_since: None,
            reporting_edits: false,
            edits_reported: false,
            flush: None,
            wanted: None,
            owed: false,
        }
    }

    /// Takes [`SettleFlush`] requests: a client that must not miss an edit the watcher has not
    /// delivered yet asks for a reconcile, and the next look for work makes one.
    pub fn flushable(mut self, flush: Arc<SettleFlush>) -> Self {
        self.flush = Some(flush);
        self
    }

    /// Takes the gate on the settled tree: while it is not wanted, file changes only mark it
    /// changed and are not reconciled until it is wanted again.
    pub fn wanted(mut self, wanted: Arc<Wanted>) -> Self {
        self.wanted = Some(wanted);
        self
    }

    /// Whether the settled tree is wanted now.
    fn wanted_now(&self) -> bool {
        self.wanted.as_ref().is_none_or(|wanted| wanted.get())
    }

    /// Also resolves with [`Settled::Edits`] whenever edits start or stop waiting to settle.
    pub fn reporting_edits(mut self) -> Self {
        self.reporting_edits = true;
        self
    }

    /// Also resolves with [`Settled::Quiet`] once after each settled tree (and after the one it
    /// starts with), when nothing in the tree was touched for `quiet_ms` since it settled.
    pub fn quiet_after(mut self, quiet_ms: u64) -> Self {
        self.quiet_ms = Some(quiet_ms);
        self.untouched_since = self.settled.map(|_| self.clock.now());
        self
    }

    /// Something in the tree was touched: a quiet period owed starts again.
    fn touched(&mut self) {
        if self.untouched_since.is_some() {
            self.untouched_since = Some(self.clock.now());
        }
    }

    /// When the settled tree counts as quiet, if it is owed that and nothing waits to settle.
    fn quiet_deadline(&self) -> Option<Millis> {
        if self.settler.deadline().is_some() {
            return None;
        }
        Some(Millis(self.untouched_since?.0 + self.quiet_ms?))
    }

    pub fn degraded(&self) -> Option<Degraded> {
        self.watcher.degraded()
    }

    /// Resolves with the next settled scan whose snapshot differs from the previous one, or
    /// as soon as a branch ref is written: refs are looked at as they move, not debounced.
    pub async fn next_settled<G: Scanner, S>(&mut self, acquisition: &Acquisition<G, S>) -> Result<Settled, GitError> {
        loop {
            let wanted = self.wanted_now();
            let waiting = self.settler.deadline().is_some() && wanted;
            if self.reporting_edits && waiting != self.edits_reported {
                self.edits_reported = waiting;
                return Ok(Settled::Edits(waiting));
            }
            if wanted && std::mem::take(&mut self.owed) {
                // The tree was touched while it was not wanted: reconcile it whole, as on resume.
                self.settler.record(Dirty::All, self.clock.now());
            }
            match self.wake().await {
                Wake::Wanted => {}
                Wake::Event(WatchEvent::RefsMoved) => return Ok(Settled::RefsMoved),
                Wake::Event(WatchEvent::Dirty(paths)) => {
                    self.touched();
                    if self.wanted_now() {
                        self.settler.record(Dirty::Paths(paths), self.clock.now());
                    } else {
                        self.owed = true;
                    }
                }
                Wake::Event(WatchEvent::MustRescan | WatchEvent::Degraded(_)) => {
                    self.touched();
                    if !self.wanted_now() {
                        self.owed = true;
                        continue;
                    }
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
                            if self.quiet_ms.is_some() {
                                self.untouched_since = Some(self.clock.now());
                            }
                            return Ok(Settled::Tree(scan));
                        }
                    }
                }
                Wake::Quiet => {
                    if self.quiet_deadline().is_some_and(|at| self.clock.now() >= at) {
                        self.untouched_since = None;
                        return Ok(Settled::Quiet);
                    }
                }
            }
        }
    }

    async fn wake(&mut self) -> Wake {
        let wanted = self.wanted_now();
        let quiet_deadline = if wanted { self.quiet_deadline() } else { None };
        let mut event = pin!(self.watcher.next());
        let mut timer = if wanted {
            self.settler
                .deadline()
                .map(|deadline| Box::pin(self.clock.sleep_until(deadline)))
        } else {
            None
        };
        let mut quiet = quiet_deadline.map(|deadline| Box::pin(self.clock.sleep_until(deadline)));
        let flush = self.flush.clone();
        let mut flushed = flush.as_ref().map(|flush| Box::pin(flush.notified()));
        let gate = self.wanted.clone();
        let mut flipped = gate.as_ref().map(|gate| Box::pin(gate.notified()));
        std::future::poll_fn(|cx| {
            if let Some(flipped) = flipped.as_mut()
                && flipped.as_mut().poll(cx).is_ready()
            {
                return Poll::Ready(Wake::Wanted);
            }
            if let Some(flushed) = flushed.as_mut()
                && flushed.as_mut().poll(cx).is_ready()
                && let Some(flush) = &flush
                && flush.take()
            {
                return Poll::Ready(Wake::Event(WatchEvent::MustRescan));
            }
            if let Poll::Ready(event) = event.as_mut().poll(cx) {
                return Poll::Ready(Wake::Event(event));
            }
            if let Some(Poll::Ready(())) = timer.as_mut().map(|t| t.as_mut().poll(cx)) {
                return Poll::Ready(Wake::Deadline);
            }
            match &mut quiet {
                Some(quiet) => quiet.as_mut().poll(cx).map(|()| Wake::Quiet),
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
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Waker};

    use vet_core::content::{Hash, Snapshot};

    use super::*;

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

    impl Scanner for FakeGit {
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
    }

    struct Harness {
        sim: Shared,
        settling: Settling<FakeWatcher, FakeClock>,
        acquisition: Acquisition<FakeGit, ()>,
    }

    fn harness(script: Vec<(u64, WatchEvent)>, scans: Vec<SnapshotId>, settled: Option<SnapshotId>) -> Harness {
        let sim = Arc::new(Mutex::new(Sim {
            script: script.into(),
            scans: scans.into(),
            ..Sim::default()
        }));
        Harness {
            settling: Settling::new(FakeWatcher(sim.clone()), FakeClock(sim.clone()), 1500, settled),
            acquisition: Acquisition::new(FakeGit(sim.clone()), ()),
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

    fn tree(settled: Settled) -> Scan {
        match settled {
            Settled::Tree(scan) => scan,
            _ => panic!("expected a settled tree"),
        }
    }

    /// Like `harness`, owing a quiet period of 3000 ms.
    fn quiet(script: Vec<(u64, WatchEvent)>, scans: Vec<SnapshotId>) -> Harness {
        let h = harness(script, scans, Some(id(0)));
        Harness {
            settling: h.settling.quiet_after(3000),
            ..h
        }
    }

    /// The next result of `h` and when it came.
    fn next(h: &mut Harness) -> (Settled, u64) {
        let settled = run(h.settling.next_settled(&h.acquisition)).unwrap().unwrap();
        (settled, h.sim.lock().unwrap().now)
    }

    #[test]
    fn the_tree_it_starts_with_is_quiet_once_after_the_quiet_period() {
        let mut h = quiet(vec![], vec![]);
        assert!(matches!(next(&mut h), (Settled::Quiet, 3000)));
        assert!(run(h.settling.next_settled(&h.acquisition)).is_none());
    }

    #[test]
    fn a_settle_within_the_quiet_period_starts_it_again() {
        let mut h = quiet(vec![(1000, dirty("a.rs"))], vec![id(1)]);
        assert!(matches!(next(&mut h), (Settled::Tree(_), 2500)));
        assert!(matches!(next(&mut h), (Settled::Quiet, 5500)));
    }

    #[test]
    fn the_tree_is_not_quiet_while_edits_keep_coming() {
        let edits = (1..=5).map(|n| (n * 1000, dirty("a.rs"))).collect();
        let mut h = quiet(edits, vec![id(1)]);
        assert!(matches!(next(&mut h), (Settled::Tree(_), 6500)));
        assert!(matches!(next(&mut h), (Settled::Quiet, 9500)));
    }

    #[test]
    fn an_edit_that_settles_to_the_same_tree_starts_the_quiet_period_again() {
        let mut h = quiet(vec![(1000, dirty("a.rs"))], vec![id(0)]);
        assert!(matches!(next(&mut h), (Settled::Quiet, 4000)));
    }

    #[test]
    fn edits_are_reported_while_they_wait_to_settle() {
        let h = harness(
            vec![(1000, dirty("a.rs")), (1500, dirty("b.rs"))],
            vec![id(1)],
            Some(id(0)),
        );
        let mut h = Harness {
            settling: h.settling.reporting_edits(),
            ..h
        };
        assert!(matches!(next(&mut h), (Settled::Edits(true), 1000)));
        assert!(matches!(next(&mut h), (Settled::Tree(_), 3000)));
        assert!(matches!(next(&mut h), (Settled::Edits(false), 3000)));
    }

    #[test]
    fn edits_that_settle_to_the_same_tree_stop_waiting_too() {
        let h = harness(vec![(1000, dirty("a.rs"))], vec![id(0)], Some(id(0)));
        let mut h = Harness {
            settling: h.settling.reporting_edits(),
            ..h
        };
        assert!(matches!(next(&mut h), (Settled::Edits(true), 1000)));
        assert!(matches!(next(&mut h), (Settled::Edits(false), 2500)));
    }

    #[test]
    fn a_write_burst_settles_after_the_quiet_window() {
        let mut h = harness(vec![(0, dirty("a.rs")), (1000, dirty("b.rs"))], vec![id(1)], None);
        let snapshot = tree(run(h.settling.next_settled(&h.acquisition)).unwrap().unwrap()).snapshot;
        assert_eq!(snapshot.id, id(1));
        let sim = h.sim.lock().unwrap();
        assert_eq!(sim.now, 2500);
        assert_eq!(sim.scanned, vec![Dirty::Paths(vec![path("a.rs"), path("b.rs")])]);
    }

    #[test]
    fn a_snapshot_that_keeps_changing_does_not_settle() {
        let script = vec![(0, WatchEvent::MustRescan), (1000, WatchEvent::MustRescan)];
        let mut h = harness(script, vec![id(1), id(2), id(2)], None);
        let snapshot = tree(run(h.settling.next_settled(&h.acquisition)).unwrap().unwrap()).snapshot;
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
    fn a_ref_write_is_reported_at_once_without_a_scan() {
        let mut h = harness(vec![(0, WatchEvent::RefsMoved)], vec![], None);
        assert!(matches!(
            run(h.settling.next_settled(&h.acquisition)).unwrap().unwrap(),
            Settled::RefsMoved
        ));
        let sim = h.sim.lock().unwrap();
        assert_eq!(sim.now, 0);
        assert!(sim.scanned.is_empty());
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
