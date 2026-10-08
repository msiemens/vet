use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::task::Waker;

use super::*;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Key {
    Kept(u8),
    InFlight(u8),
    Sampled { id: u8, generation: u64 },
}

impl Kind for Key {
    type Value = u32;
    type Error = String;

    fn retention(&self) -> Retention {
        match self {
            Key::Kept(_) => Retention::Kept,
            Key::InFlight(_) => Retention::InFlight,
            Key::Sampled { .. } => Retention::Sampled,
        }
    }

    fn generation(&self) -> Option<(Self, u64)> {
        match self {
            Key::Sampled { id, generation } => Some((Key::Sampled { id: *id, generation: 0 }, *generation)),
            _ => None,
        }
    }

    fn weight(value: &u32) -> usize {
        *value as usize
    }
}

/// What the computations of a test did.
#[derive(Default)]
struct Probe {
    /// Computations that began.
    runs: AtomicUsize,
    /// Computations dropped, finished or not.
    dropped: AtomicUsize,
    open: AtomicBool,
    /// Polls of the computations at the gate.
    polled: AtomicUsize,
    waiting: Mutex<Vec<Waker>>,
}

struct CountDrop(Arc<Probe>);

impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl Probe {
    fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }

    fn dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }

    /// Lets the computations that wait at the gate go on.
    fn open(&self) {
        self.open.store(true, Ordering::SeqCst);
        let waiting = std::mem::take(&mut *self.waiting.lock().unwrap());
        waiting.into_iter().for_each(Waker::wake);
    }
}

/// Waits until the probe is opened.
async fn gate(probe: &Probe) {
    poll_fn(|cx| {
        probe.polled.fetch_add(1, Ordering::SeqCst);
        if probe.open.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            probe.waiting.lock().unwrap().push(cx.waker().clone());
            Poll::Pending
        }
    })
    .await;
}

/// A computation that waits until the probe is opened and then yields `value`.
fn work(probe: &Arc<Probe>, value: u32, keep: bool) -> impl Future<Output = Result<Computed<u32>, String>> + use<> {
    let probe = probe.clone();
    async move {
        probe.runs.fetch_add(1, Ordering::SeqCst);
        let _counted = CountDrop(probe.clone());
        gate(&probe).await;
        Ok(Computed::new(value, keep))
    }
}

type Call<'e> = Pin<Box<dyn Future<Output = Result<Got<u32>, String>> + 'e>>;

fn call<'e>(engine: &'e Engine<Key>, key: Key, probe: &Arc<Probe>, value: u32, keep: bool) -> Call<'e> {
    let probe = probe.clone();
    Box::pin(engine.get(key, move |_| work(&probe, value, keep)))
}

fn poll<T>(fut: &mut Pin<Box<dyn Future<Output = T> + '_>>) -> Poll<T> {
    fut.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

/// Polls each call once, in order.
fn poll_all(calls: &mut [Call<'_>]) -> Vec<Poll<Result<Got<u32>, String>>> {
    calls.iter_mut().map(poll).collect()
}

fn finished(poll: Poll<Result<Got<u32>, String>>) -> Result<Got<u32>, String> {
    match poll {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("not finished"),
    }
}

fn engine(entries: usize) -> Engine<Key> {
    Engine::new(Limits::entries(entries))
}

fn fresh(value: u32) -> Got<u32> {
    Got { value, reused: false }
}

#[test]
fn callers_that_ask_while_a_value_is_computed_share_one_computation() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    let mut calls: Vec<_> = (0..8).map(|_| call(&engine, Key::Kept(1), &probe, 7, true)).collect();
    assert!(poll_all(&mut calls).iter().all(|p| p.is_pending()));
    assert_eq!(probe.runs(), 1);

    probe.open();
    for polled in poll_all(&mut calls) {
        assert_eq!(finished(polled), Ok(fresh(7)));
    }
    assert_eq!(probe.runs(), 1);

    let mut later = call(&engine, Key::Kept(1), &probe, 9, true);
    assert_eq!(finished(poll(&mut later)), Ok(Got { value: 7, reused: true }));
    assert_eq!(probe.runs(), 1);
}

#[test]
fn the_computation_is_polled_when_it_woke_and_not_once_per_waiter() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    let mut calls: Vec<_> = (0..8).map(|_| call(&engine, Key::Kept(1), &probe, 7, true)).collect();
    for _ in 0..3 {
        poll_all(&mut calls);
    }
    assert_eq!(probe.polled.load(Ordering::SeqCst), 1);

    probe.open();
    for polled in poll_all(&mut calls) {
        assert_eq!(finished(polled), Ok(fresh(7)));
    }
    assert_eq!(probe.polled.load(Ordering::SeqCst), 2);
}

#[test]
fn a_waiter_does_not_wait_for_the_thread_that_is_polling_the_computation() {
    let engine = engine(8);
    let (inside, release) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    std::thread::scope(|scope| {
        let driver = scope.spawn(|| {
            let (inside, release) = (inside.clone(), release.clone());
            let mut call: Call<'_> = Box::pin(engine.get(Key::Kept(1), move |_| async move {
                inside.store(true, Ordering::SeqCst);
                // Blocks the thread that polls it, as work that is not asynchronous does.
                while !release.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
                Ok::<_, String>(Computed::new(7, true))
            }));
            finished(poll(&mut call))
        });
        while !inside.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let mut waiter: Call<'_> = Box::pin(engine.get(Key::Kept(1), |_| async { Ok(Computed::new(8, true)) }));
        assert!(poll(&mut waiter).is_pending());
        release.store(true, Ordering::SeqCst);
        assert_eq!(driver.join().unwrap(), Ok(fresh(7)));
        assert_eq!(finished(poll(&mut waiter)), Ok(fresh(7)));
    });
}

#[test]
fn dropping_the_caller_that_started_the_computation_does_not_restart_it() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    let mut calls: Vec<_> = (0..3).map(|_| call(&engine, Key::Kept(1), &probe, 7, true)).collect();
    poll_all(&mut calls);
    drop(calls.remove(0));
    assert_eq!(probe.dropped(), 0, "the others still want it");

    probe.open();
    for polled in poll_all(&mut calls) {
        assert_eq!(finished(polled), Ok(fresh(7)));
    }
    assert_eq!(probe.runs(), 1);
}

#[test]
fn a_waiter_that_never_drove_the_computation_finishes_it() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    let mut starter = call(&engine, Key::Kept(1), &probe, 7, true);
    let mut waiter = call(&engine, Key::Kept(1), &probe, 7, true);
    assert!(poll(&mut starter).is_pending());
    assert!(poll(&mut waiter).is_pending());
    drop(starter);
    probe.open();
    assert_eq!(finished(poll(&mut waiter)), Ok(fresh(7)));
    assert_eq!(probe.runs(), 1);
}

#[test]
fn the_computation_is_cancelled_when_the_last_waiter_is_dropped() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    let mut calls: Vec<_> = (0..2).map(|_| call(&engine, Key::Kept(1), &probe, 7, true)).collect();
    poll_all(&mut calls);
    drop(calls.remove(0));
    assert_eq!(probe.dropped(), 0);
    drop(calls.remove(0));
    assert_eq!(probe.dropped(), 1);

    // Nothing is left of it: the next caller starts afresh.
    let mut next = call(&engine, Key::Kept(1), &probe, 8, true);
    assert!(poll(&mut next).is_pending());
    assert_eq!(probe.runs(), 2);
    probe.open();
    assert_eq!(finished(poll(&mut next)), Ok(fresh(8)));
}

/// Holds the flights that linger until the test lets their grace periods pass.
#[derive(Default)]
struct Held(Mutex<Vec<Pin<Box<dyn Future<Output = ()> + Send>>>>);

impl Linger for Held {
    fn hold(&self, _: Duration, work: Pin<Box<dyn Future<Output = ()> + Send>>) {
        self.0.lock().unwrap().push(work);
    }
}

impl Held {
    /// Time goes on within the grace periods: what lingers is driven.
    fn drive(&self) {
        let mut held = self.0.lock().unwrap();
        held.retain_mut(|work| work.as_mut().poll(&mut Context::from_waker(Waker::noop())).is_pending());
    }

    /// The oldest `count` grace periods are over.
    fn pass(&self, count: usize) {
        let over: Vec<_> = self.0.lock().unwrap().drain(..count).collect();
        drop(over);
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

fn lingering(held: &Arc<Held>) -> Engine<Key> {
    engine(8).lingering(Duration::from_secs(1), held.clone())
}

#[test]
fn a_caller_who_asks_while_an_abandoned_flight_lingers_joins_it() {
    let held = Arc::new(Held::default());
    let engine = lingering(&held);
    let probe = Arc::new(Probe::default());
    let mut abandoned = call(&engine, Key::InFlight(1), &probe, 7, true);
    assert!(poll(&mut abandoned).is_pending());
    drop(abandoned);
    assert_eq!((probe.dropped(), held.len()), (0, 1));

    let mut next = call(&engine, Key::InFlight(1), &probe, 8, true);
    assert!(poll(&mut next).is_pending());
    held.pass(1);
    assert_eq!(probe.dropped(), 0, "the caller who joined still wants it");
    probe.open();
    assert_eq!(finished(poll(&mut next)), Ok(fresh(7)));
    assert_eq!(probe.runs(), 1);
}

#[test]
fn an_abandoned_flight_is_dropped_once_its_grace_period_is_over() {
    let held = Arc::new(Held::default());
    let engine = lingering(&held);
    let probe = Arc::new(Probe::default());
    let mut abandoned = call(&engine, Key::InFlight(1), &probe, 7, true);
    assert!(poll(&mut abandoned).is_pending());
    drop(abandoned);
    held.drive();
    assert_eq!(probe.dropped(), 0);
    held.pass(1);
    assert_eq!(probe.dropped(), 1);

    let mut next = call(&engine, Key::InFlight(1), &probe, 8, true);
    probe.open();
    assert_eq!(finished(poll(&mut next)), Ok(fresh(8)));
    assert_eq!(probe.runs(), 2);
}

#[test]
fn a_lingering_flight_goes_on_and_keeps_what_it_finds() {
    let held = Arc::new(Held::default());
    let engine = lingering(&held);
    let probe = Arc::new(Probe::default());
    let mut abandoned = call(&engine, Key::Kept(1), &probe, 7, true);
    assert!(poll(&mut abandoned).is_pending());
    drop(abandoned);
    probe.open();
    held.drive();
    assert_eq!(held.len(), 0, "finished within its grace period");

    let mut later = call(&engine, Key::Kept(1), &probe, 8, true);
    assert_eq!(finished(poll(&mut later)), Ok(Got { value: 7, reused: true }));
    assert_eq!(probe.runs(), 1);
}

#[test]
fn every_caller_who_abandons_a_flight_last_leaves_it_a_grace_period_of_its_own() {
    let held = Arc::new(Held::default());
    let engine = lingering(&held);
    let probe = Arc::new(Probe::default());
    for _ in 0..2 {
        let mut abandoned = call(&engine, Key::InFlight(1), &probe, 7, true);
        assert!(poll(&mut abandoned).is_pending());
        drop(abandoned);
    }
    assert_eq!(held.len(), 2);
    held.pass(1);
    assert_eq!(probe.dropped(), 0, "the second grace period is still under way");

    let mut next = call(&engine, Key::InFlight(1), &probe, 8, true);
    assert!(poll(&mut next).is_pending());
    held.pass(1);
    probe.open();
    assert_eq!(finished(poll(&mut next)), Ok(fresh(7)));
    assert_eq!(probe.runs(), 1);
}

#[test]
fn every_waiter_gets_the_error_and_it_is_not_kept() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    let failing = |probe: &Arc<Probe>| {
        let probe = probe.clone();
        let call: Call<'_> = Box::pin(engine.get(Key::Kept(1), move |_| async move {
            probe.runs.fetch_add(1, Ordering::SeqCst);
            gate(&probe).await;
            Err::<Computed<u32>, _>("boom".to_owned())
        }));
        call
    };
    let mut calls = vec![failing(&probe), failing(&probe)];
    poll_all(&mut calls);
    probe.open();
    for polled in poll_all(&mut calls) {
        assert_eq!(finished(polled), Err("boom".to_owned()));
    }
    assert_eq!(probe.runs(), 1);

    let mut again = failing(&probe);
    assert_eq!(finished(poll(&mut again)), Err("boom".to_owned()));
    assert_eq!(probe.runs(), 2);
}

#[test]
fn waiters_share_a_value_that_may_not_be_kept_and_later_callers_compute_again() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    let mut calls: Vec<_> = (0..2).map(|_| call(&engine, Key::Kept(1), &probe, 7, false)).collect();
    poll_all(&mut calls);
    probe.open();
    for polled in poll_all(&mut calls) {
        assert_eq!(finished(polled), Ok(fresh(7)));
    }
    assert_eq!(probe.runs(), 1);

    let mut later = call(&engine, Key::Kept(1), &probe, 8, false);
    assert_eq!(finished(poll(&mut later)), Ok(fresh(8)));
    assert_eq!(probe.runs(), 2);
}

#[test]
fn a_value_that_is_only_shared_in_flight_is_dropped_when_the_flight_ends() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    probe.open();
    let mut first = call(&engine, Key::InFlight(1), &probe, 7, true);
    assert_eq!(finished(poll(&mut first)), Ok(fresh(7)));
    let mut second = call(&engine, Key::InFlight(1), &probe, 8, true);
    assert_eq!(finished(poll(&mut second)), Ok(fresh(8)));
    assert_eq!(probe.runs(), 2);
}

#[test]
fn a_restored_value_is_reported_as_reused_to_every_waiter() {
    let engine = engine(8);
    let mut calls: Vec<Call<'_>> = (0..2)
        .map(|_| -> Call<'_> { Box::pin(engine.get(Key::Kept(1), |_| async { Ok(Computed::restored(7)) })) })
        .collect();
    for polled in poll_all(&mut calls) {
        assert_eq!(finished(polled), Ok(Got { value: 7, reused: true }));
    }
}

#[test]
fn a_sampled_value_is_taken_once_per_generation() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    probe.open();
    let sample = |generation| Key::Sampled { id: 1, generation };
    let ask = |generation, value| {
        let mut call = call(&engine, sample(generation), &probe, value, true);
        finished(poll(&mut call))
    };
    assert_eq!(ask(1, 10), Ok(fresh(10)));
    assert_eq!(
        ask(1, 11),
        Ok(Got {
            value: 10,
            reused: true
        })
    );
    assert_eq!(probe.runs(), 1);

    // A newer generation is observed afresh, and the older one is gone.
    assert_eq!(ask(2, 20), Ok(fresh(20)));
    assert_eq!(
        ask(2, 21),
        Ok(Got {
            value: 20,
            reused: true
        })
    );
    assert_eq!(probe.runs(), 2);

    // A caller still on the older generation gets an answer that is not kept.
    assert_eq!(ask(1, 12), Ok(fresh(12)));
    assert_eq!(ask(1, 13), Ok(fresh(13)));
    assert_eq!(
        ask(2, 22),
        Ok(Got {
            value: 20,
            reused: true
        })
    );
}

#[test]
fn generations_of_one_observation_do_not_drop_those_of_another() {
    let engine = engine(8);
    let probe = Arc::new(Probe::default());
    probe.open();
    let ask = |id, generation, value| {
        let mut call = call(&engine, Key::Sampled { id, generation }, &probe, value, true);
        finished(poll(&mut call))
    };
    ask(1, 1, 10).unwrap();
    ask(2, 1, 20).unwrap();
    ask(1, 2, 11).unwrap();
    assert_eq!(
        ask(2, 1, 21),
        Ok(Got {
            value: 20,
            reused: true
        })
    );
}

#[test]
fn the_least_recently_used_value_goes_beyond_the_entry_limit() {
    let engine = engine(2);
    let probe = Arc::new(Probe::default());
    probe.open();
    let ask = |n: u8| {
        let mut call = call(&engine, Key::Kept(n), &probe, u32::from(n), true);
        finished(poll(&mut call)).unwrap().reused
    };
    ask(1);
    ask(2);
    assert!(ask(1), "used again, so 2 is the least recently used");
    ask(3);
    assert!(ask(1));
    assert!(!ask(2));
}

#[test]
fn values_beyond_the_weight_limit_go_oldest_first() {
    let engine = Engine::<Key>::new(Limits::weighing(100, 10));
    let probe = Arc::new(Probe::default());
    probe.open();
    let ask = |n: u8, weight: u32| {
        let mut call = call(&engine, Key::Kept(n), &probe, weight, true);
        finished(poll(&mut call)).unwrap().reused
    };
    ask(1, 6);
    ask(2, 3);
    ask(3, 6);
    assert!(ask(2, 3), "the weight fits again without it");
    assert!(ask(3, 6));
    assert!(!ask(1, 6), "evicted to make room for 3");
}

#[test]
fn a_flight_is_never_evicted_and_shares_even_where_nothing_is_kept() {
    let engine = engine(0);
    let probe = Arc::new(Probe::default());
    let mut calls: Vec<_> = (0..2).map(|_| call(&engine, Key::Kept(1), &probe, 7, true)).collect();
    poll_all(&mut calls);
    // Other values come and go while it is computed.
    probe.open();
    let mut other = call(&engine, Key::Kept(2), &probe, 1, true);
    finished(poll(&mut other)).unwrap();
    for polled in poll_all(&mut calls) {
        assert_eq!(finished(polled), Ok(fresh(7)));
    }
    assert_eq!(probe.runs(), 2, "one for each key");

    let mut later = call(&engine, Key::Kept(1), &probe, 7, true);
    finished(poll(&mut later)).unwrap();
    assert_eq!(probe.runs(), 3);
}

#[test]
fn clearing_drops_the_kept_values_and_leaves_the_flights() {
    let engine = engine(8);
    let (open, closed) = (Arc::new(Probe::default()), Arc::new(Probe::default()));
    open.open();
    let mut kept = call(&engine, Key::Kept(1), &open, 7, true);
    finished(poll(&mut kept)).unwrap();
    let mut flying = call(&engine, Key::Kept(2), &closed, 8, true);
    assert!(poll(&mut flying).is_pending());

    engine.clear();
    assert_eq!(engine.peek(&Key::Kept(1)), None);
    closed.open();
    assert_eq!(finished(poll(&mut flying)), Ok(fresh(8)));
    assert_eq!(engine.peek(&Key::Kept(2)), Some(8));
}

type Report = (Option<(u32, u32)>, Option<String>);

#[derive(Default)]
struct Shown {
    started: AtomicUsize,
    ended: AtomicUsize,
    reports: Mutex<Vec<Report>>,
}

struct Shows(Arc<Shown>);

impl Task for Shows {
    fn report(&self, done_of_total: Option<(u32, u32)>, detail: Option<String>) {
        self.0.reports.lock().unwrap().push((done_of_total, detail));
    }
}

impl Drop for Shows {
    fn drop(&mut self) {
        self.0.ended.fetch_add(1, Ordering::SeqCst);
    }
}

fn shown_engine(shown: &Arc<Shown>) -> Engine<Key> {
    let shown = shown.clone();
    engine(8).observing(move |key| {
        matches!(key, Key::Kept(_)).then(|| {
            shown.started.fetch_add(1, Ordering::SeqCst);
            Arc::new(Shows(shown.clone())) as Arc<dyn Task>
        })
    })
}

#[test]
fn the_flight_has_one_task_however_many_callers_and_it_ends_with_the_flight() {
    let shown = Arc::new(Shown::default());
    let engine = shown_engine(&shown);
    let probe = Arc::new(Probe::default());
    let mut calls: Vec<Call<'_>> = (0..3)
        .map(|_| -> Call<'_> {
            let probe = probe.clone();
            Box::pin(engine.get(Key::Kept(1), move |progress| async move {
                progress.report(Some((1, 2)), Some("half".into()));
                work(&probe, 7, true).await
            }))
        })
        .collect();
    poll_all(&mut calls);
    assert_eq!(shown.started.load(Ordering::SeqCst), 1);
    assert_eq!(shown.ended.load(Ordering::SeqCst), 0);
    assert_eq!(
        *shown.reports.lock().unwrap(),
        [(Some((1, 2)), Some("half".to_owned()))]
    );

    probe.open();
    poll_all(&mut calls);
    assert_eq!(shown.started.load(Ordering::SeqCst), 1);
    assert_eq!(shown.ended.load(Ordering::SeqCst), 1);
}

#[test]
fn the_task_ends_when_the_flight_is_cancelled() {
    let shown = Arc::new(Shown::default());
    let engine = shown_engine(&shown);
    let probe = Arc::new(Probe::default());
    let mut only = call(&engine, Key::Kept(1), &probe, 7, true);
    assert!(poll(&mut only).is_pending());
    assert_eq!(shown.ended.load(Ordering::SeqCst), 0);
    drop(only);
    assert_eq!(shown.ended.load(Ordering::SeqCst), 1);
}

#[test]
#[should_panic(expected = "waits for the flight")]
fn a_computation_that_waits_for_its_own_key_panics_instead_of_hanging() {
    let engine = Arc::new(engine(8));
    let inner = engine.clone();
    let mut call: Call<'_> = Box::pin(engine.get(Key::InFlight(1), move |_| async move {
        let got = inner
            .get(Key::InFlight(1), |_| async { Ok::<_, String>(Computed::new(1, false)) })
            .await?;
        Ok(Computed::new(got.value, false))
    }));
    let _ = poll(&mut call);
}
