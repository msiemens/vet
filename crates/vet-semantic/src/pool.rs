//! Server pool (§7.2). Servers are keyed by `(target, backend, policy)`, kept
//! warm while in use, and evicted LRU. The policy is part of the key so that a
//! server started without permission to execute project code stops serving once
//! the PR is trusted (and the reverse can never happen).
//!
//! One server per key, however many callers: concurrent leases share it, and
//! callers arriving while it starts wait for that start instead of launching
//! another. Only servers with no lease are evicted. No lock is held across an
//! await.

use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use vet_core::content::{RepoPath, SnapshotId};

use crate::once::{Producer, Shared};
use crate::policy::AnalysisPolicy;
use crate::ports::{
    Answered, At, Backend, BackendId, Diagnostic, LanguageServer, LaunchError, Location, Materializer,
    NotMaterializable, Provenance, QueryError, ServerIdentity, ServerLauncher, Started, SymbolInfo, Tree,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Target {
    /// The dirty tree: served in place, answers never cached.
    Live,
    /// An immutable snapshot, served from a materialized directory.
    Snapshot(SnapshotId),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PoolKey {
    pub target: Target,
    pub backend: BackendId,
    pub policy: AnalysisPolicy,
}

#[derive(Debug)]
pub enum PoolError {
    NotMaterializable(NotMaterializable),
    Launch(LaunchError),
    /// The backend would execute project code and the policy forbids it. The UI
    /// shows an "analysis limited" marker for the language (§7.5).
    AnalysisLimited(BackendId),
    /// Another caller's start of the same server failed.
    StartFailed(String),
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMaterializable(e) => write!(f, "snapshot cannot be materialized: {e}"),
            Self::Launch(e) => e.fmt(f),
            Self::AnalysisLimited(b) => write!(f, "analysis limited: {} may execute project code", b.0),
            Self::StartFailed(why) => write!(f, "server start failed: {why}"),
        }
    }
}

/// Refuse a backend the policy does not allow, before anything is started.
pub fn admit(backend: &Backend, policy: AnalysisPolicy) -> Result<(), PoolError> {
    let config = backend.config();
    if config.executes_project_code_during_analysis && !policy.allow_executing {
        return Err(PoolError::AnalysisLimited(config.backend.clone()));
    }
    Ok(())
}

/// Answers of a server on an immutable snapshot, for the server's lifetime (I2).
#[derive(Default)]
struct Cache {
    symbols: HashMap<RepoPath, Vec<SymbolInfo>>,
    definition: HashMap<At, Vec<Location>>,
    references: HashMap<At, Vec<Location>>,
    hover: HashMap<At, Option<String>>,
    incoming: HashMap<At, Vec<SymbolInfo>>,
    outgoing: HashMap<At, Vec<SymbolInfo>>,
    workspace: HashMap<String, Vec<SymbolInfo>>,
    diagnostics: HashMap<RepoPath, Vec<Diagnostic>>,
}

/// A running server, shared by its leases. It answers from the cache when it
/// has one (snapshots only), and says who answered. Fields drop in order:
/// server first, then the materialized directory.
pub struct Entry<S, G> {
    server: S,
    /// The identity the server started with.
    identity: ServerIdentity,
    cache: Option<Mutex<Cache>>,
    leases: AtomicUsize,
    used: AtomicU64,
    /// Restarted while leased: stopped when the last lease ends.
    retired: AtomicBool,
    _guard: Option<G>,
}

macro_rules! cached {
    ($self:ident, $field:ident, $key:expr, $call:expr) => {{
        let key = $key;
        if let Some(hit) = $self
            .cache
            .as_ref()
            .and_then(|c| c.lock().expect("not poisoned").$field.get(&key).cloned())
        {
            return Ok($self.answered(hit, true));
        }
        let answer = $call.await?;
        if let Some(cache) = &$self.cache {
            cache.lock().expect("not poisoned").$field.insert(key, answer.clone());
        }
        Ok($self.answered(answer, false))
    }};
}

impl<S: LanguageServer, G: Send + Sync + 'static> Entry<S, G> {
    pub fn identity(&self) -> &ServerIdentity {
        &self.identity
    }

    fn answered<T>(&self, value: T, from_cache: bool) -> Answered<T> {
        Answered {
            value,
            provenance: Provenance {
                server: self.identity.clone(),
                from_cache,
            },
        }
    }

    pub async fn document_symbols(&self, file: &RepoPath) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        cached!(self, symbols, file.clone(), self.server.document_symbols(file))
    }

    pub async fn text_symbols(&self, file: &RepoPath, text: &str) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        Ok(self.answered(self.server.text_symbols(file, text).await?, false))
    }

    pub async fn definition(&self, at: &At) -> Result<Answered<Vec<Location>>, QueryError> {
        cached!(self, definition, at.clone(), self.server.definition(at))
    }

    pub async fn references(&self, at: &At) -> Result<Answered<Vec<Location>>, QueryError> {
        cached!(self, references, at.clone(), self.server.references(at))
    }

    pub async fn hover(&self, at: &At) -> Result<Answered<Option<String>>, QueryError> {
        cached!(self, hover, at.clone(), self.server.hover(at))
    }

    pub async fn incoming_calls(&self, at: &At) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        cached!(self, incoming, at.clone(), self.server.incoming_calls(at))
    }

    pub async fn outgoing_calls(&self, at: &At) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        cached!(self, outgoing, at.clone(), self.server.outgoing_calls(at))
    }

    pub async fn workspace_symbols(&self, query: &str) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        cached!(self, workspace, query.to_owned(), self.server.workspace_symbols(query))
    }

    pub async fn diagnostics(&self, file: &RepoPath) -> Result<Answered<Vec<Diagnostic>>, QueryError> {
        cached!(self, diagnostics, file.clone(), self.server.diagnostics(file))
    }

    async fn shutdown(&self) {
        self.server.shutdown().await;
    }
}

/// How a start ended for the callers who waited for it: the server is in the
/// pool (`Ok`), or the start failed for this reason.
type StartEnd = Result<(), String>;

enum Slot<S, G> {
    Starting(Arc<Shared<StartEnd>>),
    Running(Arc<Entry<S, G>>),
}

struct State<S, G> {
    tick: u64,
    /// Bumped by `restart`; an entry from an older generation is not pooled.
    generations: HashMap<BackendId, u64>,
    slots: HashMap<PoolKey, Slot<S, G>>,
}

pub struct Pool<M: Materializer, L: ServerLauncher> {
    materializer: M,
    launcher: L,
    live_root: PathBuf,
    capacity: usize,
    state: Mutex<State<L::Server, M::Guard>>,
}

type EntryOf<M, L> = Entry<<L as ServerLauncher>::Server, <M as Materializer>::Guard>;

/// A claim on a shared server. Ends on `release` or drop.
pub struct Lease<'p, M: Materializer, L: ServerLauncher> {
    pool: &'p Pool<M, L>,
    entry: Option<Arc<EntryOf<M, L>>>,
}

impl<M: Materializer, L: ServerLauncher> Deref for Lease<'_, M, L> {
    type Target = EntryOf<M, L>;

    fn deref(&self) -> &Self::Target {
        self.entry.as_ref().expect("lease is live until released")
    }
}

impl<M: Materializer, L: ServerLauncher> Drop for Lease<'_, M, L> {
    fn drop(&mut self) {
        if let Some(entry) = self.entry.take() {
            // Not awaited: a retired server is just dropped, which kills it.
            drop(self.pool.end_lease(entry));
        }
    }
}

impl<M: Materializer, L: ServerLauncher> Lease<'_, M, L> {
    /// End the lease, shutting the server down cleanly if this was the last
    /// lease of a restarted or evicted server.
    pub async fn release(mut self) {
        if let Some(entry) = self.entry.take() {
            let stop = self.pool.end_lease(entry);
            Pool::<M, L>::stop(stop).await;
        }
    }
}

/// A start in progress. If its starter is dropped before the start ended, the
/// slot is freed and the waiters are woken, so one of them starts the server.
struct StartGuard<'p, M: Materializer, L: ServerLauncher> {
    pool: &'p Pool<M, L>,
    key: PoolKey,
    /// Taken when the start ended.
    producer: Option<Producer<StartEnd>>,
}

impl<M: Materializer, L: ServerLauncher> StartGuard<'_, M, L> {
    fn end(&mut self, outcome: StartEnd) {
        if let Some(producer) = self.producer.take() {
            producer.finish(outcome);
        }
    }
}

impl<M: Materializer, L: ServerLauncher> Drop for StartGuard<'_, M, L> {
    fn drop(&mut self) {
        // The slot goes before the producer is dropped: a woken waiter must find it free.
        if self.producer.is_some() {
            self.pool.lock().slots.remove(&self.key);
        }
    }
}

impl<M: Materializer, L: ServerLauncher> Pool<M, L> {
    /// `capacity` is the number of servers kept warm; leased ones never count against eviction.
    pub fn new(materializer: M, launcher: L, live_root: PathBuf, capacity: usize) -> Self {
        Self {
            materializer,
            launcher,
            live_root,
            capacity,
            state: Mutex::new(State {
                tick: 0,
                generations: HashMap::new(),
                slots: HashMap::new(),
            }),
        }
    }

    /// The server for `key`: the running one, the one being started, or a new one.
    /// A caller that waited for a start which was abandoned or whose server did
    /// not stay (it was restarted meanwhile) asks again, and may start the
    /// server itself.
    pub async fn lease(&self, key: &PoolKey, backend: &Backend) -> Result<Lease<'_, M, L>, PoolError> {
        admit(backend, key.policy)?;
        enum Plan<S, G> {
            Join(Arc<Entry<S, G>>),
            Wait(Arc<Shared<StartEnd>>),
            Start(Producer<StartEnd>, u64),
        }
        loop {
            let plan = {
                let mut state = self.lock();
                state.tick += 1;
                let tick = state.tick;
                let generation = state.generations.get(&key.backend).copied().unwrap_or(0);
                match state.slots.get(key) {
                    Some(Slot::Running(entry)) => {
                        entry.leases.fetch_add(1, Ordering::SeqCst);
                        entry.used.store(tick, Ordering::SeqCst);
                        Plan::Join(entry.clone())
                    }
                    Some(Slot::Starting(start)) => Plan::Wait(start.clone()),
                    None => {
                        let (start, producer) = Shared::start();
                        state.slots.insert(key.clone(), Slot::Starting(start));
                        Plan::Start(producer, generation)
                    }
                }
            };
            let entry = match plan {
                Plan::Join(entry) => entry,
                Plan::Wait(start) => match start.wait().await {
                    Some(Err(why)) => return Err(PoolError::StartFailed(why)),
                    Some(Ok(())) | None => continue,
                },
                Plan::Start(producer, generation) => self.start(key, backend, producer, generation).await?,
            };
            return Ok(Lease {
                pool: self,
                entry: Some(entry),
            });
        }
    }

    async fn start(
        &self,
        key: &PoolKey,
        backend: &Backend,
        producer: Producer<StartEnd>,
        generation: u64,
    ) -> Result<Arc<EntryOf<M, L>>, PoolError> {
        let mut guard = StartGuard {
            pool: self,
            key: key.clone(),
            producer: Some(producer),
        };
        let (Started { server, identity }, cache, dir_guard) = match self.launch(key, backend).await {
            Ok(parts) => parts,
            Err(e) => {
                self.lock().slots.remove(key);
                guard.end(Err(e.to_string()));
                return Err(e);
            }
        };
        let entry = {
            let mut state = self.lock();
            let stale = state.generations.get(&key.backend).copied().unwrap_or(0) != generation;
            state.tick += 1;
            let entry = Arc::new(Entry {
                server,
                identity,
                cache,
                leases: AtomicUsize::new(1),
                used: AtomicU64::new(state.tick),
                retired: AtomicBool::new(stale),
                _guard: dir_guard,
            });
            if stale {
                state.slots.remove(key);
            } else {
                state.slots.insert(key.clone(), Slot::Running(entry.clone()));
            }
            entry
        };
        guard.end(Ok(()));
        Ok(entry)
    }

    async fn launch(
        &self,
        key: &PoolKey,
        backend: &Backend,
    ) -> Result<(Started<L::Server>, Option<Mutex<Cache>>, Option<M::Guard>), PoolError> {
        let (root, tree, guard, cache) = match key.target {
            Target::Live => (self.live_root.clone(), Tree::Live, None, None),
            Target::Snapshot(id) => {
                let m = self
                    .materializer
                    .materialize(id, key.policy)
                    .await
                    .map_err(PoolError::NotMaterializable)?;
                (
                    m.root,
                    Tree::Immutable,
                    Some(m.guard),
                    Some(Mutex::new(Cache::default())),
                )
            }
        };
        let started = self
            .launcher
            .start(&root, tree, backend, key.policy)
            .await
            .map_err(PoolError::Launch)?;
        Ok((started, cache, guard))
    }

    /// Drops one lease; returns the entries to stop (this one if it was
    /// retired, plus whatever exceeds capacity).
    fn end_lease(&self, entry: Arc<EntryOf<M, L>>) -> Vec<Arc<EntryOf<M, L>>> {
        let mut state = self.lock();
        let left = entry.leases.fetch_sub(1, Ordering::SeqCst) - 1;
        let mut stop = Vec::new();
        if left == 0 && entry.retired.load(Ordering::SeqCst) {
            stop.push(entry);
        }
        while state.slots.len() > self.capacity {
            let oldest = state
                .slots
                .iter()
                .filter_map(|(k, s)| match s {
                    Slot::Running(e) if e.leases.load(Ordering::SeqCst) == 0 => {
                        Some((k, e.used.load(Ordering::SeqCst)))
                    }
                    _ => None,
                })
                .min_by_key(|(_, used)| *used)
                .map(|(k, _)| k.clone());
            match oldest.and_then(|k| state.slots.remove(&k)) {
                Some(Slot::Running(e)) => stop.push(e),
                _ => break,
            }
        }
        stop
    }

    /// Drop every server of `backend`: its settings changed (§7.2). Servers
    /// leased right now are stopped when their last lease ends.
    pub async fn restart(&self, backend: &BackendId) {
        let stop = {
            let mut state = self.lock();
            *state.generations.entry(backend.clone()).or_default() += 1;
            let keys: Vec<_> = state
                .slots
                .iter()
                .filter(|(k, s)| &k.backend == backend && matches!(s, Slot::Running(_)))
                .map(|(k, _)| k.clone())
                .collect();
            let mut stop = Vec::new();
            for key in keys {
                if let Some(Slot::Running(e)) = state.slots.remove(&key) {
                    if e.leases.load(Ordering::SeqCst) == 0 {
                        stop.push(e);
                    } else {
                        e.retired.store(true, Ordering::SeqCst);
                    }
                }
            }
            stop
        };
        Self::stop(stop).await;
    }

    /// Stop every server that is not leased.
    pub async fn shutdown(&self) {
        let stop = {
            let mut state = self.lock();
            let keys: Vec<_> = state
                .slots
                .iter()
                .filter(|(_, s)| matches!(s, Slot::Running(e) if e.leases.load(Ordering::SeqCst) == 0))
                .map(|(k, _)| k.clone())
                .collect();
            keys.iter()
                .filter_map(|k| match state.slots.remove(k) {
                    Some(Slot::Running(e)) => Some(e),
                    _ => None,
                })
                .collect()
        };
        Self::stop(stop).await;
    }

    #[cfg(test)]
    pub fn keys(&self) -> Vec<PoolKey> {
        self.lock().slots.keys().cloned().collect()
    }

    async fn stop(entries: Vec<Arc<EntryOf<M, L>>>) {
        for entry in entries {
            entry.shutdown().await;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<L::Server, M::Guard>> {
        self.state.lock().expect("pool state is never held across a panic")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::task::{Context, Waker};

    use vet_core::content::Hash;

    use super::*;
    use crate::join::join;
    use crate::policy::AnalysisPolicy;
    use crate::ports::{Materialized, Pos, ServerConfig};
    use crate::testutil::{block_on, yield_once};

    fn backend(executes: bool) -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId("rust".into()),
            binary: "rust-analyzer".into(),
            args: vec![],
            filetypes: vec!["rs".into()],
            root_markers: vec!["Cargo.toml".into()],
            executes_project_code_during_analysis: executes,
            loads_project_on_open: false,
        })
    }

    #[test]
    fn untrusted_policy_refuses_executing_backends_only() {
        let untrusted = AnalysisPolicy { allow_executing: false };
        let trusted = AnalysisPolicy { allow_executing: true };
        assert!(matches!(
            admit(&backend(true), untrusted),
            Err(PoolError::AnalysisLimited(_))
        ));
        assert!(admit(&backend(false), untrusted).is_ok());
        assert!(admit(&backend(true), trusted).is_ok());
    }

    #[derive(Default)]
    struct Counters {
        materialized: AtomicUsize,
        guards_dropped: AtomicUsize,
        started: AtomicUsize,
        shut_down: AtomicUsize,
        hover_calls: AtomicUsize,
        fail_start: AtomicBool,
    }

    struct Guard(Arc<Counters>);

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.guards_dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeMaterializer(Arc<Counters>);

    impl Materializer for FakeMaterializer {
        type Guard = Guard;

        async fn materialize(
            &self,
            _: SnapshotId,
            _: AnalysisPolicy,
        ) -> Result<Materialized<Guard>, NotMaterializable> {
            self.0.materialized.fetch_add(1, Ordering::SeqCst);
            Ok(Materialized {
                root: PathBuf::from("/snap"),
                guard: Guard(self.0.clone()),
            })
        }
    }

    struct FakeLauncher(Arc<Counters>);

    struct FakeServer(Arc<Counters>);

    impl ServerLauncher for FakeLauncher {
        type Server = FakeServer;

        /// Slow, so a second caller can arrive while the first is starting.
        async fn start(
            &self,
            root: &std::path::Path,
            _: Tree,
            backend: &Backend,
            _: AnalysisPolicy,
        ) -> Result<Started<FakeServer>, LaunchError> {
            self.0.started.fetch_add(1, Ordering::SeqCst);
            yield_once().await;
            if self.0.fail_start.load(Ordering::SeqCst) {
                return Err(LaunchError::Spawn("boom".into()));
            }
            Ok(Started {
                server: FakeServer(self.0.clone()),
                identity: self.identify(root, backend).await?,
            })
        }

        /// A binary that is replaced between two starts: the nth start is version n.
        async fn identify(&self, _: &std::path::Path, backend: &Backend) -> Result<ServerIdentity, LaunchError> {
            let n = self.0.started.load(Ordering::SeqCst);
            Ok(ServerIdentity {
                backend: backend.config().backend.clone(),
                fingerprint: Hash([n as u8; 32]),
                label: format!("fake {n}"),
            })
        }
    }

    impl LanguageServer for FakeServer {
        async fn document_symbols(&self, _: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn text_symbols(&self, _: &RepoPath, _: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn definition(&self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        async fn references(&self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        /// Answers with how many times it was asked, so a cache hit is visible.
        async fn hover(&self, _: &At) -> Result<Option<String>, QueryError> {
            let n = self.0.hover_calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Some(n.to_string()))
        }
        async fn incoming_calls(&self, _: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn outgoing_calls(&self, _: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn workspace_symbols(&self, _: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn diagnostics(&self, _: &RepoPath) -> Result<Vec<Diagnostic>, QueryError> {
            Ok(vec![])
        }
        async fn shutdown(&self) {
            self.0.shut_down.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn pool(capacity: usize) -> (Pool<FakeMaterializer, FakeLauncher>, Arc<Counters>) {
        let counters = Arc::new(Counters::default());
        let pool = Pool::new(
            FakeMaterializer(counters.clone()),
            FakeLauncher(counters.clone()),
            PathBuf::from("/live"),
            capacity,
        );
        (pool, counters)
    }

    fn snapshot_key(n: u8) -> PoolKey {
        PoolKey {
            target: Target::Snapshot(SnapshotId(Hash([n; 32]))),
            backend: BackendId("rust".into()),
            policy: AnalysisPolicy { allow_executing: true },
        }
    }

    fn live_key() -> PoolKey {
        PoolKey {
            target: Target::Live,
            ..snapshot_key(0)
        }
    }

    fn at() -> At {
        At {
            file: RepoPath(b"a.rs".to_vec()),
            pos: Pos { line: 1, col: 2 },
        }
    }

    #[test]
    fn snapshot_answers_are_cached_for_the_servers_lifetime() {
        let (pool, counters) = pool(2);
        block_on(async {
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            let first = lease.hover(&at()).await.unwrap().value;
            assert_eq!(lease.hover(&at()).await.unwrap().value, first);
            lease.release().await;
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            assert_eq!(lease.hover(&at()).await.unwrap().value, first);
            assert_eq!(counters.hover_calls.load(Ordering::SeqCst), 1);
            assert_eq!(counters.started.load(Ordering::SeqCst), 1);
            lease.release().await;
        });
    }

    #[test]
    fn an_answer_names_the_server_that_started_and_whether_it_was_reused() {
        let (pool, _) = pool(2);
        block_on(async {
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            let fresh = lease.hover(&at()).await.unwrap().provenance;
            let reused = lease.hover(&at()).await.unwrap().provenance;
            assert_eq!(fresh.server, *lease.identity());
            assert_eq!(fresh.server.label, "fake 1");
            assert!(!fresh.from_cache && reused.from_cache);
            assert_eq!(reused.server, fresh.server);
            assert_eq!(fresh.to_string(), "fake 1");
            assert_eq!(reused.to_string(), "fake 1, cached");
            lease.release().await;

            // A server started later is another identity, and so is its answer's.
            pool.restart(&BackendId("rust".into())).await;
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            let again = lease.hover(&at()).await.unwrap().provenance;
            assert_eq!(again.server.label, "fake 2");
            assert!(!again.from_cache);
            lease.release().await;
        });
    }

    #[test]
    fn live_answers_are_never_cached_and_nothing_is_materialized() {
        let (pool, counters) = pool(2);
        block_on(async {
            let lease = pool.lease(&live_key(), &backend(true)).await.unwrap();
            let first = lease.hover(&at()).await.unwrap().value;
            assert_ne!(lease.hover(&at()).await.unwrap().value, first);
            assert_eq!(counters.hover_calls.load(Ordering::SeqCst), 2);
            assert_eq!(counters.materialized.load(Ordering::SeqCst), 0);
            lease.release().await;
        });
    }

    #[test]
    fn concurrent_leases_of_one_key_launch_one_server() {
        let (pool, counters) = pool(2);
        block_on(async {
            let key = snapshot_key(1);
            let (a, b) = join(pool.lease(&key, &backend(true)), pool.lease(&key, &backend(true))).await;
            let (a, b) = (a.unwrap(), b.unwrap());
            assert_eq!(counters.started.load(Ordering::SeqCst), 1);
            assert_eq!(counters.materialized.load(Ordering::SeqCst), 1);
            assert!(Arc::ptr_eq(a.entry.as_ref().unwrap(), b.entry.as_ref().unwrap()));
            assert_eq!(a.leases.load(Ordering::SeqCst), 2);
            // Both see one shared cache.
            let first = a.hover(&at()).await.unwrap().value;
            assert_eq!(b.hover(&at()).await.unwrap().value, first);
            a.release().await;
            b.release().await;
            assert_eq!(pool.keys(), [key]);
        });
    }

    #[test]
    fn waiters_see_a_failed_start_and_the_next_caller_retries() {
        let (pool, counters) = pool(2);
        counters.fail_start.store(true, Ordering::SeqCst);
        block_on(async {
            let key = snapshot_key(1);
            let (a, b) = join(pool.lease(&key, &backend(true)), pool.lease(&key, &backend(true))).await;
            assert!(matches!(a, Err(PoolError::Launch(_))));
            assert!(matches!(b, Err(PoolError::StartFailed(_))));
            assert_eq!(counters.started.load(Ordering::SeqCst), 1);
            assert!(pool.keys().is_empty());
            assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);

            counters.fail_start.store(false, Ordering::SeqCst);
            pool.lease(&key, &backend(true)).await.unwrap().release().await;
            assert_eq!(counters.started.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn a_waiter_takes_over_a_start_whose_caller_gave_up() {
        let (pool, counters) = pool(2);
        let key = snapshot_key(1);
        let be = backend(true);
        let mut cx = Context::from_waker(Waker::noop());
        let mut starter = Box::pin(pool.lease(&key, &be));
        let mut first = Box::pin(pool.lease(&key, &be));
        let mut second = Box::pin(pool.lease(&key, &be));
        assert!(starter.as_mut().poll(&mut cx).is_pending());
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert!(second.as_mut().poll(&mut cx).is_pending());
        assert_eq!(counters.started.load(Ordering::SeqCst), 1);

        drop(starter);
        assert!(pool.keys().is_empty());
        block_on(async {
            let (first, second) = join(first, second).await;
            let (first, second) = (first.unwrap(), second.unwrap());
            assert_eq!(counters.started.load(Ordering::SeqCst), 2);
            assert!(Arc::ptr_eq(
                first.entry.as_ref().unwrap(),
                second.entry.as_ref().unwrap()
            ));
            assert_eq!(first.leases.load(Ordering::SeqCst), 2);
            assert_eq!(pool.keys(), [snapshot_key(1)]);
            first.release().await;
            second.release().await;
        });
        // The abandoned start's directory is gone, the taken-over one's is kept.
        assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_waiter_that_gave_up_holds_no_lease() {
        let (pool, _) = pool(2);
        let key = snapshot_key(1);
        let be = backend(true);
        let mut cx = Context::from_waker(Waker::noop());
        let mut starter = Box::pin(pool.lease(&key, &be));
        let mut waiter = Box::pin(pool.lease(&key, &be));
        assert!(starter.as_mut().poll(&mut cx).is_pending());
        assert!(waiter.as_mut().poll(&mut cx).is_pending());
        drop(waiter);
        block_on(async {
            let lease = starter.await.unwrap();
            assert_eq!(lease.leases.load(Ordering::SeqCst), 1);
            lease.release().await;
        });
    }

    #[test]
    fn leased_servers_are_never_evicted() {
        let (pool, counters) = pool(1);
        block_on(async {
            let held = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            pool.lease(&snapshot_key(2), &backend(true))
                .await
                .unwrap()
                .release()
                .await;
            assert_eq!(pool.keys(), [snapshot_key(1)]);
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 1);
            held.release().await;
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn least_recently_used_server_is_evicted_with_its_directory() {
        let (pool, counters) = pool(2);
        block_on(async {
            for n in [1, 2, 1, 3] {
                pool.lease(&snapshot_key(n), &backend(true))
                    .await
                    .unwrap()
                    .release()
                    .await;
            }
            let mut keys = pool.keys();
            keys.sort_by_key(|k| format!("{:?}", k.target));
            let mut want = vec![snapshot_key(1), snapshot_key(3)];
            want.sort_by_key(|k| format!("{:?}", k.target));
            assert_eq!(keys, want);
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 1);
            assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn restart_drops_only_that_backends_servers() {
        let (pool, counters) = pool(4);
        block_on(async {
            pool.lease(&snapshot_key(1), &backend(true))
                .await
                .unwrap()
                .release()
                .await;
            pool.restart(&BackendId("python".into())).await;
            assert_eq!(pool.keys().len(), 1);
            pool.restart(&BackendId("rust".into())).await;
            assert!(pool.keys().is_empty());
            assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn restart_stops_a_leased_server_when_its_lease_ends() {
        let (pool, counters) = pool(4);
        block_on(async {
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            pool.restart(&BackendId("rust".into())).await;
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 0);
            lease.release().await;
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 1);
            assert!(pool.keys().is_empty());
        });
    }

    #[test]
    fn refused_backend_starts_nothing() {
        let (pool, counters) = pool(2);
        let key = PoolKey {
            policy: AnalysisPolicy { allow_executing: false },
            ..snapshot_key(1)
        };
        let result = block_on(pool.lease(&key, &backend(true)));
        assert!(matches!(result, Err(PoolError::AnalysisLimited(_))));
        assert_eq!(counters.materialized.load(Ordering::SeqCst), 0);
    }
}
