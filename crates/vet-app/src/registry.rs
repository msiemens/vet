//! The open projects of a process: one per repository, the connections
//! subscribed to each, and when a project without connections closes.

use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

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
        let mut subscribers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
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

#[derive(Debug)]
pub struct NoSuchProject;

impl fmt::Display for NoSuchProject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no such project")
    }
}

type Entries<S, T> = Arc<tokio::sync::Mutex<Vec<Arc<Entry<S, T>>>>>;

pub struct Registry<S, T> {
    entries: Entries<S, T>,
    next_id: Arc<AtomicU32>,
}

impl<S, T> Clone for Registry<S, T> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            next_id: self.next_id.clone(),
        }
    }
}

impl<S, T> Default for Registry<S, T> {
    fn default() -> Self {
        Self {
            entries: Arc::default(),
            next_id: Arc::new(AtomicU32::new(1)),
        }
    }
}

impl<S: Subscriber, T: Resident> Registry<S, T> {
    /// The project of `repo` with `subscriber` subscribed; `create` builds it
    /// from its new id and subscribers when the repository is not open yet.
    /// Whether it did is the second value. Creation holds the registry, so a
    /// repository is never opened twice.
    pub async fn open<E, F>(
        &self,
        repo: &Path,
        subscriber: S,
        create: impl FnOnce(u32, Subscribers<S>) -> F,
    ) -> Result<(Arc<Entry<S, T>>, bool), E>
    where
        F: Future<Output = Result<T, E>>,
    {
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.iter().find(|e| e.repo == repo) {
            entry.subscribers.subscribe(subscriber);
            return Ok((entry.clone(), false));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let subscribers = Subscribers::new(subscriber);
        let resident = create(id, subscribers.clone()).await?;
        let entry = Arc::new(Entry {
            id,
            repo: repo.to_owned(),
            subscribers,
            resident,
        });
        entries.push(entry.clone());
        Ok((entry, true))
    }

    pub async fn find(&self, id: u32) -> Result<Arc<Entry<S, T>>, NoSuchProject> {
        let entries = self.entries.lock().await;
        entries.iter().find(|e| e.id == id).cloned().ok_or(NoSuchProject)
    }

    pub async fn all(&self) -> Vec<Arc<Entry<S, T>>> {
        self.entries.lock().await.clone()
    }

    /// Unsubscribes the connection; the last one to leave closes the project.
    pub async fn close(&self, id: u32, subscriber: &S) -> Result<(), NoSuchProject> {
        let entry = self.find(id).await?;
        if entry.subscribers.unsubscribe(subscriber) {
            self.entries.lock().await.retain(|e| e.id != id);
            entry.shut().await;
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
            let mut entries = self.entries.lock().await;
            let Some(entry) = entries.iter().find(|e| e.id == id).cloned() else {
                return;
            };
            if !entry.subscribers.is_empty() {
                orphaned_since = None;
                continue;
            }
            let now = clock.now();
            let since = *orphaned_since.get_or_insert(now);
            if now.0 - since.0 >= grace {
                // Holding the list lock, so a connection cannot attach meanwhile.
                entries.retain(|e| e.id != id);
                entry.shut().await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64};

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
    }

    impl Resident for Arc<Adapters> {
        async fn shut(&self) {
            self.shut.store(true, Ordering::SeqCst);
        }
    }

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
