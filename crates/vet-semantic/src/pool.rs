//! Server pool (§7.2). Servers are keyed by `(target, backend, policy)`, kept
//! warm while in use, and evicted LRU. The policy is part of the key so that a
//! server started without permission to execute project code stops serving once
//! the PR is trusted (and the reverse can never happen).
//!
//! A server is leased for the duration of a query and released afterwards, so
//! no lock is held across an await. Two concurrent leases of one key start two
//! servers; callers that care serialize per key.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;

use vet_core::content::{RepoPath, SnapshotId};

use crate::policy::AnalysisPolicy;
use crate::ports::{
    At, Backend, BackendId, Diagnostic, LanguageServer, LaunchError, Location, Materializer, NotMaterializable,
    QueryError, ServerLauncher, SymbolInfo,
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
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMaterializable(e) => write!(f, "snapshot cannot be materialized: {e}"),
            Self::Launch(e) => e.fmt(f),
            Self::AnalysisLimited(b) => write!(f, "analysis limited: {} may execute project code", b.0),
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

/// A running server on loan from the pool. It answers from the cache when it
/// has one (snapshots only). Fields drop in order: server first, then the
/// materialized directory.
pub struct Lease<S, G> {
    key: PoolKey,
    generation: u64,
    server: S,
    cache: Option<Cache>,
    _guard: Option<G>,
}

macro_rules! cached {
    ($self:ident, $field:ident, $key:expr, $call:expr) => {{
        let key = $key;
        if let Some(hit) = $self.cache.as_ref().and_then(|c| c.$field.get(&key)) {
            return Ok(hit.clone());
        }
        let answer = $call.await?;
        if let Some(cache) = $self.cache.as_mut() {
            cache.$field.insert(key, answer.clone());
        }
        Ok(answer)
    }};
}

impl<S: LanguageServer, G: Send + Sync + 'static> LanguageServer for Lease<S, G> {
    async fn document_symbols(&mut self, file: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
        cached!(self, symbols, file.clone(), self.server.document_symbols(file))
    }

    async fn definition(&mut self, at: &At) -> Result<Vec<Location>, QueryError> {
        cached!(self, definition, at.clone(), self.server.definition(at))
    }

    async fn references(&mut self, at: &At) -> Result<Vec<Location>, QueryError> {
        cached!(self, references, at.clone(), self.server.references(at))
    }

    async fn hover(&mut self, at: &At) -> Result<Option<String>, QueryError> {
        cached!(self, hover, at.clone(), self.server.hover(at))
    }

    async fn incoming_calls(&mut self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
        cached!(self, incoming, at.clone(), self.server.incoming_calls(at))
    }

    async fn outgoing_calls(&mut self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
        cached!(self, outgoing, at.clone(), self.server.outgoing_calls(at))
    }

    async fn workspace_symbols(&mut self, query: &str) -> Result<Vec<SymbolInfo>, QueryError> {
        cached!(self, workspace, query.to_owned(), self.server.workspace_symbols(query))
    }

    async fn diagnostics(&mut self, file: &RepoPath) -> Result<Vec<Diagnostic>, QueryError> {
        cached!(self, diagnostics, file.clone(), self.server.diagnostics(file))
    }

    async fn shutdown(&mut self) {
        self.server.shutdown().await;
    }
}

struct Idle<S, G> {
    lease: Lease<S, G>,
    used: u64,
}

struct State<S, G> {
    tick: u64,
    /// Bumped by `restart`; a lease from an older generation is not pooled again.
    generations: HashMap<BackendId, u64>,
    idle: HashMap<PoolKey, Idle<S, G>>,
}

pub struct Pool<M: Materializer, L: ServerLauncher> {
    materializer: M,
    launcher: L,
    live_root: PathBuf,
    capacity: usize,
    state: Mutex<State<L::Server, M::Guard>>,
}

impl<M: Materializer, L: ServerLauncher> Pool<M, L> {
    /// `capacity` is the number of idle servers kept warm.
    pub fn new(materializer: M, launcher: L, live_root: PathBuf, capacity: usize) -> Self {
        Self {
            materializer,
            launcher,
            live_root,
            capacity,
            state: Mutex::new(State {
                tick: 0,
                generations: HashMap::new(),
                idle: HashMap::new(),
            }),
        }
    }

    /// A warm server for `key`, started if there is none.
    pub async fn lease(&self, key: &PoolKey, backend: &Backend) -> Result<Lease<L::Server, M::Guard>, PoolError> {
        admit(backend, key.policy)?;
        if let Some(idle) = self.lock().idle.remove(key) {
            return Ok(idle.lease);
        }
        let generation = self.lock().generations.get(&key.backend).copied().unwrap_or(0);
        let (root, guard, cache) = match key.target {
            Target::Live => (self.live_root.clone(), None, None),
            Target::Snapshot(id) => {
                let m = self
                    .materializer
                    .materialize(id)
                    .await
                    .map_err(PoolError::NotMaterializable)?;
                (m.root, Some(m.guard), Some(Cache::default()))
            }
        };
        let server = self
            .launcher
            .start(&root, backend, key.policy)
            .await
            .map_err(PoolError::Launch)?;
        Ok(Lease {
            key: key.clone(),
            generation,
            server,
            cache,
            _guard: guard,
        })
    }

    /// Return a server to the pool, evicting the least recently used beyond capacity.
    pub async fn release(&self, lease: Lease<L::Server, M::Guard>) {
        let evicted = self.park(lease);
        Self::stop(evicted).await;
    }

    /// Pools `lease` and returns what must be stopped: the lease itself if its
    /// backend was restarted meanwhile, else whatever exceeds capacity.
    fn park(&self, lease: Lease<L::Server, M::Guard>) -> Vec<Idle<L::Server, M::Guard>> {
        let mut state = self.lock();
        if state.generations.get(&lease.key.backend).copied().unwrap_or(0) != lease.generation {
            return vec![Idle { lease, used: 0 }];
        }
        state.tick += 1;
        let used = state.tick;
        let mut evicted: Vec<_> = state
            .idle
            .insert(lease.key.clone(), Idle { lease, used })
            .into_iter()
            .collect();
        while state.idle.len() > self.capacity {
            let oldest = state.idle.iter().min_by_key(|(_, i)| i.used).map(|(k, _)| k.clone());
            match oldest.and_then(|k| state.idle.remove(&k)) {
                Some(idle) => evicted.push(idle),
                None => break,
            }
        }
        evicted
    }

    /// Drop every server of `backend`: its settings changed (§7.2). Servers
    /// leased right now are dropped when released.
    pub async fn restart(&self, backend: &BackendId) {
        let stale = {
            let mut state = self.lock();
            *state.generations.entry(backend.clone()).or_default() += 1;
            let keys: Vec<_> = state.idle.keys().filter(|k| &k.backend == backend).cloned().collect();
            keys.iter().filter_map(|k| state.idle.remove(k)).collect()
        };
        Self::stop(stale).await;
    }

    pub async fn shutdown(&self) {
        let all = self.lock().idle.drain().map(|(_, i)| i).collect();
        Self::stop(all).await;
    }

    pub fn idle_keys(&self) -> Vec<PoolKey> {
        self.lock().idle.keys().cloned().collect()
    }

    async fn stop(servers: Vec<Idle<L::Server, M::Guard>>) {
        for mut idle in servers {
            idle.lease.shutdown().await;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<L::Server, M::Guard>> {
        self.state.lock().expect("pool state is never held across a panic")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vet_core::content::Hash;

    use super::*;
    use crate::policy::AnalysisPolicy;
    use crate::ports::{Materialized, Pos, ServerConfig};
    use crate::testutil::block_on;

    fn backend(executes: bool) -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId("rust".into()),
            binary: "rust-analyzer".into(),
            args: vec![],
            filetypes: vec!["rs".into()],
            root_markers: vec!["Cargo.toml".into()],
            executes_project_code_during_analysis: executes,
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

    #[test]
    fn policy_is_part_of_the_pool_key() {
        let key = |allow_executing| PoolKey {
            target: Target::Live,
            backend: BackendId("rust".into()),
            policy: AnalysisPolicy { allow_executing },
        };
        assert_ne!(key(false), key(true));
    }

    #[derive(Default)]
    struct Counters {
        materialized: AtomicUsize,
        guards_dropped: AtomicUsize,
        started: AtomicUsize,
        shut_down: AtomicUsize,
        hover_calls: AtomicUsize,
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

        async fn materialize(&self, _: SnapshotId) -> Result<Materialized<Guard>, NotMaterializable> {
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

        async fn start(&self, _: &std::path::Path, _: &Backend, _: AnalysisPolicy) -> Result<FakeServer, LaunchError> {
            self.0.started.fetch_add(1, Ordering::SeqCst);
            Ok(FakeServer(self.0.clone()))
        }
    }

    impl LanguageServer for FakeServer {
        async fn document_symbols(&mut self, _: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn definition(&mut self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        async fn references(&mut self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        /// Answers with how many times it was asked, so a cache hit is visible.
        async fn hover(&mut self, _: &At) -> Result<Option<String>, QueryError> {
            let n = self.0.hover_calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Some(n.to_string()))
        }
        async fn incoming_calls(&mut self, _: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn outgoing_calls(&mut self, _: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn workspace_symbols(&mut self, _: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn diagnostics(&mut self, _: &RepoPath) -> Result<Vec<Diagnostic>, QueryError> {
            Ok(vec![])
        }
        async fn shutdown(&mut self) {
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
            let mut lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            let first = lease.hover(&at()).await.unwrap();
            assert_eq!(lease.hover(&at()).await.unwrap(), first);
            pool.release(lease).await;
            let mut lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            assert_eq!(lease.hover(&at()).await.unwrap(), first);
            assert_eq!(counters.hover_calls.load(Ordering::SeqCst), 1);
            assert_eq!(counters.started.load(Ordering::SeqCst), 1);
            pool.release(lease).await;
        });
    }

    #[test]
    fn live_answers_are_never_cached_and_nothing_is_materialized() {
        let (pool, counters) = pool(2);
        block_on(async {
            let mut lease = pool.lease(&live_key(), &backend(true)).await.unwrap();
            let first = lease.hover(&at()).await.unwrap();
            assert_ne!(lease.hover(&at()).await.unwrap(), first);
            assert_eq!(counters.hover_calls.load(Ordering::SeqCst), 2);
            assert_eq!(counters.materialized.load(Ordering::SeqCst), 0);
            pool.release(lease).await;
        });
    }

    #[test]
    fn least_recently_used_server_is_evicted_with_its_directory() {
        let (pool, counters) = pool(2);
        block_on(async {
            for n in 1..=2 {
                let lease = pool.lease(&snapshot_key(n), &backend(true)).await.unwrap();
                pool.release(lease).await;
            }
            // Touch 1 so that 2 is the least recently used.
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            pool.release(lease).await;
            let lease = pool.lease(&snapshot_key(3), &backend(true)).await.unwrap();
            pool.release(lease).await;

            let mut keys = pool.idle_keys();
            keys.sort_by_key(|k| format!("{:?}", k.target));
            assert_eq!(keys, {
                let mut want = vec![snapshot_key(1), snapshot_key(3)];
                want.sort_by_key(|k| format!("{:?}", k.target));
                want
            });
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 1);
            assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn restart_drops_only_that_backends_servers() {
        let (pool, counters) = pool(4);
        block_on(async {
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            pool.release(lease).await;
            pool.restart(&BackendId("python".into())).await;
            assert_eq!(pool.idle_keys().len(), 1);
            pool.restart(&BackendId("rust".into())).await;
            assert!(pool.idle_keys().is_empty());
            assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);
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
