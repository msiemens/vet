//! Server pool (§7.2). Servers are keyed by `(target, backend, policy)`, kept
//! warm while in use, and evicted LRU. The policy is part of the key so that a
//! server started without permission to execute project code stops serving once
//! the PR is trusted (and the reverse can never happen).
//!
//! One server per key, however many callers: concurrent leases share it, and
//! callers arriving while it starts join that start instead of launching
//! another. A start is a flight of the engine: it goes on while any of its callers
//! waits, whichever of them began it, and all of them get the server it started.
//! Only servers with no lease are evicted. No lock is held across an await.
//!
//! Answers outlive their server: they are kept by the pool under keys that name
//! the server's identity (I2, `reuse`), so a server that starts later with an
//! equal identity on an equal tree answers from them. Those that persist also
//! outlive the process, in the store the pool is given. A question whose key can be
//! worked out without a server (`recall`) is looked up in both before any server is
//! leased, so a tree whose answers are kept is neither materialized nor started on.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vet_core::content::{BlobId, RepoPath, SnapshotId};
use vet_engine::{self as engine, Engine, Kind, Limits, Retention};

use crate::gate::Gate;
use crate::policy::AnalysisPolicy;
use crate::ports::{
    Answered, At, Backend, BackendId, Detach, Diagnostic, Features, LanguageServer, LaunchError, Location,
    Materializer, NotMaterializable, Provenance, QueryError, ServerIdentity, ServerLauncher, Started, SymbolInfo, Tree,
};
use crate::reuse::{Answers, Basis, Computed, KEPT, Key, ProjectQuery, Restore, Reused, SemanticCache};

/// Requests in flight on one server at once. A server answers one at a time or a few, so
/// more only wait in its queue, where they cannot be told from a hung server.
pub const REQUESTS_PER_SERVER: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Target {
    /// The dirty tree: served in place. Its answers are asked fresh, except the
    /// symbols of a text, which depend on that text alone.
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

#[derive(Clone, Debug)]
pub enum PoolError {
    NotMaterializable(NotMaterializable),
    Launch(LaunchError),
    /// The backend would execute project code and the policy forbids it. The UI
    /// shows an "analysis limited" marker for the language (§7.5).
    AnalysisLimited(BackendId),
    /// The server died while running, and again soon after it was restarted for that; it stays
    /// stopped until it is restarted by hand. Carries why it died.
    Stopped(String),
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMaterializable(e) => write!(f, "snapshot cannot be materialized: {e}"),
            Self::Launch(e) => e.fmt(f),
            Self::Stopped(why) => write!(f, "the language server stopped unexpectedly and stays stopped: {why}"),
            Self::AnalysisLimited(b) => write!(
                f,
                "analysis limited: {} may execute project code, which this pull request is not trusted for",
                b.0
            ),
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

/// A server that died while running is started anew once. If it dies again within this long
/// of that start having finished, it stays stopped.
const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// What became of a server that stopped on its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// A new server runs in its place.
    Restarted,
    /// It stays stopped until it is restarted by hand.
    Stopped,
}

/// Watches the running servers: how the pool runs the watching, and whom it tells what became
/// of a server that died.
pub struct Supervision {
    pub detach: Arc<dyn Detach>,
    pub told: Arc<Told>,
}

/// Told which backend's server stopped on its own and what became of it.
pub type Told = dyn Fn(&BackendId, Outcome) + Send + Sync;

/// What the pool knows of a server's key after a server of it died.
enum Health {
    /// Started anew after it died, at this time.
    Restarted(Instant),
    /// Died soon after that, or could not be started anew; the reason.
    Stopped(String),
}

/// The key of a question about a whole immutable tree.
fn project_key(server: &ServerIdentity, basis: Basis, query: ProjectQuery) -> Key {
    Key::Project {
        server: server.clone(),
        basis,
        query,
    }
}

/// The key of the symbols `text` declares as the content of `file`.
fn text_key(server: &ServerIdentity, file: &RepoPath, text: &str) -> Key {
    Key::Text {
        server: server.clone(),
        extension: file.extension().unwrap_or_default().to_vec(),
        blob: BlobId::of(text.as_bytes()),
    }
}

/// An answer kept for the same text under another name names its symbols after that one.
fn named_after(file: &RepoPath, symbols: &mut [SymbolInfo]) {
    for symbol in symbols {
        symbol.sym.file = file.clone();
    }
}

fn answered<T>(value: T, server: ServerIdentity, from_cache: bool) -> Answered<T> {
    Answered {
        value,
        provenance: Provenance { server, from_cache },
    }
}

/// A running server, shared by its leases. It answers from the pool's kept
/// answers when it can (I2), and says who answered. Fields drop in order:
/// server first, then the materialized directory.
pub struct Entry<S, G> {
    server: S,
    /// What it was started for, to start it anew if it dies.
    key: PoolKey,
    backend: Backend,
    /// The identity the server started with.
    identity: ServerIdentity,
    answers: Arc<Answers>,
    /// What the tree is, for the answers that depend on all of it; none for the live tree.
    basis: Option<Basis>,
    leases: AtomicUsize,
    used: AtomicU64,
    /// Restarted while leased: stopped when the last lease ends.
    retired: AtomicBool,
    /// The server was told to stop; stopping it again does nothing.
    stopped: AtomicBool,
    /// Bounds the requests in flight on the server, however many questions are asked of it.
    requests: Arc<Gate>,
    _guard: Option<G>,
}

impl<S: LanguageServer + 'static, G: Send + Sync + 'static> Entry<S, G> {
    pub fn identity(&self) -> &ServerIdentity {
        &self.identity
    }

    pub fn features(&self) -> Features {
        self.server.features()
    }

    /// The key of a question about the whole tree; none on the live tree, which
    /// changes under the server.
    fn project(&self, query: ProjectQuery) -> Option<Key> {
        self.basis.map(|basis| project_key(&self.identity, basis, query))
    }

    /// The answer under `key`, or the server's. The server's answer is kept only if
    /// it was not at work on its own when it was asked, and did not begin work before
    /// it answered: an answer given while it indexes may be a partial one. The
    /// question is asked by a flight that outlives the caller that began it, so
    /// `ask` gets the entry and owns what it asks with.
    async fn reuse<T, F>(
        self: &Arc<Self>,
        key: Option<Key>,
        ask: impl FnOnce(Arc<Self>) -> F + Send + 'static,
    ) -> Result<Answered<T>, QueryError>
    where
        T: Reused + Send,
        F: Future<Output = Result<T, QueryError>> + Send + 'static,
    {
        let entry = self.clone();
        let asked = move || async move {
            let _turn = entry.requests.acquire().await;
            let before = entry.server.readiness();
            let value = match ask(entry.clone()).await {
                Ok(value) => value,
                Err(_) if entry.stopped.load(Ordering::SeqCst) => {
                    return Err(QueryError("the language server was restarted".into()));
                }
                Err(e) => return Err(e),
            };
            let keep = before.idle && before.epoch == entry.server.readiness().epoch;
            Ok(Computed { value, keep })
        };
        let (value, from_cache) = self.answers.get(key.as_ref(), asked).await?;
        Ok(answered(value, self.identity.clone(), from_cache))
    }

    /// Without a text the symbols are those of the file as the tree has it: only a tree that never changes has them under a key.
    pub async fn document_symbols(self: &Arc<Self>, file: &RepoPath) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        let key = self.project(ProjectQuery::DocumentSymbols(file.clone()));
        let file = file.clone();
        self.reuse(
            key,
            move |this| async move { this.server.document_symbols(&file).await },
        )
        .await
    }

    /// The symbols `text` declares as the content of `file`. They depend on the text
    /// and the language alone, so any tree has them under one key.
    pub async fn text_symbols(
        self: &Arc<Self>,
        file: &RepoPath,
        text: &str,
    ) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        let key = text_key(&self.identity, file, text);
        let (asked_file, text) = (file.clone(), text.to_owned());
        let mut answered = self
            .reuse(Some(key), move |this| async move {
                this.server.text_symbols(&asked_file, &text).await
            })
            .await?;
        named_after(file, &mut answered.value);
        Ok(answered)
    }

    pub async fn definition(self: &Arc<Self>, at: &At) -> Result<Answered<Vec<Location>>, QueryError> {
        let key = self.project(ProjectQuery::Definition(at.clone()));
        let at = at.clone();
        self.reuse(key, move |this| async move { this.server.definition(&at).await })
            .await
    }

    pub async fn references(self: &Arc<Self>, at: &At) -> Result<Answered<Vec<Location>>, QueryError> {
        let key = self.project(ProjectQuery::References(at.clone()));
        let at = at.clone();
        self.reuse(key, move |this| async move { this.server.references(&at).await })
            .await
    }

    pub async fn implementations(self: &Arc<Self>, at: &At) -> Result<Answered<Vec<Location>>, QueryError> {
        let key = self.project(ProjectQuery::Implementations(at.clone()));
        let at = at.clone();
        self.reuse(key, move |this| async move { this.server.implementations(&at).await })
            .await
    }

    pub async fn hover(self: &Arc<Self>, at: &At) -> Result<Answered<Option<String>>, QueryError> {
        let key = self.project(ProjectQuery::Hover(at.clone()));
        let at = at.clone();
        self.reuse(key, move |this| async move { this.server.hover(&at).await })
            .await
    }

    pub async fn incoming_calls(self: &Arc<Self>, at: &At) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        let key = self.project(ProjectQuery::Incoming(at.clone()));
        let at = at.clone();
        self.reuse(key, move |this| async move { this.server.incoming_calls(&at).await })
            .await
    }

    pub async fn outgoing_calls(self: &Arc<Self>, at: &At) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        let key = self.project(ProjectQuery::Outgoing(at.clone()));
        let at = at.clone();
        self.reuse(key, move |this| async move { this.server.outgoing_calls(&at).await })
            .await
    }

    pub async fn workspace_symbols(self: &Arc<Self>, query: &str) -> Result<Answered<Vec<SymbolInfo>>, QueryError> {
        let key = self.project(ProjectQuery::Workspace(query.to_owned()));
        let query = query.to_owned();
        self.reuse(
            key,
            move |this| async move { this.server.workspace_symbols(&query).await },
        )
        .await
    }

    pub async fn diagnostics(self: &Arc<Self>, file: &RepoPath) -> Result<Answered<Vec<Diagnostic>>, QueryError> {
        let key = self.project(ProjectQuery::Diagnostics(file.clone()));
        let file = file.clone();
        self.reuse(key, move |this| async move { this.server.diagnostics(&file).await })
            .await
    }

    async fn shutdown(&self) {
        if !self.stopped.swap(true, Ordering::SeqCst) {
            self.server.shutdown().await;
        }
    }
}

struct State<S, G> {
    tick: u64,
    /// Bumped by `restart`; an entry from an older generation is not pooled.
    generations: HashMap<BackendId, u64>,
    slots: HashMap<PoolKey, Arc<Entry<S, G>>>,
    /// Of the keys whose server died; a manual restart of the backend forgets them.
    health: HashMap<PoolKey, Health>,
    /// What a backend's server offers, as of the server started last; not known before one has
    /// started.
    features: HashMap<BackendId, Features>,
}

/// A server being started: the key of the flight that starts it. Callers of
/// different generations of a backend do not share a start, because the server
/// of the older one does not stay.
struct Start<M, L> {
    key: PoolKey,
    generation: u64,
    of: PhantomData<fn() -> (M, L)>,
}

impl<M, L> Start<M, L> {
    fn new(key: PoolKey, generation: u64) -> Self {
        Self {
            key,
            generation,
            of: PhantomData,
        }
    }
}

impl<M, L> Clone for Start<M, L> {
    fn clone(&self) -> Self {
        Self::new(self.key.clone(), self.generation)
    }
}

impl<M, L> PartialEq for Start<M, L> {
    fn eq(&self, other: &Self) -> bool {
        (&self.key, self.generation) == (&other.key, other.generation)
    }
}

impl<M, L> Eq for Start<M, L> {}

impl<M, L> Hash for Start<M, L> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.generation.hash(state);
    }
}

/// Every start is shared while it goes on and forgotten when it ended: the server
/// it started is the pool's, and a caller that comes later finds it there.
impl<M: Materializer, L: ServerLauncher> Kind for Start<M, L> {
    type Value = Lease<M, L>;
    type Error = PoolError;

    fn retention(&self) -> Retention {
        Retention::InFlight
    }
}

/// What a server for a key would be, observed without starting one, by the
/// directory it would run in. Sampled once per generation of the key's backend:
/// a restart makes a new generation, so nothing observed before it is reused.
struct Foreseen<W> {
    key: PoolKey,
    root: PathBuf,
    generation: u64,
    of: PhantomData<fn() -> W>,
}

impl<W> Foreseen<W> {
    fn new(key: PoolKey, root: PathBuf, generation: u64) -> Self {
        Self {
            key,
            root,
            generation,
            of: PhantomData,
        }
    }
}

impl<W> Clone for Foreseen<W> {
    fn clone(&self) -> Self {
        Self::new(self.key.clone(), self.root.clone(), self.generation)
    }
}

impl<W> PartialEq for Foreseen<W> {
    fn eq(&self, other: &Self) -> bool {
        (&self.key, &self.root, self.generation) == (&other.key, &other.root, other.generation)
    }
}

impl<W> Eq for Foreseen<W> {}

impl<W> Hash for Foreseen<W> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
        self.root.hash(state);
        self.generation.hash(state);
    }
}

/// What is observed: the identity of the server, or the basis of its kept tree.
trait Observed: 'static {
    type Value: Clone + Send + 'static;
}

/// The identity a server for the key would have.
struct Identity;

/// The basis of the kept tree of a snapshot, from the restore recorded for it.
struct Recorded;

impl Observed for Identity {
    type Value = ServerIdentity;
}

impl Observed for Recorded {
    type Value = Basis;
}

/// An observation that fails (the tree has no usable record) is not kept and is made again.
impl<W: Observed> Kind for Foreseen<W> {
    type Value = W::Value;
    type Error = ();

    fn retention(&self) -> Retention {
        Retention::Sampled
    }

    fn generation(&self) -> Option<(Self, u64)> {
        Some((Self::new(self.key.clone(), self.root.clone(), 0), self.generation))
    }
}

struct Inner<M: Materializer, L: ServerLauncher> {
    materializer: M,
    launcher: L,
    live_root: PathBuf,
    capacity: usize,
    /// How many answers `answers` keeps in memory.
    limit: usize,
    store: Option<Arc<dyn SemanticCache>>,
    answers: Arc<Answers>,
    /// What `foreseen` observed.
    identities: Engine<Foreseen<Identity>>,
    bases: Engine<Foreseen<Recorded>>,
    /// Numbers the servers started on trees whose restore left no record.
    instances: AtomicU64,
    state: Mutex<State<L::Server, M::Guard>>,
    starts: Engine<Start<M, L>>,
    supervision: Option<Supervision>,
}

pub struct Pool<M: Materializer, L: ServerLauncher> {
    inner: Arc<Inner<M, L>>,
}

/// Identities and bases kept.
const FORESEEN_KEPT: usize = 64;

type EntryOf<M, L> = Entry<<L as ServerLauncher>::Server, <M as Materializer>::Guard>;

type Entries<M, L> = Vec<Arc<EntryOf<M, L>>>;

/// A claim on a shared server. Ends on `release` or drop. A claim is a lease of
/// its own when cloned.
pub struct Lease<M: Materializer, L: ServerLauncher> {
    pool: Arc<Inner<M, L>>,
    entry: Option<Arc<EntryOf<M, L>>>,
}

impl<M: Materializer, L: ServerLauncher> Deref for Lease<M, L> {
    type Target = Arc<EntryOf<M, L>>;

    fn deref(&self) -> &Self::Target {
        self.entry.as_ref().expect("lease is live until released")
    }
}

impl<M: Materializer, L: ServerLauncher> Clone for Lease<M, L> {
    fn clone(&self) -> Self {
        let entry = self.entry.clone().expect("lease is live until released");
        entry.leases.fetch_add(1, Ordering::SeqCst);
        Self {
            pool: self.pool.clone(),
            entry: Some(entry),
        }
    }
}

impl<M: Materializer, L: ServerLauncher> Drop for Lease<M, L> {
    fn drop(&mut self) {
        if let Some(entry) = self.entry.take() {
            // Not awaited: a retired server is just dropped, which kills it. Evicting waits
            // for a release, which stops the evicted servers cleanly: a lease is also dropped
            // when a flight's copy of it goes, while the callers' own are yet to be released.
            drop(self.pool.end_lease(entry, false));
        }
    }
}

impl<M: Materializer, L: ServerLauncher> Lease<M, L> {
    /// End the lease, shutting the server down cleanly if this was the last
    /// lease of a restarted or evicted server.
    pub async fn release(mut self) {
        if let Some(entry) = self.entry.take() {
            let stop = self.pool.end_lease(entry, true);
            Inner::<M, L>::stop(stop).await;
        }
    }
}

impl<M: Materializer, L: ServerLauncher> Pool<M, L> {
    /// `capacity` is the number of servers kept warm; leased ones never count against eviction.
    pub fn new(materializer: M, launcher: L, live_root: PathBuf, capacity: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                materializer,
                launcher,
                live_root,
                capacity,
                limit: KEPT,
                store: None,
                answers: Arc::new(Answers::new(KEPT)),
                identities: Engine::new(Limits::entries(FORESEEN_KEPT)),
                bases: Engine::new(Limits::entries(FORESEEN_KEPT)),
                instances: AtomicU64::new(0),
                state: Mutex::new(State {
                    tick: 0,
                    generations: HashMap::new(),
                    slots: HashMap::new(),
                    health: HashMap::new(),
                    features: HashMap::new(),
                }),
                starts: Engine::new(Limits::entries(0)),
                supervision: None,
            }),
        }
    }

    /// Watches the servers it starts: one that dies while running is started anew once,
    /// and `supervision` is told what became of it.
    pub fn supervised(mut self, supervision: Supervision) -> Self {
        self.unshared().supervision = Some(supervision);
        self
    }

    /// The pool before it is shared: configured by the builders.
    fn unshared(&mut self) -> &mut Inner<M, L> {
        Arc::get_mut(&mut self.inner).expect("a pool is configured before it is used")
    }

    /// Keeps at most `limit` answers in memory; 0 keeps none, so every question
    /// reaches a server unless `store` has the answer.
    pub fn keeping(mut self, limit: usize) -> Self {
        let inner = self.unshared();
        inner.limit = limit;
        inner.answers = Arc::new(Answers::new(limit).persisting(inner.store.clone()));
        self
    }

    /// Keeps the answers that persist in `store`, behind the ones in memory.
    pub fn persisting(mut self, store: Arc<dyn SemanticCache>) -> Self {
        let inner = self.unshared();
        inner.store = Some(store);
        inner.answers = Arc::new(Answers::new(inner.limit).persisting(inner.store.clone()));
        self
    }

    /// Drops the answers kept in memory; the stored ones are the store's.
    pub fn forget_answers(&self) {
        self.inner.answers.forget();
    }
}

impl<M: Materializer, L: ServerLauncher> Inner<M, L> {
    /// What a server for `key` would be, as far as the keys of its answers go, worked
    /// out without starting one: its identity and, on an immutable tree, the basis, from
    /// the restore the tree's record stands for. Nothing is materialized: a snapshot
    /// with no kept tree has no such server yet. The basis is none on the live tree and
    /// for a tree with no usable record; the answers about the whole tree have no key then.
    async fn foreseen(self: &Arc<Self>, key: &PoolKey, backend: &Backend) -> Option<(ServerIdentity, Option<Basis>)> {
        let (root, snapshot) = self.foreseen_root(key, backend).await?;
        let identity = self.foreseen_identity(key, backend, &root).await?;
        let basis = match snapshot {
            Some(snapshot) => self.foreseen_basis(key, backend, &root, snapshot).await,
            None => None,
        };
        Some((identity, basis))
    }

    /// Where a server for `key` would run, and the snapshot it would serve; none if
    /// the backend is not admitted or the snapshot has no kept tree.
    async fn foreseen_root(
        self: &Arc<Self>,
        key: &PoolKey,
        backend: &Backend,
    ) -> Option<(PathBuf, Option<SnapshotId>)> {
        admit(backend, key.policy).ok()?;
        Some(match key.target {
            Target::Live => (self.live_root.clone(), None),
            Target::Snapshot(snapshot) => (self.materializer.kept(snapshot, key.policy).await?, Some(snapshot)),
        })
    }

    /// The identity a server for `key` would have, worked out once per backend generation
    /// and root: the launcher probes toolchains, reads settings and hashes binaries for it.
    async fn foreseen_identity(
        self: &Arc<Self>,
        key: &PoolKey,
        backend: &Backend,
        root: &Path,
    ) -> Option<ServerIdentity> {
        let known = Foreseen::new(key.clone(), root.to_owned(), self.generation(&key.backend));
        let (this, root, backend, policy) = (self.clone(), root.to_owned(), backend.clone(), key.policy);
        let worked_out = self
            .identities
            .get(known, move |_| async move {
                let identity = this.launcher.identify(&root, &backend, policy).await.map_err(drop)?;
                Ok(engine::Computed::new(identity, true))
            })
            .await;
        worked_out.ok().map(|got| got.value)
    }

    /// The basis of the kept tree of `snapshot` from the restore recorded for it, worked
    /// out once per backend generation and root. A tree with no usable record has none
    /// and is looked at again; every start drops what was found, because its restore
    /// may have recorded one.
    async fn foreseen_basis(
        self: &Arc<Self>,
        key: &PoolKey,
        backend: &Backend,
        root: &Path,
        snapshot: SnapshotId,
    ) -> Option<Basis> {
        let known = Foreseen::new(key.clone(), root.to_owned(), self.generation(&key.backend));
        let (this, root, backend, policy) = (self.clone(), root.to_owned(), backend.clone(), key.policy);
        let recorded = self
            .bases
            .get(known, move |_| async move {
                let fingerprint = this
                    .launcher
                    .recorded_restore(&root, &backend, policy)
                    .await
                    .ok_or(())?;
                Ok(engine::Computed::new(
                    Basis {
                        snapshot,
                        policy,
                        restore: Restore::Recorded(fingerprint),
                    },
                    true,
                ))
            })
            .await;
        recorded.ok().map(|got| got.value)
    }

    /// How often `backend` was restarted.
    fn generation(&self, backend: &BackendId) -> u64 {
        self.lock().generations.get(backend).copied().unwrap_or(0)
    }

    /// A lease on the server for `key` if one is running.
    fn join(self: &Arc<Self>, key: &PoolKey) -> Option<Lease<M, L>> {
        let mut state = self.lock();
        state.tick += 1;
        let tick = state.tick;
        let entry = state.slots.get(key)?;
        entry.leases.fetch_add(1, Ordering::SeqCst);
        entry.used.store(tick, Ordering::SeqCst);
        Some(Lease {
            pool: self.clone(),
            entry: Some(entry.clone()),
        })
    }

    /// Starts the server for `key`, unless one is running by the time this is
    /// polled, and leases it. It runs as a flight, so its result is every waiter's.
    async fn start(
        self: Arc<Self>,
        key: PoolKey,
        backend: Backend,
        generation: u64,
    ) -> Result<engine::Computed<Lease<M, L>>, PoolError> {
        if let Some(lease) = self.join(&key) {
            return Ok(engine::Computed::new(lease, false));
        }
        let (Started { server, identity, .. }, basis, dir_guard) = self.launch(&key, &backend).await?;
        let entry = {
            let mut state = self.lock();
            let stale = state.generations.get(&key.backend).copied().unwrap_or(0) != generation;
            state.tick += 1;
            if !stale {
                state.features.insert(key.backend.clone(), server.features());
            }
            let entry = Arc::new(Entry {
                server,
                key: key.clone(),
                backend,
                identity,
                answers: self.answers.clone(),
                basis,
                leases: AtomicUsize::new(1),
                used: AtomicU64::new(state.tick),
                retired: AtomicBool::new(stale),
                stopped: AtomicBool::new(false),
                requests: Gate::new(REQUESTS_PER_SERVER),
                _guard: dir_guard,
            });
            if !stale {
                state.slots.insert(key, entry.clone());
            }
            entry
        };
        self.bases.clear();
        self.watch(&entry);
        Ok(engine::Computed::new(
            Lease {
                pool: self,
                entry: Some(entry),
            },
            false,
        ))
    }

    /// Has `entry` handled when its server dies. The watching holds neither the pool nor the
    /// entry, so it does not keep a server alive that nobody wants.
    fn watch(self: &Arc<Self>, entry: &Arc<EntryOf<M, L>>) {
        let Some(supervision) = &self.supervision else { return };
        let (exited, pool, entry) = (entry.server.exited(), Arc::downgrade(self), Arc::downgrade(entry));
        supervision.detach.detach(Box::pin(async move {
            let why = exited.await;
            if let (Some(pool), Some(entry)) = (pool.upgrade(), entry.upgrade()) {
                pool.died(entry, why).await;
            }
        }));
    }

    /// The server of `entry` is gone. Unless that was asked for, it is started anew, and the
    /// callers that come meanwhile wait for that start as for any. A server that dies again
    /// within [`RESTART_WINDOW`] of the start having finished, or cannot be started anew, stays
    /// stopped: the leases of its key fail until the backend is restarted by hand.
    async fn died(self: &Arc<Self>, entry: Arc<EntryOf<M, L>>, why: String) {
        if entry.stopped.load(Ordering::SeqCst) || entry.retired.load(Ordering::SeqCst) {
            return;
        }
        let Some(supervision) = &self.supervision else { return };
        let key = &entry.key;
        let (generation, again) = {
            let mut state = self.lock();
            match state.slots.get(key) {
                Some(pooled) if Arc::ptr_eq(pooled, &entry) => {
                    state.slots.remove(key);
                }
                _ => return,
            }
            let generation = state.generations.get(&key.backend).copied().unwrap_or(0);
            let again = matches!(state.health.get(key), Some(Health::Restarted(at)) if at.elapsed() < RESTART_WINDOW);
            if again {
                state.health.insert(key.clone(), Health::Stopped(why.clone()));
            }
            (generation, again)
        };
        let outcome = if again {
            Outcome::Stopped
        } else {
            match self.lease(key, &entry.backend).await {
                Ok(lease) => {
                    self.remember(key, generation, Health::Restarted(Instant::now()));
                    lease.release().await;
                    Outcome::Restarted
                }
                Err(e) => {
                    self.remember(key, generation, Health::Stopped(format!("{why}\n{e}")));
                    Outcome::Stopped
                }
            }
        };
        (supervision.told)(&key.backend, outcome);
    }

    /// Notes `health` of `key`, unless the backend was restarted by hand since `generation`.
    fn remember(&self, key: &PoolKey, generation: u64, health: Health) {
        let mut state = self.lock();
        if state.generations.get(&key.backend).copied().unwrap_or(0) == generation {
            state.health.insert(key.clone(), health);
        }
    }

    /// The server for `key`: the running one, the one being started, or a new one.
    async fn lease(self: &Arc<Self>, key: &PoolKey, backend: &Backend) -> Result<Lease<M, L>, PoolError> {
        admit(backend, key.policy)?;
        if let Some(Health::Stopped(why)) = self.lock().health.get(key) {
            return Err(PoolError::Stopped(why.clone()));
        }
        if let Some(lease) = self.join(key) {
            return Ok(lease);
        }
        let generation = self.generation(&key.backend);
        let (inner, started, backend) = (self.clone(), key.clone(), backend.clone());
        let lease = self
            .starts
            .get(Start::new(key.clone(), generation), move |_| {
                inner.start(started, backend, generation)
            })
            .await?;
        Ok(lease.value)
    }
}

impl<M: Materializer, L: ServerLauncher> Pool<M, L> {
    /// The answer to a question about the whole tree of `key` if it is kept, in memory or
    /// in the store, found without a server (I2): the key is worked out as a start
    /// would, from the identity of the server and the restore recorded for the tree.
    /// `None` means only that a server is to be asked.
    pub async fn recall<T: Reused>(
        &self,
        key: &PoolKey,
        backend: &Backend,
        query: ProjectQuery,
    ) -> Option<Answered<T>> {
        let (identity, basis) = self.inner.foreseen(key, backend).await?;
        let value = self
            .inner
            .answers
            .recall(&project_key(&identity, basis?, query))
            .await?;
        Some(answered(value, identity, true))
    }

    /// Like [`Pool::recall`], for the symbols `text` declares as the content of `file`.
    pub async fn recall_text_symbols(
        &self,
        key: &PoolKey,
        backend: &Backend,
        file: &RepoPath,
        text: &str,
    ) -> Option<Answered<Vec<SymbolInfo>>> {
        let (root, _) = self.inner.foreseen_root(key, backend).await?;
        let identity = self.inner.foreseen_identity(key, backend, &root).await?;
        let value: Vec<SymbolInfo> = self.inner.answers.recall(&text_key(&identity, file, text)).await?;
        let mut recalled = answered(value, identity, true);
        named_after(file, &mut recalled.value);
        Some(recalled)
    }

    /// The server for `key`: the running one, the one being started, or a new one.
    /// Callers that arrive while it starts join that start, whichever of them
    /// began it. A start is not shared across a restart of the backend, because
    /// the server of the older generation does not stay.
    pub async fn lease(&self, key: &PoolKey, backend: &Backend) -> Result<Lease<M, L>, PoolError> {
        self.inner.lease(key, backend).await
    }

    /// Drop every server of `backend`: its settings changed (§7.2). Servers
    /// leased right now are stopped when their last lease ends.
    pub async fn restart(&self, backend: &BackendId) {
        let (stop, _) = self.retire(backend);
        Inner::<M, L>::stop(stop).await;
    }

    /// Drop every server of `backend`, those leased right now too: a person asked for it, and
    /// a server that hangs would hold its leases for as long as its requests take to time out.
    /// The requests in flight on a server stopped so fail as asked of a server that was restarted.
    pub async fn restart_now(&self, backend: &BackendId) {
        let (mut stop, leased) = self.retire(backend);
        stop.extend(leased);
        Inner::<M, L>::stop(stop).await;
    }

    /// Takes the servers of `backend` out of the pool, for a new generation to replace; returns
    /// the ones nobody leases, to stop now, and the leased ones, which stop with their last lease.
    fn retire(&self, backend: &BackendId) -> (Entries<M, L>, Entries<M, L>) {
        let (idle, leased) = {
            let mut state = self.inner.lock();
            *state.generations.entry(backend.clone()).or_default() += 1;
            state.features.remove(backend);
            state.health.retain(|key, _| &key.backend != backend);
            let keys: Vec<_> = state.slots.keys().filter(|k| &k.backend == backend).cloned().collect();
            let (mut idle, mut leased) = (Vec::new(), Vec::new());
            for key in keys {
                if let Some(e) = state.slots.remove(&key) {
                    e.retired.store(true, Ordering::SeqCst);
                    if e.leases.load(Ordering::SeqCst) == 0 {
                        idle.push(e);
                    } else {
                        leased.push(e);
                    }
                }
            }
            (idle, leased)
        };
        self.inner.launcher.restarted(backend);
        (idle, leased)
    }

    /// The operations the server of `backend` offers; `None` before one has started.
    pub fn features(&self, backend: &BackendId) -> Option<Features> {
        self.inner.lock().features.get(backend).copied()
    }

    /// Stop every server that is not leased.
    pub async fn shutdown(&self) {
        let stop = {
            let mut state = self.inner.lock();
            let keys: Vec<_> = state
                .slots
                .iter()
                .filter(|(_, e)| e.leases.load(Ordering::SeqCst) == 0)
                .map(|(k, _)| k.clone())
                .collect();
            keys.iter().filter_map(|k| state.slots.remove(k)).collect()
        };
        Inner::<M, L>::stop(stop).await;
    }

    #[cfg(test)]
    pub fn keys(&self) -> Vec<PoolKey> {
        self.inner.lock().slots.keys().cloned().collect()
    }
}

impl<M: Materializer, L: ServerLauncher> Inner<M, L> {
    async fn launch(
        &self,
        key: &PoolKey,
        backend: &Backend,
    ) -> Result<(Started<L::Server>, Option<Basis>, Option<M::Guard>), PoolError> {
        let (root, tree, guard) = match key.target {
            Target::Live => (self.live_root.clone(), Tree::Live, None),
            Target::Snapshot(id) => {
                let m = self
                    .materializer
                    .materialize(id, key.policy)
                    .await
                    .map_err(PoolError::NotMaterializable)?;
                (m.root, Tree::Immutable, Some(m.guard))
            }
        };
        let started = self
            .launcher
            .start(&root, tree, backend, key.policy)
            .await
            .map_err(PoolError::Launch)?;
        let basis = match key.target {
            Target::Live => None,
            Target::Snapshot(snapshot) => Some(Basis {
                snapshot,
                policy: key.policy,
                restore: match started.restore {
                    Some(fingerprint) => Restore::Recorded(fingerprint),
                    None => Restore::Instance(self.instances.fetch_add(1, Ordering::SeqCst)),
                },
            }),
        };
        Ok((started, basis, guard))
    }

    /// Drops one lease; returns the entries to stop (this one if it was
    /// retired, plus, if `evict`, whatever exceeds capacity).
    fn end_lease(&self, entry: Arc<EntryOf<M, L>>, evict: bool) -> Vec<Arc<EntryOf<M, L>>> {
        let mut state = self.lock();
        let left = entry.leases.fetch_sub(1, Ordering::SeqCst) - 1;
        let mut stop = Vec::new();
        if left == 0 && entry.retired.load(Ordering::SeqCst) {
            stop.push(entry);
        }
        while evict && state.slots.len() > self.capacity {
            let oldest = state
                .slots
                .iter()
                .filter(|(_, e)| e.leases.load(Ordering::SeqCst) == 0)
                .map(|(k, e)| (k, e.used.load(Ordering::SeqCst)))
                .min_by_key(|(_, used)| *used)
                .map(|(k, _)| k.clone());
            match oldest.and_then(|k| state.slots.remove(&k)) {
                Some(e) => stop.push(e),
                None => break,
            }
        }
        stop
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
    use std::sync::atomic::{AtomicU8, AtomicUsize};
    use std::task::{Context, Waker};

    use vet_core::content::Hash;

    use super::*;
    use crate::join::{bounded, join};
    use crate::policy::AnalysisPolicy;
    use crate::ports::{Feature, Materialized, Pos, Readiness, ServerConfig, Severity, Span, Sym, SymbolKind};
    use crate::reuse::Value;
    use crate::testutil::{block_on, yield_once};

    fn backend(executes: bool) -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId("rust".into()),
            binary: "rust-analyzer".into(),
            args: vec![],
            filetypes: vec!["rs".into()],
            file_names: vec![],
            requires: vec![],
            root_markers: vec!["Cargo.toml".into()],
            executes_project_code_during_analysis: executes,
            loads_project_on_open: false,
        })
    }

    #[test]
    fn untrusted_policy_refuses_executing_backends_only() {
        let untrusted = AnalysisPolicy::LIMITED;
        let trusted = AnalysisPolicy::EXECUTING;
        assert!(matches!(
            admit(&backend(true), untrusted),
            Err(PoolError::AnalysisLimited(_))
        ));
        assert!(admit(&backend(false), untrusted).is_ok());
        assert!(admit(&backend(true), trusted).is_ok());
    }

    struct Counters {
        materialized: AtomicUsize,
        guards_dropped: AtomicUsize,
        started: AtomicUsize,
        shut_down: AtomicUsize,
        hover_calls: AtomicUsize,
        fail_start: AtomicBool,
        /// A start goes on only when this is false.
        hold_start: AtomicBool,
        /// The version of the server binary: part of its identity and of what it answers.
        version: AtomicU8,
        /// The fingerprint of the restore of an immutable tree; 0: none was recorded.
        restore: AtomicU8,
        /// The kinds of the queries that reached a server, in order.
        asked: Mutex<Vec<&'static str>>,
        /// The server is at work on its own.
        busy: AtomicBool,
        /// The server begins work while it answers.
        begins_work: AtomicBool,
        epoch: AtomicU64,
        failing: AtomicBool,
        /// A query is not answered until the server is shut down, which fails it.
        hang: AtomicBool,
        /// Queries and observations of the launcher take a few turns of the executor.
        slow: AtomicBool,
        /// Identities and recorded restores observed, by starts too.
        identified: AtomicUsize,
        recorded: AtomicUsize,
        /// Requests being answered by the server now, and the most there were at once.
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        /// The operations a started server does not offer.
        lacking: Mutex<Vec<Feature>>,
    }

    impl Default for Counters {
        fn default() -> Self {
            Self {
                materialized: AtomicUsize::new(0),
                guards_dropped: AtomicUsize::new(0),
                started: AtomicUsize::new(0),
                shut_down: AtomicUsize::new(0),
                hover_calls: AtomicUsize::new(0),
                fail_start: AtomicBool::new(false),
                hold_start: AtomicBool::new(false),
                version: AtomicU8::new(1),
                restore: AtomicU8::new(5),
                asked: Mutex::default(),
                busy: AtomicBool::new(false),
                begins_work: AtomicBool::new(false),
                epoch: AtomicU64::new(0),
                failing: AtomicBool::new(false),
                hang: AtomicBool::new(false),
                slow: AtomicBool::new(false),
                identified: AtomicUsize::new(0),
                recorded: AtomicUsize::new(0),
                in_flight: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                lacking: Mutex::default(),
            }
        }
    }

    impl Counters {
        /// The kinds of the queries that reached a server since the last call.
        fn asked(&self) -> Vec<&'static str> {
            std::mem::take(&mut *self.asked.lock().unwrap())
        }

        async fn take_turns(&self) {
            if self.slow.load(Ordering::SeqCst) {
                for _ in 0..3 {
                    yield_once().await;
                }
            }
        }
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

        /// A tree stays after it was made, whatever pool made it.
        async fn kept(&self, _: SnapshotId, _: AnalysisPolicy) -> Option<PathBuf> {
            (self.0.materialized.load(Ordering::SeqCst) > 0).then(|| PathBuf::from("/snap"))
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
            tree: Tree,
            backend: &Backend,
            policy: AnalysisPolicy,
        ) -> Result<Started<FakeServer>, LaunchError> {
            self.0.started.fetch_add(1, Ordering::SeqCst);
            yield_once().await;
            while self.0.hold_start.load(Ordering::SeqCst) {
                yield_once().await;
            }
            if self.0.fail_start.load(Ordering::SeqCst) {
                return Err(LaunchError::Spawn("boom".into()));
            }
            let restore = match tree {
                Tree::Immutable => self.recorded_restore(root, backend, policy).await,
                Tree::Live => None,
            };
            Ok(Started {
                server: FakeServer(self.0.clone()),
                identity: self.identify(root, backend, policy).await?,
                restore,
            })
        }

        /// What the tree's record says; 0: it has none.
        async fn recorded_restore(&self, _: &std::path::Path, _: &Backend, _: AnalysisPolicy) -> Option<Hash> {
            self.0.recorded.fetch_add(1, Ordering::SeqCst);
            self.0.take_turns().await;
            match self.0.restore.load(Ordering::SeqCst) {
                0 => None,
                n => Some(Hash([n; 32])),
            }
        }

        /// A binary that is replaced between two starts: the version it has when the server starts.
        async fn identify(
            &self,
            _: &std::path::Path,
            backend: &Backend,
            _: AnalysisPolicy,
        ) -> Result<ServerIdentity, LaunchError> {
            self.0.identified.fetch_add(1, Ordering::SeqCst);
            self.0.take_turns().await;
            let version = self.0.version.load(Ordering::SeqCst);
            Ok(ServerIdentity {
                backend: backend.config().backend.clone(),
                fingerprint: Hash([version; 32]),
                label: format!("fake {version}"),
            })
        }
    }

    fn span(at: Pos, version: u8) -> Span {
        Span {
            start: at,
            end: Pos {
                line: at.line,
                col: at.col + u32::from(version),
            },
        }
    }

    fn symbol(file: &RepoPath, name: String, at: Pos, version: u8) -> SymbolInfo {
        SymbolInfo {
            sym: Sym {
                file: file.clone(),
                name: vec![name],
            },
            span: span(at, version),
            at,
            detail: None,
            kind: SymbolKind::Other,
            modules: 0,
        }
    }

    /// Answers are functions of what is asked and of the version of the server.
    impl FakeServer {
        async fn asked(&self, kind: &'static str) -> Result<u8, QueryError> {
            self.0.asked.lock().unwrap().push(kind);
            let in_flight = self.0.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.0.peak.fetch_max(in_flight, Ordering::SeqCst);
            self.0.take_turns().await;
            if self.0.hang.load(Ordering::SeqCst) {
                while self.0.shut_down.load(Ordering::SeqCst) == 0 {
                    yield_once().await;
                }
                self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
                return Err(QueryError("server exited".into()));
            }
            self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
            if self.0.begins_work.load(Ordering::SeqCst) {
                self.0.epoch.fetch_add(1, Ordering::SeqCst);
            }
            if self.0.failing.load(Ordering::SeqCst) {
                return Err(QueryError("boom".into()));
            }
            Ok(self.0.version.load(Ordering::SeqCst))
        }
    }

    impl LanguageServer for FakeServer {
        fn readiness(&self) -> Readiness {
            Readiness {
                idle: !self.0.busy.load(Ordering::SeqCst),
                epoch: self.0.epoch.load(Ordering::SeqCst),
            }
        }
        fn features(&self) -> Features {
            let lacking = self.0.lacking.lock().unwrap();
            lacking.iter().fold(Features::ALL, |all, f| all.with(*f, false))
        }
        async fn document_symbols(&self, file: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
            let version = self.asked("document_symbols").await?;
            Ok(vec![symbol(file, "on disk".into(), Pos { line: 0, col: 0 }, version)])
        }
        async fn text_symbols(&self, file: &RepoPath, text: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            let version = self.asked("text_symbols").await?;
            Ok(vec![symbol(file, text.to_owned(), Pos { line: 0, col: 0 }, version)])
        }
        async fn definition(&self, at: &At) -> Result<Vec<Location>, QueryError> {
            let version = self.asked("definition").await?;
            Ok(vec![Location {
                file: at.file.clone(),
                span: span(at.pos, version),
                external: false,
            }])
        }
        async fn references(&self, at: &At) -> Result<Vec<Location>, QueryError> {
            let version = self.asked("references").await?;
            Ok(vec![Location {
                file: at.file.clone(),
                span: span(at.pos, version),
                external: false,
            }])
        }
        async fn implementations(&self, at: &At) -> Result<Vec<Location>, QueryError> {
            let version = self.asked("implementations").await?;
            Ok(vec![Location {
                file: at.file.clone(),
                span: span(at.pos, version),
                external: false,
            }])
        }
        async fn hover(&self, at: &At) -> Result<Option<String>, QueryError> {
            let version = self.asked("hover").await?;
            self.0.hover_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(format!("{version} {}:{}", at.pos.line, at.pos.col)))
        }
        async fn incoming_calls(&self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            let version = self.asked("incoming_calls").await?;
            Ok(vec![symbol(&at.file, "caller".into(), at.pos, version)])
        }
        async fn outgoing_calls(&self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            let version = self.asked("outgoing_calls").await?;
            Ok(vec![symbol(&at.file, "callee".into(), at.pos, version)])
        }
        async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            let version = self.asked("workspace_symbols").await?;
            Ok(vec![symbol(
                &RepoPath(b"w.rs".to_vec()),
                query.to_owned(),
                Pos { line: 0, col: 0 },
                version,
            )])
        }
        async fn diagnostics(&self, file: &RepoPath) -> Result<Vec<Diagnostic>, QueryError> {
            let version = self.asked("diagnostics").await?;
            Ok(vec![Diagnostic {
                span: span(Pos { line: 0, col: 0 }, version),
                severity: Severity::Error,
                message: file.to_string_lossy(),
                source: None,
                code: None,
            }])
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
            policy: AnalysisPolicy::EXECUTING,
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

    fn file(name: &str) -> RepoPath {
        RepoPath(name.as_bytes().to_vec())
    }

    type TestPool = Pool<FakeMaterializer, FakeLauncher>;

    /// Every input of the cached kinds of query.
    #[derive(Clone)]
    struct Ask {
        file: RepoPath,
        at: At,
        text: &'static str,
        query: &'static str,
    }

    fn ask() -> Ask {
        Ask {
            file: file("a.rs"),
            at: at(),
            text: "fn a() {}",
            query: "a",
        }
    }

    /// The kinds of query in the order `ask_all` asks them, and those that depend on
    /// a whole tree.
    const ALL: [&str; 10] = [
        "document_symbols",
        "text_symbols",
        "definition",
        "references",
        "implementations",
        "hover",
        "incoming_calls",
        "outgoing_calls",
        "workspace_symbols",
        "diagnostics",
    ];
    const WHOLE_TREE: [&str; 9] = [
        "document_symbols",
        "definition",
        "references",
        "implementations",
        "hover",
        "incoming_calls",
        "outgoing_calls",
        "workspace_symbols",
        "diagnostics",
    ];

    /// Asks every cached kind of query: the answers, each with whether it was reused.
    async fn ask_all(lease: &Lease<FakeMaterializer, FakeLauncher>, ask: &Ask) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        macro_rules! record {
            ($answered:expr) => {{
                let answered = $answered.await.unwrap();
                out.push((format!("{:?}", answered.value), answered.provenance.from_cache));
            }};
        }
        record!(lease.document_symbols(&ask.file));
        record!(lease.text_symbols(&ask.file, ask.text));
        record!(lease.definition(&ask.at));
        record!(lease.references(&ask.at));
        record!(lease.implementations(&ask.at));
        record!(lease.hover(&ask.at));
        record!(lease.incoming_calls(&ask.at));
        record!(lease.outgoing_calls(&ask.at));
        record!(lease.workspace_symbols(ask.query));
        record!(lease.diagnostics(&ask.file));
        out
    }

    /// Asks everything of the server for `key`; what it answered and which queries reached it.
    fn ask_in(
        pool: &TestPool,
        counters: &Counters,
        key: &PoolKey,
        ask: &Ask,
    ) -> (Vec<(String, bool)>, Vec<&'static str>) {
        block_on(async {
            let lease = pool.lease(key, &backend(false)).await.unwrap();
            let answers = ask_all(&lease, ask).await;
            lease.release().await;
            (answers, counters.asked())
        })
    }

    #[test]
    fn answers_outlive_their_server_while_every_key_input_is_unchanged() {
        let (pool, counters) = pool(4);
        let key = snapshot_key(1);
        let (first, asked) = ask_in(&pool, &counters, &key, &ask());
        assert_eq!(asked, ALL);
        assert!(first.iter().all(|(_, reused)| !reused));

        // The same server, and one started later that is the same in every respect.
        for _ in 0..2 {
            let (again, asked) = ask_in(&pool, &counters, &key, &ask());
            assert!(asked.is_empty(), "{asked:?}");
            assert!(again.iter().all(|(_, reused)| *reused));
            let values = |answers: &[(String, bool)]| answers.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
            assert_eq!(values(&again), values(&first));
            block_on(pool.restart(&BackendId("rust".into())));
        }
        assert_eq!(counters.started.load(Ordering::SeqCst), 2);
    }

    type Change = fn(&Counters, &mut PoolKey, &mut Ask);

    /// Each input of a key, what changing it does to the test's world, and the
    /// queries that reach a server afterwards.
    fn key_input_cases() -> [(&'static str, Change, &'static [&'static str]); 12] {
        [
            ("nothing", |_, _, _| {}, &[]),
            (
                "the server binary",
                |c, _, _| c.version.store(2, Ordering::SeqCst),
                &ALL,
            ),
            (
                "the restore of the tree",
                |c, _, _| c.restore.store(6, Ordering::SeqCst),
                &WHOLE_TREE,
            ),
            (
                "a restore nothing was recorded of",
                |c, _, _| c.restore.store(0, Ordering::SeqCst),
                &WHOLE_TREE,
            ),
            (
                "the snapshot",
                |_, key, _| key.target = Target::Snapshot(SnapshotId(Hash([2; 32]))),
                &WHOLE_TREE,
            ),
            (
                "the analysis policy",
                |_, key, _| key.policy = AnalysisPolicy::LIMITED,
                &WHOLE_TREE,
            ),
            ("the text", |_, _, ask| ask.text = "fn b() {}", &["text_symbols"]),
            (
                "the extension of the file",
                |_, _, ask| ask.file = file("a.py"),
                &["document_symbols", "text_symbols", "diagnostics"],
            ),
            (
                "the name of the file",
                |_, _, ask| ask.file = file("b.rs"),
                &["document_symbols", "diagnostics"],
            ),
            (
                "the position",
                |_, _, ask| ask.at.pos = Pos { line: 3, col: 4 },
                &[
                    "definition",
                    "references",
                    "implementations",
                    "hover",
                    "incoming_calls",
                    "outgoing_calls",
                ],
            ),
            (
                "the file of the position",
                |_, _, ask| ask.at.file = file("b.rs"),
                &[
                    "definition",
                    "references",
                    "implementations",
                    "hover",
                    "incoming_calls",
                    "outgoing_calls",
                ],
            ),
            (
                "the workspace query",
                |_, _, ask| ask.query = "b",
                &["workspace_symbols"],
            ),
        ]
    }

    #[test]
    fn changing_any_one_key_input_asks_a_server_and_only_for_the_answers_that_depend_on_it() {
        for (name, change, asks_again) in key_input_cases() {
            let (pool, counters) = pool(8);
            let (mut key, mut asked_about) = (snapshot_key(1), ask());
            ask_in(&pool, &counters, &key, &asked_about);

            // A server started afterwards is where the changes show, whether or not it differs.
            change(&counters, &mut key, &mut asked_about);
            block_on(pool.restart(&BackendId("rust".into())));
            let (_, asked) = ask_in(&pool, &counters, &key, &asked_about);
            assert_eq!(asked, asks_again, "after changing {name}");
        }
    }

    /// Answers kept by the key, in memory, as a stand-in for a file.
    #[derive(Default)]
    struct MemoryCache(Mutex<HashMap<Key, Value>>);

    impl SemanticCache for MemoryCache {
        fn get<'a>(&'a self, key: &'a Key) -> std::pin::Pin<Box<dyn Future<Output = Option<Value>> + Send + 'a>> {
            Box::pin(async move { self.0.lock().unwrap().get(key).cloned() })
        }

        fn put(&self, key: &Key, value: &Value) {
            self.0.lock().unwrap().insert(key.clone(), value.clone());
        }
    }

    /// A pool of a later launch: nothing in memory, the same store and the same world.
    fn launch_over(counters: &Arc<Counters>, store: &Arc<MemoryCache>) -> TestPool {
        Pool::new(
            FakeMaterializer(counters.clone()),
            FakeLauncher(counters.clone()),
            PathBuf::from("/live"),
            8,
        )
        .persisting(store.clone())
    }

    #[test]
    fn a_later_launch_reads_what_persists_from_the_store_and_asks_a_server_for_the_rest() {
        let counters = Arc::new(Counters::default());
        let store = Arc::new(MemoryCache::default());
        let (first, asked) = ask_in(&launch_over(&counters, &store), &counters, &snapshot_key(1), &ask());
        assert_eq!(asked, ALL);

        let (later, asked) = ask_in(&launch_over(&counters, &store), &counters, &snapshot_key(1), &ask());
        assert_eq!(asked, ["workspace_symbols"]);
        let values = |answers: &[(String, bool)]| answers.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
        assert_eq!(values(&later), values(&first));
        let reused: Vec<_> = later.iter().map(|(_, reused)| *reused).collect();
        assert_eq!(reused, [true, true, true, true, true, true, true, true, false, true]);
    }

    /// What `recall` finds of every cached kind of question: the kind and what was found.
    fn recall_all(pool: &TestPool, key: &PoolKey, ask: &Ask) -> Vec<(&'static str, String)> {
        let be = backend(false);
        let (file, at) = (&ask.file, &ask.at);
        block_on(async {
            let mut found = Vec::new();
            macro_rules! recalled {
                ($kind:expr, $answered:expr) => {
                    if let Some(answered) = $answered.await {
                        assert!(answered.provenance.from_cache);
                        found.push(($kind, format!("{:?}", answered.value)));
                    }
                };
            }
            let project = |query| pool.recall::<Vec<SymbolInfo>>(key, &be, query);
            recalled!("document_symbols", project(ProjectQuery::DocumentSymbols(file.clone())));
            recalled!("text_symbols", pool.recall_text_symbols(key, &be, file, ask.text));
            recalled!(
                "definition",
                pool.recall::<Vec<Location>>(key, &be, ProjectQuery::Definition(at.clone()))
            );
            recalled!(
                "references",
                pool.recall::<Vec<Location>>(key, &be, ProjectQuery::References(at.clone()))
            );
            recalled!(
                "implementations",
                pool.recall::<Vec<Location>>(key, &be, ProjectQuery::Implementations(at.clone()))
            );
            recalled!(
                "hover",
                pool.recall::<Option<String>>(key, &be, ProjectQuery::Hover(at.clone()))
            );
            recalled!("incoming_calls", project(ProjectQuery::Incoming(at.clone())));
            recalled!("outgoing_calls", project(ProjectQuery::Outgoing(at.clone())));
            recalled!(
                "workspace_symbols",
                project(ProjectQuery::Workspace(ask.query.to_owned()))
            );
            recalled!(
                "diagnostics",
                pool.recall::<Vec<Diagnostic>>(key, &be, ProjectQuery::Diagnostics(file.clone()))
            );
            found
        })
    }

    fn kinds(found: &[(&'static str, String)]) -> Vec<&'static str> {
        found.iter().map(|(kind, _)| *kind).collect()
    }

    /// The kinds a store keeps between launches.
    fn stored() -> Vec<&'static str> {
        ALL.into_iter().filter(|kind| *kind != "workspace_symbols").collect()
    }

    #[test]
    fn a_later_launch_finds_what_persists_without_materializing_a_tree_or_starting_a_server() {
        let counters = Arc::new(Counters::default());
        let store = Arc::new(MemoryCache::default());
        let (first, _) = ask_in(&launch_over(&counters, &store), &counters, &snapshot_key(1), &ask());
        let (started, materialized) = (
            counters.started.load(Ordering::SeqCst),
            counters.materialized.load(Ordering::SeqCst),
        );
        assert_eq!((started, materialized), (1, 1), "the first launch starts a server");

        let found = recall_all(&launch_over(&counters, &store), &snapshot_key(1), &ask());
        assert_eq!(kinds(&found), stored());
        for (kind, value) in &found {
            let asked = ALL.iter().position(|k| k == kind).expect("a kind of question");
            assert_eq!(*value, first[asked].0, "{kind}");
        }
        assert_eq!(counters.started.load(Ordering::SeqCst), started);
        assert_eq!(counters.materialized.load(Ordering::SeqCst), materialized);
    }

    #[test]
    fn changing_any_one_key_input_leaves_a_later_launch_nothing_to_find_for_what_depends_on_it() {
        for (name, change, asks_again) in key_input_cases() {
            let counters = Arc::new(Counters::default());
            let store = Arc::new(MemoryCache::default());
            let (mut key, mut asked_about) = (snapshot_key(1), ask());
            ask_in(&launch_over(&counters, &store), &counters, &key, &asked_about);

            change(&counters, &mut key, &mut asked_about);
            let found = recall_all(&launch_over(&counters, &store), &key, &asked_about);
            let expected: Vec<_> = stored().into_iter().filter(|kind| !asks_again.contains(kind)).collect();
            assert_eq!(kinds(&found), expected, "after changing {name}");
        }
    }

    #[test]
    fn nothing_is_found_for_a_snapshot_whose_tree_is_not_kept() {
        let counters = Arc::new(Counters::default());
        let store = Arc::new(MemoryCache::default());
        // The answers are stored, but the tree they were made on has not been made by this world.
        ask_in(&launch_over(&counters, &store), &counters, &snapshot_key(1), &ask());
        counters.materialized.store(0, Ordering::SeqCst);
        let found = recall_all(&launch_over(&counters, &store), &snapshot_key(1), &ask());
        assert_eq!(kinds(&found), Vec::<&str>::new());
    }

    #[test]
    fn a_running_servers_answers_are_found_in_memory_and_the_live_trees_only_for_a_text() {
        let (pool, counters) = pool(4);
        ask_in(&pool, &counters, &snapshot_key(1), &ask());
        assert_eq!(kinds(&recall_all(&pool, &snapshot_key(1), &ask())), ALL);

        ask_in(&pool, &counters, &live_key(), &ask());
        assert_eq!(kinds(&recall_all(&pool, &live_key(), &ask())), ["text_symbols"]);
    }

    #[test]
    fn changing_any_one_key_input_asks_a_server_in_a_later_launch_too() {
        for (name, change, asks_again) in key_input_cases() {
            let counters = Arc::new(Counters::default());
            let store = Arc::new(MemoryCache::default());
            let (mut key, mut asked_about) = (snapshot_key(1), ask());
            ask_in(&launch_over(&counters, &store), &counters, &key, &asked_about);

            change(&counters, &mut key, &mut asked_about);
            let (_, asked) = ask_in(&launch_over(&counters, &store), &counters, &key, &asked_about);
            // The workspace queries are not kept between launches.
            let expected: Vec<_> = ALL
                .into_iter()
                .filter(|kind| asks_again.contains(kind) || *kind == "workspace_symbols")
                .collect();
            assert_eq!(asked, expected, "after changing {name}");
        }
    }

    #[test]
    fn what_a_launch_may_not_keep_is_not_stored_for_the_next() {
        let hover_in_later_launch = |prepare: fn(&Counters, &mut PoolKey)| {
            let counters = Arc::new(Counters::default());
            let store = Arc::new(MemoryCache::default());
            let mut key = snapshot_key(1);
            prepare(&counters, &mut key);
            ask_in(&launch_over(&counters, &store), &counters, &key, &ask());
            counters.busy.store(false, Ordering::SeqCst);
            counters.begins_work.store(false, Ordering::SeqCst);
            ask_in(&launch_over(&counters, &store), &counters, &key, &ask())
                .1
                .contains(&"hover")
        };
        assert!(
            hover_in_later_launch(|c, _| c.busy.store(true, Ordering::SeqCst)),
            "given while the server indexed"
        );
        assert!(
            hover_in_later_launch(|c, _| c.begins_work.store(true, Ordering::SeqCst)),
            "given while work began"
        );
        assert!(
            hover_in_later_launch(|c, _| c.restore.store(0, Ordering::SeqCst)),
            "about a tree whose restore left no record"
        );
        assert!(
            hover_in_later_launch(|_, key| key.target = Target::Live),
            "about the live tree"
        );
        assert!(!hover_in_later_launch(|_, _| {}), "the same, when none of it applies");
    }

    #[test]
    fn an_answer_in_the_store_that_is_not_of_the_kind_asked_for_is_asked_of_the_server() {
        let counters = Arc::new(Counters::default());
        let store = Arc::new(MemoryCache::default());
        ask_in(&launch_over(&counters, &store), &counters, &snapshot_key(1), &ask());
        for value in store.0.lock().unwrap().values_mut() {
            *value = Value::Hover(None);
        }
        let (answers, asked) = ask_in(&launch_over(&counters, &store), &counters, &snapshot_key(1), &ask());
        assert_eq!(
            asked,
            [
                "document_symbols",
                "text_symbols",
                "definition",
                "references",
                "implementations",
                "incoming_calls",
                "outgoing_calls",
                "workspace_symbols",
                "diagnostics"
            ]
        );
        assert_eq!(answers.len(), ALL.len());
    }

    #[test]
    fn forgetting_the_answers_in_memory_leaves_those_in_the_store() {
        let counters = Arc::new(Counters::default());
        let store = Arc::new(MemoryCache::default());
        let pool = launch_over(&counters, &store);
        ask_in(&pool, &counters, &snapshot_key(1), &ask());
        pool.forget_answers();
        let (_, asked) = ask_in(&pool, &counters, &snapshot_key(1), &ask());
        assert_eq!(asked, ["workspace_symbols"]);

        store.0.lock().unwrap().clear();
        pool.forget_answers();
        let (_, asked) = ask_in(&pool, &counters, &snapshot_key(1), &ask());
        assert_eq!(asked, ALL);
    }

    #[test]
    fn the_symbols_kept_for_a_text_are_named_after_the_file_that_asks() {
        let (pool, counters) = pool(4);
        block_on(async {
            let lease = pool.lease(&snapshot_key(1), &backend(false)).await.unwrap();
            lease.text_symbols(&file("a.rs"), "fn a() {}").await.unwrap();
            let renamed = lease.text_symbols(&file("b.rs"), "fn a() {}").await.unwrap();
            assert!(renamed.provenance.from_cache);
            assert!(renamed.value.iter().all(|s| s.sym.file == file("b.rs")));
            assert_eq!(counters.asked(), ["text_symbols"]);
            lease.release().await;
        });
    }

    #[test]
    fn answers_of_the_live_tree_are_asked_every_time_except_the_symbols_of_a_text_and_nothing_is_materialized() {
        let (pool, counters) = pool(4);
        let mut asked = Vec::new();
        for _ in 0..2 {
            asked.extend(ask_in(&pool, &counters, &live_key(), &ask()).1);
        }
        let count = |kind: &str| asked.iter().filter(|k| **k == kind).count();
        assert!(WHOLE_TREE.iter().all(|kind| count(kind) == 2), "{asked:?}");
        assert_eq!(count("text_symbols"), 1);
        assert_eq!(counters.materialized.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn caching_on_and_off_gives_identical_answers() {
        fn script(kept: usize, store: Option<&Arc<MemoryCache>>) -> (Vec<Vec<String>>, usize) {
            let (pool, counters) = pool(8);
            let pool = pool.keeping(kept);
            let pool = match store {
                Some(store) => pool.persisting(store.clone()),
                None => pool,
            };
            let mut answered = Vec::new();
            let mut total = 0;
            let mut run = |counters: &Counters, key: &PoolKey, ask: &Ask| {
                let (answers, asked) = ask_in(&pool, counters, key, ask);
                total += asked.len();
                answered.push(answers.into_iter().map(|(value, _)| value).collect());
            };
            let rust = BackendId("rust".into());
            run(&counters, &snapshot_key(1), &ask());
            run(&counters, &snapshot_key(2), &ask());
            run(&counters, &snapshot_key(1), &ask());
            run(&counters, &live_key(), &ask());
            run(
                &counters,
                &snapshot_key(1),
                &Ask {
                    text: "fn b() {}",
                    ..ask()
                },
            );
            counters.restore.store(6, Ordering::SeqCst);
            block_on(pool.restart(&rust));
            run(&counters, &snapshot_key(1), &ask());
            counters.version.store(2, Ordering::SeqCst);
            block_on(pool.restart(&rust));
            run(&counters, &snapshot_key(1), &ask());
            run(&counters, &snapshot_key(2), &ask());
            (answered, total)
        }
        let (without, asked_without) = script(0, None);
        let (with, asked_with) = script(crate::reuse::KEPT, None);
        assert_eq!(with, without);
        assert!(asked_with < asked_without, "{asked_with} < {asked_without}");

        // Run again by a later launch over what the first one stored.
        let store = Arc::new(MemoryCache::default());
        let (first_launch, asked_first) = script(crate::reuse::KEPT, Some(&store));
        let (restarted, asked_restarted) = script(crate::reuse::KEPT, Some(&store));
        assert_eq!(first_launch, without);
        assert_eq!(restarted, without);
        assert_eq!(asked_first, asked_with);
        assert!(asked_restarted < asked_with, "{asked_restarted} < {asked_with}");
    }

    #[test]
    fn an_answer_given_while_the_server_was_not_ready_is_never_kept() {
        let (pool, counters) = pool(4);
        let key = snapshot_key(1);
        let hover = |reused_after: bool| {
            let (_, asked) = ask_in(&pool, &counters, &key, &ask());
            assert_eq!(asked.contains(&"hover"), !reused_after, "{asked:?}");
        };

        // Indexing: nothing is kept, however often it is asked.
        counters.busy.store(true, Ordering::SeqCst);
        hover(false);
        hover(false);

        // Work begins while the query is answered, though the server was idle when it was asked.
        counters.busy.store(false, Ordering::SeqCst);
        counters.begins_work.store(true, Ordering::SeqCst);
        hover(false);
        hover(false);

        // Idle all the way: kept.
        counters.begins_work.store(false, Ordering::SeqCst);
        hover(false);
        hover(true);
    }

    #[test]
    fn an_error_is_never_kept() {
        let (pool, counters) = pool(4);
        let key = snapshot_key(1);
        counters.failing.store(true, Ordering::SeqCst);
        block_on(async {
            let lease = pool.lease(&key, &backend(false)).await.unwrap();
            assert!(lease.hover(&at()).await.is_err());
            counters.failing.store(false, Ordering::SeqCst);
            assert!(!lease.hover(&at()).await.unwrap().provenance.from_cache);
            assert!(lease.hover(&at()).await.unwrap().provenance.from_cache);
            lease.release().await;
        });
    }

    #[test]
    fn an_answer_names_the_server_that_started_and_whether_it_was_reused() {
        let (pool, counters) = pool(2);
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

            // A server started later with another binary is another identity, and so is its answer's.
            counters.version.store(2, Ordering::SeqCst);
            pool.restart(&BackendId("rust".into())).await;
            let lease = pool.lease(&snapshot_key(1), &backend(true)).await.unwrap();
            let again = lease.hover(&at()).await.unwrap().provenance;
            assert_eq!(again.server.label, "fake 2");
            assert!(!again.from_cache);
            lease.release().await;
        });
    }

    #[test]
    fn however_many_ask_a_server_only_its_request_budget_is_in_flight_on_it() {
        let (pool, counters) = pool(2);
        counters.slow.store(true, Ordering::SeqCst);
        block_on(async {
            let lease = pool.lease(&live_key(), &backend(true)).await.unwrap();
            let asks = bounded(48, 0..48, |line| {
                let lease = &lease;
                async move {
                    let at = At {
                        file: RepoPath(b"a.rs".to_vec()),
                        pos: Pos { line, col: 0 },
                    };
                    lease.hover(&at).await.unwrap()
                }
            })
            .await;
            assert_eq!(asks.len(), 48);
            assert_eq!(
                counters.hover_calls.load(Ordering::SeqCst),
                48,
                "every question was asked"
            );
            assert_eq!(counters.peak.load(Ordering::SeqCst), REQUESTS_PER_SERVER);
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
            // Both see the same kept answers.
            let first = a.hover(&at()).await.unwrap().value;
            assert_eq!(b.hover(&at()).await.unwrap().value, first);
            assert_eq!(counters.hover_calls.load(Ordering::SeqCst), 1);
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
            assert!(matches!(b, Err(PoolError::Launch(_))));
            assert_eq!(counters.started.load(Ordering::SeqCst), 1);
            assert!(pool.keys().is_empty());
            assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);

            counters.fail_start.store(false, Ordering::SeqCst);
            pool.lease(&key, &backend(true)).await.unwrap().release().await;
            assert_eq!(counters.started.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn the_waiters_finish_a_start_whose_caller_gave_up_without_starting_another() {
        let (pool, counters) = pool(2);
        counters.hold_start.store(true, Ordering::SeqCst);
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
        counters.hold_start.store(false, Ordering::SeqCst);
        block_on(async {
            let (first, second) = join(first, second).await;
            let (first, second) = (first.unwrap(), second.unwrap());
            assert_eq!(counters.started.load(Ordering::SeqCst), 1);
            assert_eq!(counters.materialized.load(Ordering::SeqCst), 1);
            assert!(Arc::ptr_eq(
                first.entry.as_ref().unwrap(),
                second.entry.as_ref().unwrap()
            ));
            assert_eq!(first.leases.load(Ordering::SeqCst), 2);
            assert_eq!(pool.keys(), [snapshot_key(1)]);
            first.release().await;
            second.release().await;
        });
        assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_start_nobody_waits_for_any_more_is_dropped_with_its_directory() {
        let (pool, counters) = pool(2);
        counters.hold_start.store(true, Ordering::SeqCst);
        let key = snapshot_key(1);
        let be = backend(true);
        let mut cx = Context::from_waker(Waker::noop());
        let mut first = Box::pin(pool.lease(&key, &be));
        let mut second = Box::pin(pool.lease(&key, &be));
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert!(second.as_mut().poll(&mut cx).is_pending());
        drop(first);
        assert_eq!(
            counters.guards_dropped.load(Ordering::SeqCst),
            0,
            "the other still waits"
        );
        drop(second);
        assert_eq!(counters.guards_dropped.load(Ordering::SeqCst), 1);
        assert!(pool.keys().is_empty());

        // The next caller starts the server afresh.
        counters.hold_start.store(false, Ordering::SeqCst);
        block_on(pool.lease(&key, &be)).unwrap();
        assert_eq!(counters.started.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_start_is_not_shared_with_a_caller_that_came_after_a_restart() {
        let (pool, counters) = pool(2);
        counters.hold_start.store(true, Ordering::SeqCst);
        let key = snapshot_key(1);
        let be = backend(true);
        let mut cx = Context::from_waker(Waker::noop());
        let mut before = Box::pin(pool.lease(&key, &be));
        assert!(before.as_mut().poll(&mut cx).is_pending());
        block_on(pool.restart(&BackendId("rust".into())));
        let mut after = Box::pin(pool.lease(&key, &be));
        assert!(after.as_mut().poll(&mut cx).is_pending());
        assert_eq!(counters.started.load(Ordering::SeqCst), 2);
        counters.hold_start.store(false, Ordering::SeqCst);

        block_on(async {
            let (before, after) = join(before, after).await;
            let (before, after) = (before.unwrap(), after.unwrap());
            assert!(before.retired.load(Ordering::SeqCst), "it started before the restart");
            assert!(!after.retired.load(Ordering::SeqCst));
            assert_eq!(pool.keys(), [snapshot_key(1)]);
            before.release().await;
            after.release().await;
        });
    }

    #[test]
    fn a_waiter_that_gave_up_holds_no_lease() {
        let (pool, counters) = pool(2);
        counters.hold_start.store(true, Ordering::SeqCst);
        let key = snapshot_key(1);
        let be = backend(true);
        let mut cx = Context::from_waker(Waker::noop());
        let mut starter = Box::pin(pool.lease(&key, &be));
        let mut waiter = Box::pin(pool.lease(&key, &be));
        assert!(starter.as_mut().poll(&mut cx).is_pending());
        assert!(waiter.as_mut().poll(&mut cx).is_pending());
        drop(waiter);
        counters.hold_start.store(false, Ordering::SeqCst);
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
    fn what_a_backends_server_offers_is_known_once_one_started_and_forgotten_by_a_restart() {
        let (pool, counters) = pool(4);
        let rust = BackendId("rust".into());
        block_on(async {
            assert_eq!(pool.features(&rust), None);
            pool.lease(&snapshot_key(1), &backend(true))
                .await
                .unwrap()
                .release()
                .await;
            assert_eq!(pool.features(&rust), Some(Features::ALL));
            pool.restart(&rust).await;
            assert_eq!(pool.features(&rust), None);

            counters.lacking.lock().unwrap().push(Feature::CallHierarchy);
            pool.lease(&snapshot_key(1), &backend(true))
                .await
                .unwrap()
                .release()
                .await;
            let offered = pool.features(&rust).unwrap();
            assert!(!offered.offers(Feature::CallHierarchy));
            assert!(offered.offers(Feature::Hover));
        });
    }

    #[test]
    fn a_query_for_an_operation_the_server_lacks_is_not_sent_and_says_which_server_lacks_it() {
        use crate::query::{Primitives, Semantic, SemanticError, Served};
        use vet_core::review::Side;

        let (pool, counters) = pool(4);
        counters
            .lacking
            .lock()
            .unwrap()
            .extend([Feature::CallHierarchy, Feature::WorkspaceSymbols]);
        let semantic = Arc::new(Semantic::new(pool, vec![backend(true)]));
        let served = Served {
            target: Target::Live,
            policy: AnalysisPolicy::EXECUTING,
        };
        let pair = semantic.pair(served, served);
        block_on(async {
            for error in [
                pair.incoming_calls(Side::Target, &at()).await.unwrap_err(),
                pair.outgoing_calls(Side::Target, &at()).await.unwrap_err(),
            ] {
                assert!(
                    matches!(
                        error,
                        SemanticError::NotOffered {
                            feature: Feature::CallHierarchy,
                            ..
                        }
                    ),
                    "{error:?}"
                );
            }
            assert!(pair.hover(Side::Target, &at()).await.is_ok());
            let workspace = semantic
                .workspace_symbols(&served, &[BackendId("rust".into())], "x")
                .await;
            assert!(matches!(
                workspace[0].1,
                Err(SemanticError::NotOffered {
                    feature: Feature::WorkspaceSymbols,
                    ..
                })
            ));
            let asked = counters.asked();
            assert!(asked.contains(&"hover"), "{asked:?}");
            assert!(
                !asked
                    .iter()
                    .any(|k| ["incoming_calls", "outgoing_calls", "workspace_symbols"].contains(k)),
                "{asked:?}"
            );
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
    fn restarting_now_stops_a_leased_server_and_fails_its_requests_in_flight_as_restarted() {
        let (pool, counters) = pool(4);
        counters.hang.store(true, Ordering::SeqCst);
        block_on(async {
            let key = snapshot_key(1);
            let lease = pool.lease(&key, &backend(true)).await.unwrap();
            let (asked, ()) = join(lease.hover(&at()), async {
                yield_once().await;
                pool.restart_now(&BackendId("rust".into())).await;
            })
            .await;
            assert_eq!(asked.unwrap_err().0, "the language server was restarted");
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 1);
            assert!(pool.keys().is_empty());

            // The last lease of the stopped server ending stops nothing again.
            lease.release().await;
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 1);

            counters.hang.store(false, Ordering::SeqCst);
            let fresh = pool.lease(&key, &backend(true)).await.unwrap();
            assert_eq!(counters.started.load(Ordering::SeqCst), 2);
            assert!(fresh.hover(&at()).await.is_ok());
            fresh.release().await;
        });
    }

    #[test]
    fn restarting_now_leaves_other_backends_servers_running() {
        let (pool, counters) = pool(4);
        block_on(async {
            pool.lease(&snapshot_key(1), &backend(true))
                .await
                .unwrap()
                .release()
                .await;
            pool.restart_now(&BackendId("python".into())).await;
            assert_eq!(pool.keys().len(), 1);
            assert_eq!(counters.shut_down.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn refused_backend_starts_nothing() {
        let (pool, counters) = pool(2);
        let key = PoolKey {
            policy: AnalysisPolicy::LIMITED,
            ..snapshot_key(1)
        };
        let result = block_on(pool.lease(&key, &backend(true)));
        assert!(matches!(result, Err(PoolError::AnalysisLimited(_))));
        assert_eq!(counters.materialized.load(Ordering::SeqCst), 0);
    }

    type Call<'a> = std::pin::Pin<Box<dyn Future<Output = ()> + 'a>>;

    /// Polls the calls in turn until all are done. `abandon` is dropped after the first turn.
    fn run_together(mut calls: Vec<Option<Call<'_>>>, abandon: Option<usize>) {
        let mut cx = Context::from_waker(Waker::noop());
        for turn in 0..1000 {
            for call in &mut calls {
                if let Some(running) = call
                    && running.as_mut().poll(&mut cx).is_ready()
                {
                    *call = None;
                }
            }
            if let (0, Some(at)) = (turn, abandon) {
                calls[at] = None;
            }
            if calls.iter().all(Option::is_none) {
                return;
            }
        }
        panic!("the calls do not finish");
    }

    /// A question of each kind a server answers, its inputs told apart by `n`.
    enum Question {
        Text { file: RepoPath, text: String },
        Project(ProjectQuery),
    }

    fn questions(n: u32) -> Vec<Question> {
        let at = At {
            file: file("a.rs"),
            pos: Pos { line: n, col: n },
        };
        let own_file = file(&format!("f{n}.rs"));
        vec![
            Question::Text {
                file: file("a.rs"),
                text: format!("fn a{n}() {{}}"),
            },
            Question::Project(ProjectQuery::DocumentSymbols(own_file.clone())),
            Question::Project(ProjectQuery::Definition(at.clone())),
            Question::Project(ProjectQuery::References(at.clone())),
            Question::Project(ProjectQuery::Implementations(at.clone())),
            Question::Project(ProjectQuery::Hover(at.clone())),
            Question::Project(ProjectQuery::Incoming(at.clone())),
            Question::Project(ProjectQuery::Outgoing(at)),
            Question::Project(ProjectQuery::Workspace(format!("q{n}"))),
            Question::Project(ProjectQuery::Diagnostics(own_file)),
        ]
    }

    /// Asks `question` of the server. Every kind of question is listed here without a
    /// wildcard, so a new one does not compile until it is asked.
    fn put<'a>(lease: &'a Lease<FakeMaterializer, FakeLauncher>, question: &'a Question) -> Call<'a> {
        match question {
            Question::Text { file, text } => Box::pin(async move {
                lease.text_symbols(file, text).await.unwrap();
            }),
            Question::Project(query) => match query {
                ProjectQuery::DocumentSymbols(file) => Box::pin(async move {
                    lease.document_symbols(file).await.unwrap();
                }),
                ProjectQuery::Definition(at) => Box::pin(async move {
                    lease.definition(at).await.unwrap();
                }),
                ProjectQuery::References(at) => Box::pin(async move {
                    lease.references(at).await.unwrap();
                }),
                ProjectQuery::Implementations(at) => Box::pin(async move {
                    lease.implementations(at).await.unwrap();
                }),
                ProjectQuery::Hover(at) => Box::pin(async move {
                    lease.hover(at).await.unwrap();
                }),
                ProjectQuery::Incoming(at) => Box::pin(async move {
                    lease.incoming_calls(at).await.unwrap();
                }),
                ProjectQuery::Outgoing(at) => Box::pin(async move {
                    lease.outgoing_calls(at).await.unwrap();
                }),
                ProjectQuery::Workspace(query) => Box::pin(async move {
                    lease.workspace_symbols(query).await.unwrap();
                }),
                ProjectQuery::Diagnostics(file) => Box::pin(async move {
                    lease.diagnostics(file).await.unwrap();
                }),
            },
        }
    }

    #[test]
    fn every_kind_of_question_reaches_the_server_once_however_many_callers_ask_or_give_up() {
        let (pool, counters) = pool(4);
        counters.slow.store(true, Ordering::SeqCst);
        block_on(async {
            let lease = pool.lease(&snapshot_key(1), &backend(false)).await.unwrap();
            counters.asked();
            // The caller that began the flight stays in the first round and gives up in the second.
            for (n, abandon) in [(1, None), (2, Some(0))] {
                for question in &questions(n) {
                    let callers = (0..8).map(|_| Some(put(&lease, question))).collect();
                    run_together(callers, abandon);
                }
                let mut asked = counters.asked();
                asked.sort_unstable();
                let mut expected = ALL.to_vec();
                expected.sort_unstable();
                assert_eq!(asked, expected, "round {n}");
            }
            lease.release().await;
        });
    }

    #[test]
    fn the_identity_and_the_basis_of_a_key_are_observed_once_per_generation_however_many_ask() {
        let (pool, counters) = pool(4);
        let (key, be) = (snapshot_key(1), backend(false));
        block_on(async {
            pool.lease(&key, &be).await.unwrap().release().await;
        });
        let observed = || {
            (
                counters.identified.load(Ordering::SeqCst),
                counters.recorded.load(Ordering::SeqCst),
            )
        };
        let recall = |query: ProjectQuery| {
            let (pool, key, be) = (&pool, &key, &be);
            Some(Box::pin(async move {
                pool.recall::<Option<String>>(key, be, query).await;
            }) as Call<'_>)
        };

        counters.slow.store(true, Ordering::SeqCst);
        let (identified, recorded) = observed();
        run_together(
            (0..8).map(|n| recall(ProjectQuery::Workspace(n.to_string()))).collect(),
            None,
        );
        assert_eq!(
            observed(),
            (identified + 1, recorded + 1),
            "eight callers, one observation"
        );
        run_together(
            (8..40)
                .map(|n| recall(ProjectQuery::Workspace(n.to_string())))
                .collect(),
            None,
        );
        assert_eq!(
            observed(),
            (identified + 1, recorded + 1),
            "and the same for every later caller"
        );

        // A start may have recorded a restore: the basis is looked at again, the identity is not.
        block_on(async {
            pool.restart(&BackendId("rust".into())).await;
        });
        let (identified, recorded) = observed();
        run_together(
            (0..8).map(|n| recall(ProjectQuery::Workspace(n.to_string()))).collect(),
            None,
        );
        assert_eq!(
            observed(),
            (identified + 1, recorded + 1),
            "a new generation observes anew"
        );
        block_on(async {
            pool.lease(&key, &be).await.unwrap().release().await;
        });
        let (identified, recorded) = observed();
        run_together(
            (0..8).map(|n| recall(ProjectQuery::Workspace(n.to_string()))).collect(),
            None,
        );
        assert_eq!(observed(), (identified, recorded + 1), "a start drops the basis only");
    }
}
