//! The open projects of a process: one per repository, the connections
//! subscribed to each, and when a project without connections closes.

use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::mutex::lock;
use crate::ports::{Clock, Millis};

/// A connection that wants a project's events.
pub trait Subscriber: Clone + Send + Sync + 'static {
    type Event: Clone;

    /// Sends are dropped once the connection is gone.
    fn send(&self, event: Self::Event);

    /// Whether the connection is gone.
    fn is_closed(&self) -> bool;

    /// Whether both belong to the same connection.
    fn same_connection(&self, other: &Self) -> bool;
}

/// The connections subscribed to one project.
pub struct Subscribers<S>(Arc<Mutex<Vec<S>>>);

impl<S> Clone for Subscribers<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// No connection yet: for events that belong to the process, not to a session.
impl<S> Default for Subscribers<S> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

impl<S: Subscriber> Subscribers<S> {
    fn new(first: S) -> Self {
        Self(Arc::new(Mutex::new(vec![first])))
    }

    /// The connections still open.
    fn live(&self) -> MutexGuard<'_, Vec<S>> {
        let mut subscribers = lock(&self.0);
        subscribers.retain(|s| !s.is_closed());
        subscribers
    }

    pub fn subscribe(&self, subscriber: S) {
        let mut subscribers = self.live();
        if !subscribers.iter().any(|s| s.same_connection(&subscriber)) {
            subscribers.push(subscriber);
        }
    }

    /// Whether no connection is left after removing `subscriber`.
    fn unsubscribe(&self, subscriber: &S) -> bool {
        let mut subscribers = self.live();
        subscribers.retain(|s| !s.same_connection(subscriber));
        subscribers.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.live().is_empty()
    }

    /// Tells every connection, the one whose request made the change included: a window's stores
    /// that did not ask learn of it only this way.
    pub fn broadcast(&self, event: S::Event) {
        for subscriber in self.live().iter() {
            subscriber.send(event.clone());
        }
    }
}

/// What an open project needs to do when it closes.
pub trait Resident: Send + Sync + 'static {
    /// Stops the project's background work. The review itself stays in the
    /// store for the next open of the repository to resume.
    fn shut(&self) -> impl Future<Output = ()> + Send;
}

/// An open project: the registry's bookkeeping around the adapters `T` wires.
pub struct Entry<S, T> {
    pub id: u32,
    pub repo: PathBuf,
    pub subscribers: Subscribers<S>,
    resident: T,
}

impl<S, T> Deref for Entry<S, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.resident
    }
}

#[derive(Clone, Debug)]
pub struct NoSuchProject;

impl fmt::Display for NoSuchProject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no such project")
    }
}

/// Held while a repository's project is built or shut, so that one repository is never open
/// twice and never opened while it closes. Other repositories do not wait for it.
type Gate = Arc<tokio::sync::Mutex<()>>;

struct Shared<S, T> {
    /// The open projects: only ever locked to look at or change the list, never across an await.
    entries: Mutex<Vec<Arc<Entry<S, T>>>>,
    /// The gates of the repositories in use. A gate nobody else holds is dropped on the next look.
    gates: Mutex<HashMap<PathBuf, Gate>>,
    next_id: AtomicU32,
}

pub struct Registry<S, T>(Arc<Shared<S, T>>);

impl<S, T> Clone for Registry<S, T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, T> Default for Registry<S, T> {
    fn default() -> Self {
        Self(Arc::new(Shared {
            entries: Mutex::default(),
            gates: Mutex::default(),
            next_id: AtomicU32::new(1),
        }))
    }
}

impl<S: Subscriber, T: Resident> Registry<S, T> {
    fn gate(&self, repo: &Path) -> Gate {
        let mut gates = lock(&self.0.gates);
        gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        gates.entry(repo.to_owned()).or_default().clone()
    }

    fn entry_of(&self, matches: impl Fn(&Entry<S, T>) -> bool) -> Option<Arc<Entry<S, T>>> {
        lock(&self.0.entries).iter().find(|e| matches(e)).cloned()
    }

    /// The project of `repo` with `subscriber` subscribed; `create` builds it
    /// from its new id and subscribers when the repository is not open yet.
    /// Whether it did is the second value. Opening the same repository waits for
    /// the one under way, so it is never opened twice; other projects go on meanwhile.
    pub async fn open<E, F>(
        &self,
        repo: &Path,
        subscriber: S,
        create: impl FnOnce(u32, Subscribers<S>) -> F,
    ) -> Result<(Arc<Entry<S, T>>, bool), E>
    where
        F: Future<Output = Result<T, E>>,
    {
        let gate = self.gate(repo);
        let _building = gate.lock().await;
        if let Some(entry) = self.entry_of(|e| e.repo == repo) {
            entry.subscribers.subscribe(subscriber);
            return Ok((entry, false));
        }
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let subscribers = Subscribers::new(subscriber);
        let resident = create(id, subscribers.clone()).await?;
        let entry = Arc::new(Entry {
            id,
            repo: repo.to_owned(),
            subscribers,
            resident,
        });
        lock(&self.0.entries).push(entry.clone());
        Ok((entry, true))
    }

    pub async fn find(&self, id: u32) -> Result<Arc<Entry<S, T>>, NoSuchProject> {
        self.entry_of(|e| e.id == id).ok_or(NoSuchProject)
    }

    pub async fn all(&self) -> Vec<Arc<Entry<S, T>>> {
        lock(&self.0.entries).clone()
    }

    /// Takes `entry` out and shuts it, unless a connection subscribed meanwhile or it is gone.
    /// Whether it did. Subscribing goes through the same gate, so none can attach while it shuts.
    async fn retire(&self, entry: &Entry<S, T>) -> bool {
        let gate = self.gate(&entry.repo);
        let _closing = gate.lock().await;
        if !entry.subscribers.is_empty() {
            return false;
        }
        let removed = {
            let mut entries = lock(&self.0.entries);
            let at = entries.iter().position(|e| e.id == entry.id);
            at.map(|at| entries.remove(at))
        };
        let Some(entry) = removed else {
            return false;
        };
        entry.shut().await;
        true
    }

    /// Takes the project of `repo` out whatever connections are subscribed to it, shuts it and tells
    /// them `farewell`, then runs `work` with the repository's gate held: opening the repository
    /// again waits until `work` is done, and other repositories do not. Answers what `work` does.
    pub async fn evict<W: Future>(&self, repo: &Path, farewell: S::Event, work: W) -> W::Output {
        let gate = self.gate(repo);
        let _evicting = gate.lock().await;
        let removed = {
            let mut entries = lock(&self.0.entries);
            let at = entries.iter().position(|e| e.repo == repo);
            at.map(|at| entries.remove(at))
        };
        if let Some(entry) = removed {
            entry.shut().await;
            entry.subscribers.broadcast(farewell);
        }
        work.await
    }

    /// Unsubscribes the connection; the last one to leave closes the project.
    pub async fn close(&self, id: u32, subscriber: &S) -> Result<(), NoSuchProject> {
        let entry = self.find(id).await?;
        if entry.subscribers.unsubscribe(subscriber) {
            self.retire(&entry).await;
        }
        Ok(())
    }

    /// Returns once the project is gone: closed by its last connection leaving,
    /// or by none being subscribed for `grace`.
    pub async fn reap(&self, id: u32, grace: Duration, clock: &impl Clock) {
        let grace = grace.as_millis() as u64;
        let tick = (grace / 4).max(20);
        let mut orphaned_since: Option<Millis> = None;
        loop {
            clock.sleep_until(Millis(clock.now().0 + tick)).await;
            let Some(entry) = self.entry_of(|e| e.id == id) else {
                return;
            };
            if !entry.subscribers.is_empty() {
                orphaned_since = None;
                continue;
            }
            let now = clock.now();
            let since = *orphaned_since.get_or_insert(now);
            if now.0 - since.0 >= grace {
                if self.retire(&entry).await {
                    return;
                }
                // A connection attached after all, or the project closed meanwhile.
                orphaned_since = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64};

    use tokio::sync::Notify;

    use super::*;

    #[derive(Clone)]
    struct Connection {
        name: &'static str,
        closed: Arc<AtomicBool>,
        received: Arc<Mutex<Vec<String>>>,
    }

    impl Connection {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                closed: Arc::default(),
                received: Arc::default(),
            }
        }

        fn hang_up(&self) {
            self.closed.store(true, Ordering::SeqCst);
        }

        fn received(&self) -> Vec<String> {
            self.received.lock().unwrap().clone()
        }
    }

    impl Subscriber for Connection {
        type Event = String;

        fn send(&self, event: String) {
            self.received.lock().unwrap().push(event);
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }

        fn same_connection(&self, other: &Self) -> bool {
            self.name == other.name
        }
    }

    #[derive(Default)]
    struct Adapters {
        shut: AtomicBool,
        /// Shutting waits for `release` when set.
        slow_shut: AtomicBool,
        release: Notify,
    }

    impl Resident for Arc<Adapters> {
        async fn shut(&self) {
            if self.slow_shut.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            self.shut.store(true, Ordering::SeqCst);
        }
    }

    /// Long enough to tell that something waits, short enough for a test.
    const WAITING: Duration = Duration::from_millis(50);

    /// Past this, a request that should not have waited for another project did.
    const BLOCKED: Duration = Duration::from_secs(10);

    type TestRegistry = Registry<Connection, Arc<Adapters>>;

    async fn open(
        registry: &TestRegistry,
        repo: &str,
        connection: &Connection,
        adapters: &Arc<Adapters>,
    ) -> (Arc<Entry<Connection, Arc<Adapters>>>, bool) {
        registry
            .open(Path::new(repo), connection.clone(), |_, _| async {
                Ok::<_, ()>(adapters.clone())
            })
            .await
            .unwrap()
    }

    /// Time moves only when something sleeps; `on_tick` sees each sleep's number.
    struct Ticks<F> {
        now: AtomicU64,
        ticks: AtomicU64,
        on_tick: F,
    }

    impl<F: Fn(u64) + Sync> Clock for Ticks<F> {
        fn now(&self) -> Millis {
            Millis(self.now.load(Ordering::SeqCst))
        }

        async fn sleep_until(&self, deadline: Millis) {
            self.now.store(deadline.0, Ordering::SeqCst);
            (self.on_tick)(self.ticks.fetch_add(1, Ordering::SeqCst) + 1);
        }
    }

    fn ticks<F: Fn(u64) + Sync>(on_tick: F) -> Ticks<F> {
        Ticks {
            now: AtomicU64::new(0),
            ticks: AtomicU64::new(0),
            on_tick,
        }
    }

    #[tokio::test]
    async fn a_repository_has_one_project_that_every_opener_subscribes_to() {
        let registry = TestRegistry::default();
        let (a, b) = (Connection::new("a"), Connection::new("b"));
        let adapters = Arc::new(Adapters::default());

        let (first, created) = open(&registry, "/repo", &a, &adapters).await;
        let (second, created_again) = open(&registry, "/repo", &b, &adapters).await;
        open(&registry, "/repo", &b, &adapters).await;
        let (other, _) = open(&registry, "/other", &a, &Arc::default()).await;

        assert!(created && !created_again);
        assert_eq!(first.id, second.id);
        assert_ne!(first.id, other.id);
        first.subscribers.broadcast("settled".into());
        assert_eq!(
            (a.received(), b.received()),
            (vec!["settled".into()], vec!["settled".into()])
        );
    }

    #[tokio::test]
    async fn opening_a_slow_project_does_not_hold_up_the_others() {
        let registry = TestRegistry::default();
        let (a, b) = (Connection::new("a"), Connection::new("b"));
        let built = Notify::new();
        let slow = registry.open(Path::new("/slow"), a.clone(), |_, _| async {
            built.notified().await;
            Ok::<_, ()>(Arc::default())
        });
        let meanwhile = async {
            let (fast, created) = open(&registry, "/fast", &b, &Arc::default()).await;
            assert!(created);
            assert!(registry.find(fast.id).await.is_ok());
            assert_eq!(registry.all().await.len(), 1);
            built.notify_one();
        };

        let (slow, ()) = tokio::join!(slow, async {
            tokio::time::timeout(BLOCKED, meanwhile)
                .await
                .expect("another project waited for the slow one")
        });

        assert!(slow.unwrap().1);
        assert_eq!(registry.all().await.len(), 2);
    }

    #[tokio::test]
    async fn a_repository_opened_twice_at_once_is_built_once() {
        let registry = TestRegistry::default();
        let (a, b) = (Connection::new("a"), Connection::new("b"));
        let built = Notify::new();
        let builds = AtomicU64::new(0);
        let first = registry.open(Path::new("/repo"), a.clone(), |_, _| async {
            builds.fetch_add(1, Ordering::SeqCst);
            built.notified().await;
            Ok::<_, ()>(Arc::<Adapters>::default())
        });
        let second = registry.open(Path::new("/repo"), b.clone(), |_, _| async {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(Arc::<Adapters>::default())
        });

        let (first, second, ()) = tokio::join!(first, second, async {
            tokio::time::sleep(WAITING).await;
            built.notify_one();
        });

        let ((first, created), (second, created_again)) = (first.unwrap(), second.unwrap());
        assert_eq!((created, created_again), (true, false));
        assert_eq!(first.id, second.id);
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        first.subscribers.broadcast("x".into());
        assert_eq!((a.received(), b.received()), (vec!["x".into()], vec!["x".into()]));
    }

    #[tokio::test]
    async fn shutting_a_project_does_not_hold_up_the_others_but_its_repository_waits() {
        let registry = TestRegistry::default();
        let a = Connection::new("a");
        let slow = Arc::new(Adapters::default());
        slow.slow_shut.store(true, Ordering::SeqCst);
        let (leaving, _) = open(&registry, "/slow", &a, &slow).await;
        let closing = registry.close(leaving.id, &a);
        let meanwhile = async {
            let (other, _) = open(&registry, "/other", &a, &Arc::default()).await;
            assert!(registry.find(other.id).await.is_ok());
            let reopened = tokio::time::timeout(WAITING, open(&registry, "/slow", &a, &Arc::default())).await;
            assert!(reopened.is_err(), "the repository opened while it was shutting");
            slow.release.notify_one();
        };

        let (closed, ()) = tokio::join!(closing, async {
            tokio::time::timeout(BLOCKED, meanwhile)
                .await
                .expect("another project waited for the shutdown")
        });

        closed.unwrap();
        assert!(slow.shut.load(Ordering::SeqCst));
        let (again, created) = open(&registry, "/slow", &a, &Arc::default()).await;
        assert!(created && again.id != leaving.id);
    }

    #[tokio::test]
    async fn an_evicted_project_is_shut_and_tells_every_subscriber_whatever_they_do() {
        let registry = TestRegistry::default();
        let (a, b) = (Connection::new("a"), Connection::new("b"));
        let adapters = Arc::new(Adapters::default());
        let (entry, _) = open(&registry, "/repo", &a, &adapters).await;
        open(&registry, "/repo", &b, &adapters).await;
        let (other, _) = open(&registry, "/other", &a, &Arc::default()).await;

        let worked = registry.evict(Path::new("/repo"), "gone".into(), async { 7 }).await;

        assert_eq!(worked, 7);
        assert!(adapters.shut.load(Ordering::SeqCst));
        assert!(registry.find(entry.id).await.is_err());
        assert_eq!((a.received(), b.received()), (vec!["gone".into()], vec!["gone".into()]));
        assert!(registry.find(other.id).await.is_ok(), "another repository stays open");
        let (again, created) = open(&registry, "/repo", &a, &Arc::default()).await;
        assert!(created && again.id != entry.id);
    }

    #[tokio::test]
    async fn the_repository_of_an_evicted_project_cannot_open_while_the_work_runs() {
        let registry = TestRegistry::default();
        let a = Connection::new("a");
        open(&registry, "/repo", &a, &Arc::default()).await;
        let release = Notify::new();
        let evicting = registry.evict(Path::new("/repo"), "gone".into(), release.notified());
        let meanwhile = async {
            let reopened = tokio::time::timeout(WAITING, open(&registry, "/repo", &a, &Arc::default())).await;
            assert!(reopened.is_err(), "the repository opened while its removal ran");
            let (fast, created) = open(&registry, "/other", &a, &Arc::default()).await;
            assert!(created && registry.find(fast.id).await.is_ok());
            release.notify_one();
        };

        tokio::join!(evicting, async {
            tokio::time::timeout(BLOCKED, meanwhile)
                .await
                .expect("another repository waited for the removal")
        });

        let (_, created) = open(&registry, "/repo", &a, &Arc::default()).await;
        assert!(created);
    }

    #[tokio::test]
    async fn connections_that_hung_up_hear_nothing_and_do_not_count() {
        let registry = TestRegistry::default();
        let (a, b) = (Connection::new("a"), Connection::new("b"));
        let adapters = Arc::new(Adapters::default());
        let (entry, _) = open(&registry, "/repo", &a, &adapters).await;
        open(&registry, "/repo", &b, &adapters).await;

        a.hang_up();
        entry.subscribers.broadcast("x".into());
        assert_eq!((a.received(), b.received()), (vec![], vec!["x".into()]));

        b.hang_up();
        assert!(entry.subscribers.is_empty());
    }

    #[tokio::test]
    async fn the_last_connection_leaving_closes_the_project() {
        let registry = TestRegistry::default();
        let (a, b) = (Connection::new("a"), Connection::new("b"));
        let adapters = Arc::new(Adapters::default());
        let (entry, _) = open(&registry, "/repo", &a, &adapters).await;
        open(&registry, "/repo", &b, &adapters).await;

        registry.close(entry.id, &a).await.unwrap();
        assert!(!adapters.shut.load(Ordering::SeqCst));
        assert!(registry.find(entry.id).await.is_ok());

        registry.close(entry.id, &b).await.unwrap();
        assert!(adapters.shut.load(Ordering::SeqCst));
        assert!(registry.find(entry.id).await.is_err());
        assert!(registry.close(entry.id, &b).await.is_err());
    }

    #[tokio::test]
    async fn a_project_nobody_is_subscribed_to_closes_after_the_grace_period() {
        let registry = TestRegistry::default();
        let a = Connection::new("a");
        let adapters = Arc::new(Adapters::default());
        let (entry, _) = open(&registry, "/repo", &a, &adapters).await;
        a.hang_up();
        let clock = ticks(|_| {});

        registry.reap(entry.id, Duration::from_millis(400), &clock).await;

        assert!(adapters.shut.load(Ordering::SeqCst));
        assert!(registry.find(entry.id).await.is_err());
        assert!(clock.now().0 >= 400);
    }

    #[tokio::test]
    async fn a_connection_arriving_within_the_grace_period_restarts_it() {
        let registry = TestRegistry::default();
        let a = Connection::new("a");
        let adapters = Arc::new(Adapters::default());
        let (entry, _) = open(&registry, "/repo", &a, &adapters).await;
        let b = Connection::new("b");
        // Orphaned from tick 1; a reload subscribes on tick 3, leaves on tick 5.
        let clock = ticks(|tick| match tick {
            1 => a.hang_up(),
            3 => entry.subscribers.subscribe(b.clone()),
            5 => b.hang_up(),
            _ => {}
        });

        registry.reap(entry.id, Duration::from_millis(400), &clock).await;

        // Orphaned again from tick 5 on, so closing takes the whole grace again.
        assert!(clock.now().0 >= 5 * 100 + 400);
        assert!(adapters.shut.load(Ordering::SeqCst));
    }
}
