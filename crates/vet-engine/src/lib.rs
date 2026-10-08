//! Derived work, computed once per key however many callers ask. Runtime-free:
//! waiters are woken through their wakers.
//!
//! A computation under way is a *flight*: the future itself, owned by the engine
//! and driven by whichever waiter is polled. The caller that started it has no
//! special part, so dropping it changes nothing for the others, and nothing
//! starts over. Every waiter of a flight gets its result, whether it can be kept
//! or not, a value or an error. Only a caller that arrives after the flight ended
//! computes again, and only if the result was not kept.
//!
//! A flight lives while at least one waiter wants it. When the last one is
//! dropped, the computation is dropped with it, and whatever it was waiting for
//! sees the cancellation. A flight therefore owns what it uses (`'static`).
//!
//! An engine may let flights [linger](Engine::lingering): when the last waiter
//! is dropped, the computation goes on for a grace period, and a caller that asks
//! for the key meanwhile joins it. A caller dropped because its request was
//! superseded is followed by the request that superseded it, which then finds the
//! work under way instead of starting it again. Once the grace period is over
//! with nobody waiting, the computation is dropped as above.
//!
//! What stays after a flight ended is the key's [`Retention`]:
//!
//! - [`Retention::Kept`]: the key names every input. The value stays, least
//!   recently used first to go beyond the [`Limits`].
//! - [`Retention::InFlight`]: shared while it is computed, dropped when it ends.
//! - [`Retention::Sampled`]: an observation of the environment, taken once per
//!   generation. The key carries its generation; a newer one drops the older.
//!
//! Keys form a DAG: a computation may wait on flights of other keys, never on its
//! own. That is a bug, and it panics instead of hanging.

use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::hash::Hash;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

/// What stays of a value after the flight that computed it ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Retention {
    Kept,
    Sampled,
    InFlight,
}

/// A key of derived work, and what its value is.
pub trait Kind: Eq + Hash + Clone + Send + Sync + 'static {
    type Value: Clone + Send + 'static;
    type Error: Clone + Send + 'static;

    fn retention(&self) -> Retention;

    /// For a [`Retention::Sampled`] key: the key of the same observation with a
    /// fixed generation, and the generation this key belongs to. A key of a newer
    /// generation than any seen drops the values of the older ones.
    fn generation(&self) -> Option<(Self, u64)> {
        None
    }

    /// What a kept value counts against [`Limits::weight`].
    fn weight(_value: &Self::Value) -> usize {
        1
    }
}

/// How much an engine keeps: at most `entries` values weighing at most `weight`
/// together. Flights are not counted; each has a waiter or lingers.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub entries: usize,
    pub weight: usize,
}

impl Limits {
    pub fn entries(entries: usize) -> Self {
        Self {
            entries,
            weight: usize::MAX,
        }
    }

    pub fn weighing(entries: usize, weight: usize) -> Self {
        Self { entries, weight }
    }
}

/// A computed value and what may become of it.
pub struct Computed<V> {
    pub value: V,
    /// Whether the value may be kept for later callers. A value that depends on
    /// something the key does not name (a server at work on its own) may not.
    pub keep: bool,
    /// The value was supplied from elsewhere, not worked out: callers are told.
    pub reused: bool,
}

impl<V> Computed<V> {
    pub fn new(value: V, keep: bool) -> Self {
        Self {
            value,
            keep,
            reused: false,
        }
    }

    /// A value read from a lower tier: kept like one worked out, and reported as reused.
    pub fn restored(value: V) -> Self {
        Self {
            value,
            keep: true,
            reused: true,
        }
    }
}

impl<V: Clone> Clone for Computed<V> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            keep: self.keep,
            reused: self.reused,
        }
    }
}

/// What [`Engine::get`] yields.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Got<V> {
    pub value: V,
    /// The value was not worked out by the flight that gave it: it was kept, or
    /// read from a lower tier.
    pub reused: bool,
}

/// A task shown to the person while a flight runs. It ends when its last handle
/// is dropped, which is when the flight ends or is cancelled.
pub trait Task: Send + Sync {
    /// `done` of `total` units are complete, and the step is `detail`.
    fn report(&self, done_of_total: Option<(u32, u32)>, detail: Option<String>);
}

/// Where a computation reports its progress: to the task of its flight, if the
/// engine has one for the key.
#[derive(Clone, Default)]
pub struct Progress(Option<Arc<dyn Task>>);

impl Progress {
    pub fn report(&self, done_of_total: Option<(u32, u32)>, detail: Option<String>) {
        if let Some(task) = &self.0 {
            task.report(done_of_total, detail);
        }
    }
}

/// Starts the task a flight of a key reports as; none: it has none.
type Observe<K> = Box<dyn Fn(&K) -> Option<Arc<dyn Task>> + Send + Sync>;

/// What drives a lingering flight, since the engine has no runtime of its own.
pub trait Linger: Send + Sync {
    /// Polls `work` on its own until it ends or `grace` has passed, and then drops it.
    fn hold(&self, grace: Duration, work: Pin<Box<dyn Future<Output = ()> + Send>>);
}

/// How long flights linger, and what drives them meanwhile.
#[derive(Clone)]
struct Lingering {
    grace: Duration,
    linger: Arc<dyn Linger>,
}

type Outcome<K> = Result<Computed<<K as Kind>::Value>, <K as Kind>::Error>;
type Work<K> = Pin<Box<dyn Future<Output = Outcome<K>> + Send>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The wakers of the waiters of one flight. The computation is polled with this
/// one waker, so whoever polled it last, even if gone, does not decide who is
/// woken. It is polled only after it woke, however many waiters are polled.
struct Wakers {
    waiting: Mutex<Vec<Waker>>,
    /// The computation woke since it was polled last, or was never polled.
    woken: AtomicBool,
}

impl Wakers {
    fn new() -> Self {
        Self {
            waiting: Mutex::default(),
            woken: AtomicBool::new(true),
        }
    }

    fn register(&self, waker: &Waker) {
        let mut waiting = lock(&self.waiting);
        if !waiting.iter().any(|w| w.will_wake(waker)) {
            waiting.push(waker.clone());
        }
    }

    /// Whether the computation is to be polled now; it is not until it wakes again.
    fn take_woken(&self) -> bool {
        self.woken.swap(false, Ordering::SeqCst)
    }

    /// Whether the computation woke and nobody polled it since.
    fn is_woken(&self) -> bool {
        self.woken.load(Ordering::SeqCst)
    }

    fn wake_all(&self) {
        self.woken.store(true, Ordering::SeqCst);
        let waiting = std::mem::take(&mut *lock(&self.waiting));
        waiting.into_iter().for_each(Waker::wake);
    }
}

impl Wake for Wakers {
    fn wake(self: Arc<Self>) {
        self.wake_all();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wake_all();
    }
}

enum Phase<K: Kind> {
    Running {
        work: Work<K>,
        /// Ends with the flight.
        _task: Option<Arc<dyn Task>>,
    },
    Done(Outcome<K>),
    /// Cancelled: no waiter was left.
    Dropped,
}

struct Flight<K: Kind> {
    /// Callers that still want the result; changed under the engine's lock.
    waiting: AtomicUsize,
    /// Grace periods under way since the last caller left; changed under the engine's lock.
    lingering: AtomicUsize,
    /// The thread polling the computation, 0 if none.
    driver: AtomicU64,
    wakers: Arc<Wakers>,
    phase: Mutex<Phase<K>>,
}

enum Slot<K: Kind> {
    Flying(Arc<Flight<K>>),
    Kept { value: K::Value, weight: usize, used: u64 },
}

struct Map<K: Kind> {
    limits: Limits,
    tick: u64,
    slots: HashMap<K, Slot<K>>,
    kept: usize,
    weight: usize,
    /// The newest generation seen of each sampled observation.
    latest: HashMap<K, u64>,
}

impl<K: Kind> Map<K> {
    fn drop_kept(&mut self, key: &K) {
        if let Some(Slot::Kept { weight, .. }) = self.slots.remove(key) {
            self.kept -= 1;
            self.weight -= weight;
        }
    }

    /// Notes the generation of a sampled key; a newer one drops the older values.
    fn observe(&mut self, subject: K, generation: u64) {
        let latest = self.latest.entry(subject.clone()).or_insert(generation);
        if generation <= *latest {
            return;
        }
        *latest = generation;
        let older: Vec<K> = self
            .slots
            .iter()
            .filter(|(key, slot)| {
                matches!(slot, Slot::Kept { .. })
                    && key
                        .generation()
                        .is_some_and(|(of, at)| of == subject && at < generation)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in older {
            self.drop_kept(&key);
        }
    }

    fn insert(&mut self, key: K, value: K::Value) {
        self.tick += 1;
        let weight = K::weight(&value);
        let slot = Slot::Kept {
            value,
            weight,
            used: self.tick,
        };
        self.slots.insert(key, slot);
        self.kept += 1;
        self.weight += weight;
        self.evict();
    }

    /// The flight of `key` ended with `outcome`: it is no longer joined, and its
    /// value stays if the key's retention and the outcome allow.
    fn settle(&mut self, key: &K, flight: &Arc<Flight<K>>, outcome: &Outcome<K>) {
        if !self.is_flying(key, flight) {
            return;
        }
        self.slots.remove(key);
        let Ok(computed) = outcome else {
            return;
        };
        let current = match key.retention() {
            Retention::InFlight => false,
            Retention::Kept => true,
            Retention::Sampled => match key.generation() {
                Some((subject, generation)) => self.latest.get(&subject) == Some(&generation),
                None => true,
            },
        };
        if computed.keep && current {
            self.insert(key.clone(), computed.value.clone());
        }
    }

    /// Whether `flight` is the one callers of `key` join.
    fn is_flying(&self, key: &K, flight: &Arc<Flight<K>>) -> bool {
        matches!(self.slots.get(key), Some(Slot::Flying(f)) if Arc::ptr_eq(f, flight))
    }

    fn evict(&mut self) {
        let limits = self.limits;
        while self.kept > limits.entries || self.weight > limits.weight {
            let oldest = self
                .slots
                .iter()
                .filter_map(|(key, slot)| match slot {
                    Slot::Kept { used, .. } => Some((key, *used)),
                    Slot::Flying(_) => None,
                })
                .min_by_key(|(_, used)| *used)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(oldest) => self.drop_kept(&oldest),
                None => break,
            }
        }
    }
}

enum Entered<K: Kind> {
    Hit(K::Value),
    Flight(Arc<Flight<K>>),
}

/// Values by key. See the module's documentation.
pub struct Engine<K: Kind> {
    observe: Option<Observe<K>>,
    lingering: Option<Lingering>,
    /// Shared with the waiters, which outlive the caller while they linger.
    map: Arc<Mutex<Map<K>>>,
}

fn thread_token() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    thread_local! {
        static TOKEN: u64 = NEXT.fetch_add(1, Ordering::Relaxed);
    }
    TOKEN.with(|token| *token)
}

impl<K: Kind> Engine<K> {
    pub fn new(limits: Limits) -> Self {
        Self {
            observe: None,
            lingering: None,
            map: Arc::new(Mutex::new(Map {
                limits,
                tick: 0,
                slots: HashMap::new(),
                kept: 0,
                weight: 0,
                latest: HashMap::new(),
            })),
        }
    }

    /// Starts a task for every flight whose key `observe` gives one for, and ends
    /// it with the flight. The computation reports to it through its [`Progress`].
    pub fn observing(mut self, observe: impl Fn(&K) -> Option<Arc<dyn Task>> + Send + Sync + 'static) -> Self {
        self.observe = Some(Box::new(observe));
        self
    }

    /// Lets a flight whose last waiter was dropped go on for `grace`, driven by
    /// `linger`, so that a caller who asks for its key meanwhile joins it.
    pub fn lingering(mut self, grace: Duration, linger: Arc<dyn Linger>) -> Self {
        self.lingering = Some(Lingering { grace, linger });
        self
    }

    fn map(&self) -> MutexGuard<'_, Map<K>> {
        lock(&self.map)
    }

    /// The value for `key`: kept, joined while it is computed, or computed by
    /// `compute`. `compute` only builds the computation, and runs under the
    /// engine's lock: it must not call the engine. The computation may.
    pub async fn get<F>(&self, key: K, compute: impl FnOnce(Progress) -> F) -> Result<Got<K::Value>, K::Error>
    where
        F: Future<Output = Result<Computed<K::Value>, K::Error>> + Send + 'static,
    {
        let flight = match self.enter(&key, compute) {
            Entered::Hit(value) => return Ok(Got { value, reused: true }),
            Entered::Flight(flight) => flight,
        };
        let waiter = Waiter {
            map: self.map.clone(),
            key,
            flight,
            role: Role::Caller(self.lingering.clone()),
        };
        let Computed { value, reused, .. } = poll_fn(|cx| waiter.poll(cx)).await?;
        Ok(Got { value, reused })
    }

    fn enter<F>(&self, key: &K, compute: impl FnOnce(Progress) -> F) -> Entered<K>
    where
        F: Future<Output = Result<Computed<K::Value>, K::Error>> + Send + 'static,
    {
        let mut map = self.map();
        map.tick += 1;
        let tick = map.tick;
        if key.retention() == Retention::Sampled
            && let Some((subject, generation)) = key.generation()
        {
            map.observe(subject, generation);
        }
        match map.slots.get_mut(key) {
            Some(Slot::Kept { value, used, .. }) => {
                *used = tick;
                return Entered::Hit(value.clone());
            }
            Some(Slot::Flying(flight)) => {
                flight.waiting.fetch_add(1, Ordering::SeqCst);
                return Entered::Flight(flight.clone());
            }
            None => {}
        }
        let task = self.observe.as_ref().and_then(|observe| observe(key));
        let work: Work<K> = Box::pin(compute(Progress(task.clone())));
        let flight = Arc::new(Flight {
            waiting: AtomicUsize::new(1),
            lingering: AtomicUsize::new(0),
            driver: AtomicU64::new(0),
            wakers: Arc::new(Wakers::new()),
            phase: Mutex::new(Phase::Running { work, _task: task }),
        });
        map.slots.insert(key.clone(), Slot::Flying(flight.clone()));
        Entered::Flight(flight)
    }

    /// The value under `key` if one is kept; nothing is computed or waited for.
    pub fn peek(&self, key: &K) -> Option<K::Value> {
        let mut map = self.map();
        map.tick += 1;
        let tick = map.tick;
        match map.slots.get_mut(key) {
            Some(Slot::Kept { value, used, .. }) => {
                *used = tick;
                Some(value.clone())
            }
            _ => None,
        }
    }

    /// Keeps `value` under `key` unless the key has a value or is being computed.
    pub fn keep(&self, key: &K, value: K::Value) {
        let mut map = self.map();
        if key.retention() == Retention::InFlight || map.slots.contains_key(key) {
            return;
        }
        map.insert(key.clone(), value);
    }

    /// Drops the kept values. Flights stay, with their waiters.
    pub fn clear(&self) {
        let mut map = self.map();
        map.slots.retain(|_, slot| matches!(slot, Slot::Flying(_)));
        map.kept = 0;
        map.weight = 0;
    }
}

/// Whose interest in a flight a waiter is.
enum Role {
    /// A caller's, and how long the flight lingers once the last caller left.
    Caller(Option<Lingering>),
    /// A grace period's, after the last caller left.
    Lingerer,
}

/// An interest in a flight. Dropping it withdraws the interest.
struct Waiter<K: Kind> {
    map: Arc<Mutex<Map<K>>>,
    key: K,
    flight: Arc<Flight<K>>,
    role: Role,
}

impl<K: Kind> Waiter<K> {
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Outcome<K>> {
        let flight = &self.flight;
        flight.wakers.register(cx.waker());
        let token = thread_token();
        assert_ne!(
            flight.driver.load(Ordering::SeqCst),
            token,
            "a computation waits for the flight it is part of"
        );
        // A waiter never waits for a thread that is polling the computation, which
        // may take long to return: it is woken when that one is through.
        let mut phase = match flight.phase.try_lock() {
            Ok(phase) => phase,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Poll::Pending,
        };
        let polled = match &mut *phase {
            Phase::Done(outcome) => return Poll::Ready(outcome.clone()),
            Phase::Dropped => unreachable!("a flight is kept while it has a waiter"),
            Phase::Running { work, .. } => {
                if !flight.wakers.take_woken() {
                    return Poll::Pending;
                }
                let waker = Waker::from(flight.wakers.clone());
                flight.driver.store(token, Ordering::SeqCst);
                let polled = work.as_mut().poll(&mut Context::from_waker(&waker));
                flight.driver.store(0, Ordering::SeqCst);
                polled
            }
        };
        let Poll::Ready(outcome) = polled else {
            return Poll::Pending;
        };
        let finished = std::mem::replace(&mut *phase, Phase::Done(outcome.clone()));
        drop(phase);
        drop(finished);
        lock(&self.map).settle(&self.key, flight, &outcome);
        flight.wakers.wake_all();
        Poll::Ready(outcome)
    }
}

impl<K: Kind> Drop for Waiter<K> {
    /// The interest is withdrawn. The last caller's leaves the computation to a
    /// grace period if the engine lingers; without any interest left, it is dropped.
    fn drop(&mut self) {
        let flight = &self.flight;
        let mut lingers = None;
        let cancel = {
            let mut map = lock(&self.map);
            let flying = map.is_flying(&self.key, flight);
            let (waiting, lingering) = match &self.role {
                Role::Caller(lingering) => {
                    let waiting = flight.waiting.fetch_sub(1, Ordering::SeqCst) - 1;
                    lingers = lingering.clone().filter(|_| waiting == 0 && flying);
                    if lingers.is_some() {
                        flight.lingering.fetch_add(1, Ordering::SeqCst);
                    }
                    (waiting, flight.lingering.load(Ordering::SeqCst))
                }
                Role::Lingerer => (
                    flight.waiting.load(Ordering::SeqCst),
                    flight.lingering.fetch_sub(1, Ordering::SeqCst) - 1,
                ),
            };
            let cancel = waiting == 0 && lingering == 0;
            if cancel && flying {
                map.slots.remove(&self.key);
            }
            cancel
        };
        if let Some(Lingering { grace, linger }) = lingers {
            let lingerer = Waiter {
                map: self.map.clone(),
                key: self.key.clone(),
                flight: flight.clone(),
                role: Role::Lingerer,
            };
            linger.hold(
                grace,
                Box::pin(async move {
                    let _ = poll_fn(|cx| lingerer.poll(cx)).await;
                }),
            );
        } else if cancel {
            let cancelled = {
                let mut phase = lock(&flight.phase);
                match &*phase {
                    Phase::Running { .. } => Some(std::mem::replace(&mut *phase, Phase::Dropped)),
                    _ => None,
                }
            };
            // Dropped outside the locks: it may run code of its own.
            drop(cancelled);
        } else if flight.wakers.is_woken() {
            // The waiter that was to poll the computation may be this one.
            flight.wakers.wake_all();
        }
    }
}

#[cfg(test)]
mod tests;
