//! The background runs of one project: which of them is alive, how often each kind started, and
//! how they end together when the project closes. A run is its own task, so one that panics
//! takes no other down. The mechanism knows nothing of what the runs do.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::sync::watch;
use tokio::task::{AbortHandle, JoinSet};

use crate::mutex::lock;

/// How long `Supervisor::shutdown` waits for the runs to end after cancelling them.
pub const SHUTDOWN_BOUND: Duration = Duration::from_secs(5);

/// How often each kind of run started and how many of its runs are alive: `(started, live)`.
type Counts = HashMap<&'static str, (usize, usize)>;

/// The runs of one project. Cloning gives another handle on the same runs.
#[derive(Clone)]
pub struct Supervisor(Arc<Shared>);

struct Shared {
    /// Locked on its own, never together with `runs`: dropping a run's future ends its `Counted`.
    counts: Mutex<Counts>,
    runs: Mutex<Runs>,
    bound: Duration,
}

#[derive(Default)]
struct Runs {
    /// The latest run of each slot; a newer run of the slot aborts it.
    slots: HashMap<&'static str, AbortHandle>,
    tasks: JoinSet<()>,
    /// Runs that end themselves when they see `closed`, which `shutdown` waits for before it aborts.
    cooperative: JoinSet<()>,
    /// Set by `shutdown`: nothing starts after it.
    closed: bool,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::within(SHUTDOWN_BOUND)
    }
}

impl Supervisor {
    /// A supervisor whose shutdown waits at most `bound` for the runs to end.
    pub fn within(bound: Duration) -> Self {
        Self(Arc::new(Shared {
            counts: Mutex::default(),
            runs: Mutex::default(),
            bound,
        }))
    }

    /// Counts a run of `kind` that is starting; dropping the guard says it ended, whether it
    /// returned, was aborted or panicked.
    pub fn counted(&self, kind: &'static str) -> Counted {
        {
            let mut all = lock(&self.0.counts);
            let counts = all.entry(kind).or_default();
            counts.0 += 1;
            counts.1 += 1;
        }
        Counted {
            shared: self.0.clone(),
            kind,
        }
    }

    /// `work` as a run of `kind`, counted from this call on until the future is dropped or done.
    pub fn track<F: Future>(&self, kind: &'static str, work: F) -> impl Future<Output = F::Output> + use<F> {
        let counted = self.counted(kind);
        async move {
            let _counted = counted;
            work.await
        }
    }

    /// Runs `work` as the latest run of `slot`: the slot's older run is aborted.
    pub fn replace(&self, slot: &'static str, work: impl Future<Output = ()> + Send + 'static) {
        self.spawn(slot, Some(slot), work);
    }

    /// Runs `work` in no slot, so only `shutdown` ends it early. `label` names it in the log.
    pub fn start(&self, label: &'static str, work: impl Future<Output = ()> + Send + 'static) {
        self.spawn(label, None, work);
    }

    /// Runs `work` in no slot, and never aborts it for a newer run. It is to end by itself once
    /// `closed` says so; `shutdown` waits for that, up to its bound, before it aborts the run.
    pub fn cooperate(&self, label: &'static str, work: impl Future<Output = ()> + Send + 'static) {
        let work = isolated(label, work);
        let mut runs = lock(&self.0.runs);
        if runs.closed {
            return;
        }
        while runs.cooperative.try_join_next().is_some() {}
        runs.cooperative.spawn(work);
    }

    /// Whether `shutdown` began: runs that cooperate stop at their next step.
    pub fn closed(&self) -> bool {
        lock(&self.0.runs).closed
    }

    /// Aborts the run in `slot`, if one is alive.
    pub fn stop(&self, slot: &'static str) {
        if let Some(run) = lock(&self.0.runs).slots.remove(slot) {
            run.abort();
        }
    }

    /// How many runs are alive now, of every kind.
    pub fn live(&self) -> usize {
        lock(&self.0.counts).values().map(|(_, live)| live).sum()
    }

    /// How often runs of `kind` started and how many are alive now.
    pub fn counts(&self, kind: &'static str) -> (usize, usize) {
        lock(&self.0.counts).get(kind).copied().unwrap_or_default()
    }

    /// Ends every run and waits until none is left, so whatever they used can be torn down after.
    /// Runs that cooperate get the bound to end by themselves, the others are aborted at once. A
    /// run that does not end within the bound after its abort is left behind and logged. Nothing
    /// starts after.
    pub async fn shutdown(&self) {
        let (mut tasks, mut cooperative) = {
            let mut runs = lock(&self.0.runs);
            runs.closed = true;
            runs.slots.clear();
            (std::mem::take(&mut runs.tasks), std::mem::take(&mut runs.cooperative))
        };
        tasks.abort_all();
        let ended = tokio::time::timeout(self.0.bound, async {
            while cooperative.join_next().await.is_some() {}
        })
        .await;
        if ended.is_err() {
            tracing::warn!(left = cooperative.len(), "cooperating runs did not end in time");
            cooperative.abort_all();
        }
        let joined = tokio::time::timeout(self.0.bound, async {
            while tasks.join_next().await.is_some() {}
            while cooperative.join_next().await.is_some() {}
        })
        .await;
        if joined.is_err() {
            tracing::warn!(left = tasks.len() + cooperative.len(), "background runs did not end in time");
            tasks.detach_all();
            cooperative.detach_all();
        }
    }

    fn spawn(&self, label: &'static str, slot: Option<&'static str>, work: impl Future<Output = ()> + Send + 'static) {
        let work = isolated(label, work);
        let mut runs = lock(&self.0.runs);
        if runs.closed {
            return;
        }
        // Runs that ended are collected as new ones come, so the set stays as small as the runs alive.
        while runs.tasks.try_join_next().is_some() {}
        let run = runs.tasks.spawn(work);
        if let Some(slot) = slot
            && let Some(older) = runs.slots.insert(slot, run)
        {
            older.abort();
        }
    }
}

/// The kind under which the loops of a trigger's runs are counted, so the supervisor is not idle
/// between one run and the one that waits.
const LOOP: &str = "triggered";

/// Work that requests ask for. One run goes on at a time and at most one waits behind it; a request
/// that comes while a run waits is merged into it. A request is answered with a ticket that is done
/// when a run that started after the request has ended, so no request is answered by a run that
/// began before it.
///
/// The runs cooperate with `Supervisor::shutdown`: none is aborted for a newer one, and none starts
/// once the supervisor is closed.
pub struct Trigger<R, O>(Arc<Triggered<R, O>>);

impl<R, O> Clone for Trigger<R, O> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct Triggered<R, O> {
    supervisor: Supervisor,
    kind: &'static str,
    /// Adds the reasons of a later request to those of the run that waits.
    merge: fn(&mut R, R),
    run: Box<dyn Fn(R) -> BoxFuture<'static, O> + Send + Sync>,
    state: Mutex<TriggerState<R, O>>,
}

struct TriggerState<R, O> {
    /// Whether a loop of runs is alive, which takes the waiting run when it is done with the one before.
    running: bool,
    /// The run that waits, with the reasons merged so far and where its outcome goes.
    pending: Option<(R, watch::Sender<Option<O>>)>,
}

impl<R: Send + 'static, O: Clone + Send + Sync + 'static> Trigger<R, O> {
    /// A trigger whose runs are `run`, each counted under `kind`.
    pub fn new(
        supervisor: Supervisor,
        kind: &'static str,
        merge: fn(&mut R, R),
        run: impl Fn(R) -> BoxFuture<'static, O> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Triggered {
            supervisor,
            kind,
            merge,
            run: Box::new(run),
            state: Mutex::new(TriggerState {
                running: false,
                pending: None,
            }),
        }))
    }

    /// Asks for a run with `reason`: merged into the run that waits, or becoming it.
    pub fn request(&self, reason: R) -> Ticket<O> {
        let (ticket, start) = {
            let mut state = lock(&self.0.state);
            let ticket = match &mut state.pending {
                Some((merged, done)) => {
                    (self.0.merge)(merged, reason);
                    Ticket(done.subscribe())
                }
                None => {
                    let (done, ticket) = watch::channel(None);
                    state.pending = Some((reason, done));
                    Ticket(ticket)
                }
            };
            (ticket, !std::mem::replace(&mut state.running, true))
        };
        if start {
            self.spawn();
        }
        ticket
    }

    fn spawn(&self) {
        let shared = self.0.clone();
        let (supervisor, kind) = (shared.supervisor.clone(), shared.kind);
        let counted = supervisor.counted(LOOP);
        let ending = Ending {
            shared: shared.clone(),
            over: false,
        };
        supervisor.cooperate(kind, async move {
            let _counted = counted;
            let mut ending = ending;
            while !shared.supervisor.closed() {
                let next = {
                    let mut state = lock(&shared.state);
                    let next = state.pending.take();
                    state.running = next.is_some();
                    next
                };
                let Some((reason, done)) = next else {
                    ending.over = true;
                    return;
                };
                let outcome = shared.supervisor.track(shared.kind, (shared.run)(reason)).await;
                done.send_replace(Some(outcome));
            }
        });
    }
}

/// Clears what a loop of runs leaves behind when it is dropped before it ran out of work, whether
/// aborted, panicked or never started: the requests waiting for it are never answered.
struct Ending<R, O> {
    shared: Arc<Triggered<R, O>>,
    over: bool,
}

impl<R, O> Drop for Ending<R, O> {
    fn drop(&mut self) {
        if !self.over {
            let mut state = lock(&self.shared.state);
            state.running = false;
            state.pending = None;
        }
    }
}

/// What a request waits on.
pub struct Ticket<O>(watch::Receiver<Option<O>>);

impl<O: Clone> Ticket<O> {
    /// The outcome of the run that answers the request; none if that run never finished.
    pub async fn wait(mut self) -> Option<O> {
        let done = self.0.wait_for(Option::is_some).await.ok()?;
        done.clone()
    }
}

/// The end of one counted run.
pub struct Counted {
    shared: Arc<Shared>,
    kind: &'static str,
}

impl Drop for Counted {
    fn drop(&mut self) {
        if let Some(counts) = lock(&self.shared.counts).get_mut(self.kind) {
            counts.1 = counts.1.saturating_sub(1);
        }
    }
}

/// `work`, with a panic in it logged and ended there.
async fn isolated(label: &'static str, work: impl Future<Output = ()>) {
    if let Err(panic) = AssertUnwindSafe(work).catch_unwind().await {
        let message = panic
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("without a message");
        tracing::error!(run = label, message, "a background run panicked");
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::io;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use tokio::sync::{Notify, oneshot};
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    /// Set when the value that holds it is dropped: whoever owns a future that owns one can see the
    /// future end.
    #[derive(Clone, Default)]
    struct Dropped(Arc<AtomicBool>);

    impl Dropped {
        fn is_set(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }

        /// A run that never ends by itself and says when it is dropped.
        fn run(&self) -> impl Future<Output = ()> + Send + 'static {
            let held = Held(self.clone());
            async move {
                let _held = held;
                pending::<()>().await;
            }
        }
    }

    struct Held(Dropped);

    impl Drop for Held {
        fn drop(&mut self) {
            (self.0).0.store(true, Ordering::SeqCst);
        }
    }

    async fn eventually(mut condition: impl FnMut() -> bool) {
        let waiting = async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the condition never held");
    }

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);

    impl Log {
        fn text(&self) -> String {
            String::from_utf8_lossy(&lock(&self.0)).into_owned()
        }
    }

    impl io::Write for Log {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            lock(&self.0).extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl MakeWriter<'_> for Log {
        type Writer = Log;

        fn make_writer(&self) -> Log {
            self.clone()
        }
    }

    #[tokio::test]
    async fn a_newer_run_in_a_slot_and_stop_abort_the_run_there_and_no_other() {
        let supervisor = Supervisor::default();
        let (first, second, elsewhere) = (Dropped::default(), Dropped::default(), Dropped::default());
        supervisor.replace("analysis", first.run());
        supervisor.replace("checks", elsewhere.run());

        supervisor.replace("analysis", second.run());
        eventually(|| first.is_set()).await;
        assert!(!second.is_set() && !elsewhere.is_set());

        supervisor.stop("analysis");
        eventually(|| second.is_set()).await;
        assert!(!elsewhere.is_set());
    }

    #[tokio::test]
    async fn a_panic_is_logged_and_the_other_runs_go_on() {
        let log = Log::default();
        let _logging = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(log.clone())
                .with_ansi(false)
                .finish(),
        );
        let supervisor = Supervisor::default();
        let go = Arc::new(Notify::new());
        let finished = Dropped::default();
        supervisor.replace("bystander", {
            let (go, held) = (go.clone(), Held(finished.clone()));
            async move {
                go.notified().await;
                drop(held);
            }
        });

        supervisor.replace(
            "failing",
            supervisor.track("failing", async { panic!("the analysis blew up") }),
        );
        eventually(|| supervisor.counts("failing") == (1, 0)).await;
        assert!(log.text().contains("the analysis blew up"), "{}", log.text());
        assert!(!finished.is_set());

        go.notify_one();
        eventually(|| finished.is_set()).await;
        // The slot of the run that panicked takes a new run.
        let again = Dropped::default();
        supervisor.replace("failing", again.run());
        supervisor.stop("failing");
        eventually(|| again.is_set()).await;
    }

    #[tokio::test]
    async fn shutdown_returns_when_every_run_has_ended_and_nothing_starts_after() {
        let supervisor = Supervisor::default();
        let (slotted, unslotted) = (Dropped::default(), Dropped::default());
        supervisor.replace("analysis", slotted.run());
        supervisor.start("retire", unslotted.run());
        tokio::task::yield_now().await;

        supervisor.shutdown().await;

        assert!(slotted.is_set() && unslotted.is_set());
        let started = Arc::new(AtomicBool::new(false));
        supervisor.start("late", {
            let started = started.clone();
            async move { started.store(true, Ordering::SeqCst) }
        });
        supervisor.replace("analysis", pending());
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        assert!(!started.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_stops_waiting_for_a_run_that_never_yields() {
        let supervisor = Supervisor::within(Duration::from_millis(50));
        let (blocked, blocking) = oneshot::channel();
        supervisor.start("stuck", async move {
            let _ = blocked.send(());
            std::thread::sleep(Duration::from_secs(1));
        });
        blocking.await.unwrap();

        let began = Instant::now();
        supervisor.shutdown().await;

        assert!(began.elapsed() < Duration::from_millis(500), "{:?}", began.elapsed());
    }

    /// A trigger whose run notes its reasons, waits for `gate` and answers with how many runs began.
    fn gated(
        supervisor: &Supervisor,
        gate: &Arc<Notify>,
    ) -> (Trigger<Vec<u32>, usize>, Arc<Mutex<Vec<Vec<u32>>>>) {
        let began = Arc::new(Mutex::new(Vec::new()));
        let trigger = Trigger::new(
            supervisor.clone(),
            "sync",
            |merged: &mut Vec<u32>, more| merged.extend(more),
            {
                let (began, gate) = (began.clone(), gate.clone());
                move |reasons| {
                    let (began, gate) = (began.clone(), gate.clone());
                    async move {
                        let number = {
                            let mut began = lock(&began);
                            began.push(reasons);
                            began.len()
                        };
                        gate.notified().await;
                        number
                    }
                    .boxed()
                }
            },
        );
        (trigger, began)
    }

    #[tokio::test]
    async fn requests_during_a_run_share_one_later_run_that_started_after_them() {
        let (supervisor, gate) = (Supervisor::default(), Arc::new(Notify::new()));
        let (trigger, began) = gated(&supervisor, &gate);

        let first = trigger.request(vec![1]);
        eventually(|| lock(&began).len() == 1).await;
        let later = [trigger.request(vec![2]), trigger.request(vec![3]), trigger.request(vec![4])];

        gate.notify_one();
        assert_eq!(first.wait().await, Some(1));
        eventually(|| lock(&began).len() == 2).await;
        gate.notify_one();
        for ticket in later {
            assert_eq!(ticket.wait().await, Some(2));
        }
        assert_eq!(*lock(&began), [vec![1], vec![2, 3, 4]]);
        eventually(|| supervisor.live() == 0).await;
        assert_eq!(supervisor.counts("sync"), (2, 0), "two runs for four requests");
    }

    #[tokio::test]
    async fn a_request_after_the_runs_ended_starts_a_new_one() {
        let (supervisor, gate) = (Supervisor::default(), Arc::new(Notify::new()));
        let (trigger, began) = gated(&supervisor, &gate);
        gate.notify_one();
        assert_eq!(trigger.request(vec![1]).wait().await, Some(1));
        eventually(|| supervisor.live() == 0).await;

        gate.notify_one();
        assert_eq!(trigger.request(vec![2]).wait().await, Some(2));
        assert_eq!(*lock(&began), [vec![1], vec![2]]);
    }

    #[tokio::test]
    async fn shutdown_gives_a_run_the_bound_then_aborts_it_and_answers_no_request() {
        let supervisor = Supervisor::within(Duration::from_millis(50));
        let (trigger, began) = gated(&supervisor, &Arc::new(Notify::new()));
        let running = trigger.request(vec![1]);
        eventually(|| lock(&began).len() == 1).await;
        let waiting = trigger.request(vec![2]);

        let shut = Instant::now();
        supervisor.shutdown().await;

        assert!(shut.elapsed() >= Duration::from_millis(50), "the run got its bound");
        assert!(shut.elapsed() < Duration::from_millis(500), "{:?}", shut.elapsed());
        assert_eq!((running.wait().await, waiting.wait().await), (None, None));
        assert_eq!(trigger.request(vec![3]).wait().await, None);
        assert_eq!(lock(&began).len(), 1, "nothing started after");
    }

    #[tokio::test]
    async fn the_counters_follow_each_kind_through_start_replace_finish_abort_and_panic() {
        let supervisor = Supervisor::default();
        assert_eq!(supervisor.counts("work"), (0, 0));

        supervisor.start("done", supervisor.track("work", async {}));
        eventually(|| supervisor.counts("work") == (1, 0)).await;

        // The second run of the slot replaces the first, which ends.
        supervisor.replace("slot", supervisor.track("work", pending()));
        supervisor.replace("slot", supervisor.track("work", pending()));
        eventually(|| supervisor.counts("work") == (3, 1)).await;

        supervisor.start("failing", supervisor.track("work", async { panic!("failed") }));
        eventually(|| supervisor.counts("work") == (4, 1)).await;

        let other = supervisor.counted("other");
        assert_eq!(supervisor.counts("other"), (1, 1));
        assert_eq!(supervisor.live(), 2);

        supervisor.stop("slot");
        eventually(|| supervisor.counts("work") == (4, 0)).await;
        drop(other);
        assert_eq!((supervisor.counts("other"), supervisor.live()), ((1, 0), 0));
    }
}
