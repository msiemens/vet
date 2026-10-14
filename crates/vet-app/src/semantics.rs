//! Semantic queries of a session (§7.3): the side being viewed is asked
//! directly, derived analyses combine the answers with the revision's diff.
//! Positions and results belong to a revision; a stale one is refused like any
//! other request. Base answers come from the session's base snapshot. Target
//! answers come from the live tree, which is the revision's target until the next
//! snapshot settles, or, when the target is a commit's tree, from a snapshot.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use vet_core::content::{FileDiff, RepoPath};
use vet_core::review::Side;
use vet_semantic::derived::{
    Bounds, Calls, Change, Compared, Derived, EntryPoints, FileChanges, Judgement, Located, OutlineSymbol, Reach,
    Reference, RemovalImpact, Texts, Usages, calls, compared, judged, located, outline, reachability, removal_impact,
    symbol_at, usages,
};
use vet_semantic::join::bounded;
use vet_semantic::pool::{Phase, Pool, PoolKey, Supervision, Target};
use vet_semantic::ports::{
    At, Backend, BackendId, Diagnostic, Features, Location, Materializer, Provenance, RestoreFailure, ServerLauncher,
    Severity, Sym, SymbolInfo, prefer_openable,
};
use vet_semantic::query::{Asked, Finds, Pair, Primitives, Semantic, SemanticError, Served};
use vet_semantic::reuse::SemanticCache;
use vet_semantic::signature::{Signature, signature};
use vet_semantic::symbols::enclosing;
use vet_semantic::trace;

use crate::acquisition::Acquisition;
use crate::limits::RestoreLimits;
use crate::ports::{BlobRead, BlobReader, GitRepo, LiveStore, Store};
use crate::revisions::Revisions;
use crate::session::{RevisionView, SemanticSides, SessionError};
use crate::trust::Trust;

/// Servers kept warm per session: base and target of each language.
const POOL_CAPACITY: usize = 8;

/// Reachability stops at the first entry point within this many calls and symbols.
const REACH_BOUNDS: Bounds = Bounds {
    max_depth: 8,
    max_nodes: 200,
};

/// The calls of a symbol are followed this far, through this many changed callers.
const CALLS_BOUNDS: Bounds = Bounds {
    max_depth: 6,
    max_nodes: 50,
};

/// An answer to a primitive query, at a revision. The query itself can fail
/// without the request failing: the language may have no usable server.
#[derive(Debug)]
pub struct Answer<T> {
    pub revision: u32,
    pub result: Result<T, SemanticError>,
    /// The server that answered and whether the answer was reused (I6); none if no
    /// server did, or the answer is made of more than one server's.
    pub provenance: Option<Provenance>,
    /// No server of the answer was at work on its own while it answered (see
    /// [`vet_semantic::ports::Answered::complete`]).
    pub complete: bool,
}

impl<T> Answer<T> {
    fn of(revision: u32, asked: Asked<T>) -> Self {
        match asked {
            Ok(answered) => Self {
                revision,
                result: Ok(answered.value),
                provenance: Some(answered.provenance),
                complete: answered.complete,
            },
            Err(e) => Self::derived(revision, Err(e)),
        }
    }

    /// An answer made of more than one server's, or of none: no one to name.
    pub fn derived(revision: u32, result: Result<T, SemanticError>) -> Self {
        Self {
            revision,
            result,
            provenance: None,
            complete: true,
        }
    }
}

type Fallible<T> = Result<T, SessionError>;

/// The changed file with `path` on one of its sides.
fn changed_file<'a>(files: &'a [FileDiff], path: &RepoPath) -> Option<&'a FileDiff> {
    files
        .iter()
        .find(|f| f.new_path.as_ref() == Some(path) || f.old_path.as_ref() == Some(path))
}

/// What the changeset did to the symbols of one file.
pub struct FileSymbols {
    pub changes: Derived<FileChanges>,
    /// Why the file's symbols are unknown, if they are.
    pub gap: Option<SemanticError>,
}

/// What one language's server answered to a search of the project's symbols by name.
pub struct LanguageSymbols {
    pub language: BackendId,
    /// A server that cannot serve is reported, not left out.
    pub result: Result<Vec<SymbolInfo>, SemanticError>,
}

/// The symbols of the target side that match a name, per language of the project.
pub struct WorkspaceSymbols {
    pub revision: u32,
    pub languages: Vec<LanguageSymbols>,
}

/// A symbol found by its identity, with the side it exists on.
struct Resolved {
    /// The target side's symbol, or the base side's if it exists there only.
    symbol: SymbolInfo,
    side: Side,
    /// `None`: unchanged.
    change: Option<Change>,
    /// The symbol on the base side, for a modified one.
    base: Option<SymbolInfo>,
}

/// The symbol `sym` of the changed `file`, whose sides' symbols are `found`: a symbol of the new
/// path (target side), or one of the old path that the target does not have (removed, base side). A
/// renamed file's symbols are named by the path of the side they are on.
fn locate(file: &FileDiff, found: &Compared, sym: &Sym) -> Option<Resolved> {
    let named = |symbols: &[SymbolInfo]| symbols.iter().find(|s| s.sym.name == sym.name).cloned();
    let base = named(&found.base);
    let target = named(&found.target);
    if file.new_path.as_ref() == Some(&sym.file)
        && let Some(symbol) = target
    {
        let change = (found.changes.change_of(&symbol.sym).map(|c| c.change))
            .or_else(|| base.is_none().then_some(Change::Added));
        return Some(Resolved {
            symbol,
            side: Side::Target,
            change,
            base: base.filter(|_| change == Some(Change::Modified)),
        });
    }
    if file.old_path.as_ref() == Some(&sym.file) && named(&found.target).is_none() {
        return base.map(|symbol| Resolved {
            symbol,
            side: Side::Base,
            change: Some(Change::Removed),
            base: None,
        });
    }
    None
}

/// The symbols `text` declares as the content of `path`, none of them marked as changed.
async fn unmarked(p: &impl Primitives, path: &RepoPath, text: &str) -> Result<Vec<OutlineSymbol>, SemanticError> {
    let found = p.text_symbols(path, text).await?;
    Ok(found
        .value
        .into_iter()
        .map(|symbol| OutlineSymbol { symbol, change: None })
        .collect())
}

/// How many of a file's diagnostics count towards what a viewer is told: hints are not counted.
pub fn countable(diagnostics: &[Diagnostic]) -> u32 {
    diagnostics.iter().filter(|d| d.severity != Severity::Hint).count() as u32
}

/// What semantics share with the rest of the session.
pub struct Links {
    /// The session's, which announces every move of its revision. Work for an older
    /// one is dropped, and with it the requests it waits for.
    pub revisions: Arc<Revisions>,
    /// The one the launcher's restorer updates.
    pub limits: RestoreLimits,
}

/// What the server of a language is doing for one side of the revision on view.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerRuntime {
    pub language: BackendId,
    pub side: Side,
    pub phase: Phase,
}

/// What the servers of the revision on view are doing. A later `serial` is newer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerRuntimes {
    pub serial: u64,
    pub runtimes: Vec<ServerRuntime>,
}

/// Told what the servers of the revision on view are doing, after every change.
pub type RuntimeSink = dyn Fn(ServerRuntimes) + Send + Sync;

/// The revision on view, whose servers are reported, and whom to report them to.
#[derive(Default)]
struct Watched {
    sides: Option<(u32, Served, Served)>,
    sink: Option<Arc<RuntimeSink>>,
    /// The latest change of the pool taken into account.
    pool_serial: u64,
    serial: u64,
}

impl Watched {
    fn runtimes(&self, phases: &[(PoolKey, Phase)]) -> Vec<ServerRuntime> {
        let Some((_, base, target)) = &self.sides else {
            return Vec::new();
        };
        let mut runtimes: Vec<_> = phases
            .iter()
            .flat_map(|(key, phase)| {
                [(Side::Base, base), (Side::Target, target)]
                    .into_iter()
                    .filter(|(_, served)| key.target == served.target && key.policy == served.policy)
                    .map(|(side, _)| ServerRuntime {
                        language: key.backend.clone(),
                        side,
                        phase: phase.clone(),
                    })
            })
            .collect();
        runtimes.sort_by(|a, b| (&a.language.0, a.side as u8).cmp(&(&b.language.0, b.side as u8)));
        runtimes
    }

    /// Tells the sink what the servers are doing, as of the pool's change `pool_serial`; what is
    /// older than what was told already is dropped.
    fn pooled(&mut self, pool_serial: u64, phases: &[(PoolKey, Phase)]) {
        if pool_serial >= self.pool_serial {
            self.pool_serial = pool_serial;
            self.announce(phases);
        }
    }

    fn announce(&mut self, phases: &[(PoolKey, Phase)]) {
        if let Some(sink) = self.sink.clone() {
            self.serial += 1;
            sink(ServerRuntimes {
                serial: self.serial,
                runtimes: self.runtimes(phases),
            });
        }
    }
}

pub struct Semantics<G, S, L, M>
where
    G: GitRepo + Send + Sync,
    S: Store + Send + Sync,
    L: ServerLauncher,
    M: Materializer,
{
    semantic: Arc<Semantic<M, L>>,
    watched: Arc<Mutex<Watched>>,
    /// Where the texts of the files come from when they are not the session's own: the
    /// semantic lane reads them on its own budget.
    texts: Option<Arc<Acquisition<G, S>>>,
    revisions: Arc<Revisions>,
    /// Kept current by the launcher's restorer (`Limiting`).
    limits: RestoreLimits,
    /// The languages that have a file in the target of a revision, found once per revision: finding
    /// them walks every file of the snapshot.
    languages: Mutex<Option<(u32, Vec<BackendId>)>>,
}

/// The file contents of both sides of a revision, as the acquisition keeps them.
struct Content<G, S>(Arc<Acquisition<G, S>>);

impl<G: BlobReader + Send + Sync, S: Store + Send + Sync> Content<G, S> {
    async fn text(&self, path: Option<&RepoPath>, blob: Option<vet_core::content::BlobId>) -> Option<String> {
        match self.0.blob(path?, blob?, true).await {
            Ok(BlobRead::Found(bytes)) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            _ => None,
        }
    }
}

impl<G: BlobReader + Send + Sync, S: Store + Send + Sync> Texts for Content<G, S> {
    async fn base_text(&self, file: &FileDiff) -> Option<String> {
        self.text(file.old_path.as_ref(), file.old_blob).await
    }

    async fn target_text(&self, file: &FileDiff) -> Option<String> {
        self.text(file.new_path.as_ref(), file.new_blob).await
    }
}

/// The base and the target side as the pool serves them: each may execute
/// project code as far as `trust` holds for the commit it is built on (the base
/// shares the trust of a head the pull request is trusted at). The target
/// is served from the live tree only when it is the live tree; a commit's tree is
/// materialized like the base.
fn served(trust: &Trust, sides: &SemanticSides) -> (Served, Served) {
    (
        Served {
            target: Target::Snapshot(sides.base),
            policy: trust.base_policy(&sides.base_commit, &sides.head),
        },
        Served {
            target: if sides.live_target {
                Target::Live
            } else {
                Target::Snapshot(sides.target)
            },
            policy: trust.policy(&sides.head),
        },
    )
}

/// Starts what the changed `files` need on `sides`, and the target side's
/// servers for the languages of `others`, only those of `only` if it is given.
/// Changed languages come first, because only that many fit in the pool, and
/// are the ones the pool keeps (unless `only` is given, which leaves what it keeps as it was).
/// The error is the first of the starts that failed.
async fn start_servers_for<M: Materializer, L: ServerLauncher>(
    pair: &Pair<M, L>,
    files: &[FileDiff],
    sides: &[Side],
    others: &[RepoPath],
    only: Option<&BackendId>,
) -> Result<(), SemanticError> {
    let semantic = pair.semantic();
    let mut candidates: Vec<(Side, &Served, &RepoPath)> = Vec::new();
    for file in files.iter().filter(|f| !f.hunks.is_empty()) {
        for (side, served, path) in [
            (Side::Base, &pair.base, &file.old_path),
            (Side::Target, &pair.target, &file.new_path),
        ] {
            if let (true, Some(path)) = (sides.contains(&side), path) {
                candidates.push((side, served, path));
            }
        }
    }
    let changed = candidates.len();
    candidates.extend(others.iter().map(|path| (Side::Target, &pair.target, path)));

    let mut wanted: Vec<(Side, &Served, &RepoPath)> = Vec::new();
    let mut languages: Vec<(Side, &BackendId)> = Vec::new();
    let mut changed_wanted = 0;
    for (at, (side, served, path)) in candidates.into_iter().enumerate() {
        let Some(language) = semantic
            .language_of(path)
            .filter(|l| only.is_none_or(|only| only == *l))
        else {
            continue;
        };
        if !languages.contains(&(side, language)) {
            languages.push((side, language));
            wanted.push((side, served, path));
            changed_wanted += usize::from(at < changed);
        }
    }
    // Servers beyond what the pool keeps warm would only evict each other.
    wanted.truncate(POOL_CAPACITY);
    if only.is_none() {
        let pinned: Vec<_> = wanted[..changed_wanted.min(wanted.len())]
            .iter()
            .map(|(_, served, file)| (*served, *file))
            .collect();
        semantic.keep_warm(&pinned);
    }
    // The base side's servers restore and index a snapshot, which slows the
    // target side's startup down, and the target side is what the first
    // views need. So they start once the target side's are running, and
    // are warm by the time anyone hovers the base side.
    let mut failed = Ok(());
    for phase in [Side::Target, Side::Base] {
        let phase: Vec<_> = wanted.iter().filter(|(side, ..)| *side == phase).collect();
        let started = bounded(POOL_CAPACITY, 0..phase.len(), |at| {
            let (_, served, file) = phase[at];
            semantic.start_for(served, file)
        })
        .await;
        failed = failed.and(started.into_iter().collect::<Result<(), _>>());
    }
    failed
}

impl<G, S, L, M> Semantics<G, S, L, M>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
    L: ServerLauncher,
    M: Materializer,
{
    /// `materializer` serves the session's snapshots as directories and
    /// `live_root` is the worktree the target side is served from.
    pub fn new(
        materializer: M,
        launcher: L,
        live_root: std::path::PathBuf,
        backends: Vec<Backend>,
        links: Links,
    ) -> Self {
        let pool = Pool::new(materializer, launcher, live_root, POOL_CAPACITY);
        Self {
            semantic: Arc::new(Semantic::new(pool, backends)),
            watched: Arc::default(),
            texts: None,
            revisions: links.revisions,
            limits: links.limits,
            languages: Mutex::new(None),
        }
    }

    /// Configures the semantic layer, which is not shared yet.
    fn configured(mut self, configure: impl FnOnce(Semantic<M, L>) -> Semantic<M, L>) -> Self {
        let semantic = Arc::into_inner(self.semantic).expect("semantics are configured before they are used");
        self.semantic = Arc::new(configure(semantic));
        self
    }

    /// Reads the texts the changed symbols are made from through `acquisition`, not the session's.
    pub fn reading(mut self, acquisition: Arc<Acquisition<G, S>>) -> Self {
        self.texts = Some(acquisition);
        self
    }

    /// Keeps at most `limit` answers of the servers for reuse; 0 keeps none.
    pub fn keeping(self, limit: usize) -> Self {
        self.configured(|semantic| semantic.keeping(limit))
    }

    /// Keeps the answers that persist in `store`, behind the ones in memory.
    pub fn persisting(self, store: Arc<dyn SemanticCache>) -> Self {
        self.configured(|semantic| semantic.persisting(store))
    }

    /// Starts a server that dies while running anew once, and tells `supervision` what became of it.
    pub fn supervised(self, supervision: Supervision) -> Self {
        self.configured(|semantic| semantic.supervised(supervision))
    }

    /// Looks up the files that servers require of a project (`ServerConfig::requires`) with `finds`.
    pub fn finding(self, finds: Finds) -> Self {
        self.configured(|semantic| semantic.finding(finds))
    }

    /// Drops the answers kept in memory; the next question is asked again (or read from the store).
    pub fn forget_answers(&self) {
        self.semantic.pool().forget_answers();
    }

    /// Tells `sink` what the servers of the revision on view are doing, after every change.
    pub fn observing(self, sink: Arc<RuntimeSink>) -> Self {
        self.watched.lock().expect("not poisoned").sink = Some(sink);
        let watched = self.watched.clone();
        self.configured(|semantic| {
            semantic.observed(Arc::new(move |pool_serial, phases| {
                watched.lock().expect("not poisoned").pooled(pool_serial, &phases);
            }))
        })
    }

    /// What the servers of the revision on view are doing now.
    pub fn runtimes(&self) -> ServerRuntimes {
        let watched = self.watched.lock().expect("not poisoned");
        let (_, phases) = self.semantic.pool().phases();
        ServerRuntimes {
            serial: watched.serial,
            runtimes: watched.runtimes(&phases),
        }
    }

    /// The language that serves `file`; `None`: it is unsupported.
    pub fn language_of(&self, file: &RepoPath) -> Option<&BackendId> {
        self.semantic.language_of(file)
    }

    /// What the server of `language` offers; `None` before one has started.
    pub fn features(&self, language: &BackendId) -> Option<Features> {
        self.semantic.pool().features(language)
    }

    /// Restores that failed and have not succeeded since.
    pub fn restore_failures(&self) -> Vec<RestoreFailure> {
        self.limits.all()
    }

    async fn pair(&self, view: &RevisionView<G, S>, revision: u32) -> Fallible<Pair<M, L>> {
        let sides = view.semantic_sides(revision).await?;
        let (base, target) = served(&view.trust(&sides.source).await?, &sides);
        self.view(revision, base, target);
        Ok(self.semantic.pair(base, target))
    }

    /// The servers of `base` and `target` are those whose state is reported, unless a newer
    /// revision is on view.
    fn view(&self, revision: u32, base: Served, target: Served) {
        let mut watched = self.watched.lock().expect("not poisoned");
        match &watched.sides {
            Some((at, ..)) if *at > revision => return,
            Some((at, b, t)) if *at == revision && *b == base && *t == target => return,
            _ => {}
        }
        watched.sides = Some((revision, base, target));
        let (pool_serial, phases) = self.semantic.pool().phases();
        watched.pool_serial = watched.pool_serial.max(pool_serial);
        watched.announce(&phases);
    }

    /// Runs the work of a request for `revision`, which fails as stale, with the work
    /// dropped, as soon as a newer revision is known. Callers box the work, so that a
    /// request does not carry the size of every analysis in its own future.
    async fn current<T>(&self, revision: u32, work: Pin<Box<impl Future<Output = Fallible<T>>>>) -> Fallible<T> {
        match self.revisions.until_superseded(revision, work).await {
            Ok(done) => done,
            Err(current) => Err(SessionError::Stale { current }),
        }
    }

    pub async fn hover(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Option<String>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(view, revision).await?;
                Ok(Answer::of(revision, pair.hover(side, at).await))
            }),
        )
        .await
    }

    pub async fn definition(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Vec<Location>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(view, revision).await?;
                let asked = pair.definition(side, at).await.map(|answered| {
                    answered.map(|mut found| {
                        prefer_openable(&mut found);
                        found
                    })
                });
                Ok(Answer::of(revision, asked))
            }),
        )
        .await
    }

    /// The references to the symbol at `at`, without its declaration, each with the symbol it is in.
    pub async fn references(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Vec<Reference>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(view, revision).await?;
                Ok(Self::within(revision, side, &pair, pair.references(side, at).await).await)
            }),
        )
        .await
    }

    /// The implementations of the interface, trait or abstract member at `at`, each with the
    /// symbol it is in.
    pub async fn implementations(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Vec<Reference>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(view, revision).await?;
                Ok(Self::within(revision, side, &pair, pair.implementations(side, at).await).await)
            }),
        )
        .await
    }

    async fn within(
        revision: u32,
        side: Side,
        pair: &Pair<M, L>,
        asked: Asked<Vec<Location>>,
    ) -> Answer<Vec<Reference>> {
        match asked {
            Ok(answered) => Answer {
                revision,
                result: Ok(located(pair, side, answered.value).await.value),
                provenance: Some(answered.provenance),
                complete: answered.complete,
            },
            Err(e) => Answer::of(revision, Err(e)),
        }
    }

    /// Diagnostics of a file of the target side.
    pub async fn diagnostics(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        file: &RepoPath,
    ) -> Fallible<Answer<Vec<Diagnostic>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(view, revision).await?;
                Ok(Answer::of(revision, pair.diagnostics(Side::Target, file).await))
            }),
        )
        .await
    }

    /// The symbols of the target side whose names match `query`, from the server of each language
    /// that has a file in the project: one that cannot answer says so and the others still do.
    pub async fn workspace_symbols(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        query: &str,
    ) -> Fallible<WorkspaceSymbols> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(view, revision).await?;
                let languages = self.project_languages(view, revision).await?;
                let found = self.semantic.workspace_symbols(&pair.target, &languages, query).await;
                Ok(WorkspaceSymbols {
                    revision,
                    languages: found
                        .into_iter()
                        .map(|(language, result)| LanguageSymbols { language, result })
                        .collect(),
                })
            }),
        )
        .await
    }

    /// Diagnostics of a file of the target side, each with its origin against the base side's
    /// (derived, §7.3). A file the change leaves alone is compared with itself on the base.
    pub async fn judged_diagnostics(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        file: &RepoPath,
    ) -> Fallible<Answer<Judgement>> {
        self.current(
            revision,
            Box::pin(async move {
                let files = view.changed_files(revision).await?;
                let diff = files.iter().find(|f| f.new_path.as_ref() == Some(file));
                let (old, hunks) = match diff {
                    Some(diff) => (diff.old_path.as_ref(), diff.hunks.as_slice()),
                    None => (Some(file), [].as_slice()),
                };
                let pair = self.pair(view, revision).await?;
                let judged = judged(&pair, old, file, hunks).await;
                Ok(Answer::derived(revision, judged.value))
            }),
        )
        .await
    }

    async fn project_languages(&self, view: &RevisionView<G, S>, revision: u32) -> Fallible<Vec<BackendId>> {
        if let Some((at, languages)) = &*self.languages.lock().expect("not poisoned")
            && *at == revision
        {
            return Ok(languages.clone());
        }
        let languages: Vec<BackendId> = view
            .target_file_per_key(revision, POOL_CAPACITY, |path| self.language_of(path).cloned())
            .await?
            .into_iter()
            .map(|(language, _)| language)
            .collect();
        *self.languages.lock().expect("not poisoned") = Some((revision, languages.clone()));
        Ok(languages)
    }

    /// Starts the servers for every language among the revision's changed
    /// files on the target side and for the other languages of the repository,
    /// all at once, then those of the base side, and returns when they are running.
    /// A server already running or starting is left alone. A server that
    /// fails to start is not reported here: the query that needs it reports it.
    pub async fn start_servers(&self, view: &RevisionView<G, S>, revision: u32) -> Fallible<()> {
        let _ = self.start_servers_of(view, revision, None).await?;
        Ok(())
    }

    /// Stops the servers that may execute project code under a policy the revision no longer
    /// has: trust was withdrawn (§7.5).
    pub async fn retire_disallowed(&self, view: &RevisionView<G, S>, revision: u32) -> Fallible<()> {
        let sides = view.semantic_sides(revision).await?;
        let (base, target) = served(&view.trust(&sides.source).await?, &sides);
        self.semantic
            .pool()
            .retire_executing_except(&[base.policy, target.policy])
            .await;
        Ok(())
    }

    /// Stops the servers of `language` at once, whatever they are doing, and starts the ones
    /// the revision needs anew. The requests that were in flight on the old servers fail.
    /// The error is the reason a new server did not start.
    pub async fn restart_now(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        language: &BackendId,
    ) -> Fallible<Result<(), SemanticError>> {
        self.semantic.pool().restart_now(language).await;
        self.start_servers_of(view, revision, Some(language)).await
    }

    async fn start_servers_of(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        only: Option<&BackendId>,
    ) -> Fallible<Result<(), SemanticError>> {
        let pair = self.pair(view, revision).await?;
        let files = view.changed_files(revision).await?;
        let repo = view
            .target_file_per_key(revision, POOL_CAPACITY, |path| self.language_of(path))
            .await?;
        let others: Vec<_> = repo.into_iter().map(|(_, path)| path).collect();
        Ok(start_servers_for(&pair, &files, &[Side::Base, Side::Target], &others, only).await)
    }

    fn content(&self, view: &RevisionView<G, S>) -> Content<G, S> {
        Content(self.texts.clone().unwrap_or_else(|| view.acquisition()))
    }

    /// What the changeset did to the symbols of the changed file `path`, from the texts of its
    /// sides: a revision that changed one file asks a server for that file only, and the
    /// symbols of a text are kept (I2).
    pub async fn file_symbols(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        path: &RepoPath,
    ) -> Fallible<FileSymbols> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let files = view.changed_files(revision).await?;
                let file = changed_file(&files, path)
                    .ok_or_else(|| SessionError::NotFound(format!("changed file {}", path.to_string_lossy())))?;
                let none = |gap| FileSymbols {
                    changes: Derived {
                        value: FileChanges::default(),
                        inputs: Default::default(),
                    },
                    gap,
                };
                let pair = self.pair(view, revision).await?;
                let Derived { value, inputs } = compared(&pair, &self.content(view), file).await;
                trace::log(
                    || format!("symbols of {} (both sides' texts)", path.to_string_lossy()),
                    started,
                );
                Ok(match value {
                    Ok(found) => FileSymbols {
                        changes: Derived {
                            value: found.changes,
                            inputs,
                        },
                        gap: None,
                    },
                    // A language no server handles has no symbols to know.
                    Err(SemanticError::NoBackend(_)) => none(None),
                    Err(e) => none(Some(e)),
                })
            }),
        )
        .await
    }

    /// Every symbol of the file `path` as the changeset leaves it, changed or not: a changed file's
    /// from the texts of both sides, with the symbols it lost and what the change did to each;
    /// any other file's from its target text, with no marks. A file no language handles has no
    /// outline to know (`NoBackend`).
    pub async fn file_outline(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        path: &RepoPath,
    ) -> Fallible<Answer<Vec<OutlineSymbol>>> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                if self.language_of(path).is_none() {
                    return Ok(Answer::of(revision, Err(SemanticError::NoBackend(path.clone()))));
                }
                let files = view.changed_files(revision).await?;
                let pair = self.pair(view, revision).await?;
                let result = match changed_file(&files, path) {
                    Some(file) => (compared(&pair, &self.content(view), file).await.value).map(|found| outline(&found)),
                    None => {
                        let text = (view.side_text(revision, Side::Target, path).await?)
                            .ok_or_else(|| SessionError::NotFound(format!("text of {}", path.to_string_lossy())))?;
                        unmarked(&pair, path, &text).await
                    }
                };
                trace::log(|| format!("outline of {}", path.to_string_lossy()), started);
                Ok(Answer::derived(revision, result))
            }),
        )
        .await
    }

    /// Every symbol of `text`, the content of the file `path` in a view the semantic layer holds no
    /// snapshots of (a commit): with no marks.
    pub async fn text_outline(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        path: &RepoPath,
        text: &str,
    ) -> Fallible<Answer<Vec<OutlineSymbol>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(view, revision).await?;
                Ok(Answer::derived(revision, unmarked(&pair, path, text).await))
            }),
        )
        .await
    }

    /// The symbol of the target side (the base side's if it exists there only) that holds `at`
    /// on `side`, and what became of it. A file the changeset left alone has its symbols from
    /// the server of its side.
    pub async fn symbol_at(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Option<Located>>> {
        self.current(
            revision,
            Box::pin(async move {
                let files = view.changed_files(revision).await?;
                let pair = self.pair(view, revision).await?;
                let on_side = |f: &&FileDiff| match side {
                    Side::Base => f.old_path.as_ref() == Some(&at.file),
                    Side::Target => f.new_path.as_ref() == Some(&at.file),
                };
                Ok(match files.iter().find(on_side) {
                    Some(file) => Answer::derived(
                        revision,
                        compared(&pair, &self.content(view), file)
                            .await
                            .value
                            .map(|found| symbol_at(&found, side, at.pos)),
                    ),
                    None => Answer::of(
                        revision,
                        pair.document_symbols(side, &at.file).await.map(|answered| {
                            answered.map(|symbols| {
                                enclosing(&symbols, at.pos).map(|symbol| Located {
                                    symbol: symbol.clone(),
                                    change: None,
                                })
                            })
                        }),
                    ),
                })
            }),
        )
        .await
    }

    /// The symbol `sym` by its identity: where the changeset changed its file, from the texts of
    /// both sides, else from its side's server.
    async fn resolve(&self, view: &RevisionView<G, S>, revision: u32, sym: &Sym) -> Fallible<Resolved> {
        let files = view.changed_files(revision).await?;
        let pair = self.pair(view, revision).await?;
        let not_found = || SessionError::NotFound(format!("symbol {sym}"));
        let Some(file) = changed_file(&files, &sym.file) else {
            let found = pair
                .document_symbols(Side::Target, &sym.file)
                .await
                .map_err(|e| SessionError::Invalid(e.to_string()))?;
            let symbol = found.value.into_iter().find(|s| s.sym == *sym).ok_or_else(not_found)?;
            return Ok(Resolved {
                symbol,
                side: Side::Target,
                change: None,
                base: None,
            });
        };
        let found = compared(&pair, &self.content(view), file)
            .await
            .value
            .map_err(|e| SessionError::Invalid(e.to_string()))?;
        locate(file, &found, sym).ok_or_else(not_found)
    }

    /// The signature of a changed symbol, from the side it exists on: the base for a removed one.
    pub async fn signature(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<Answer<Option<Signature>>> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let Resolved { symbol, side, .. } = self.resolve(view, revision, sym).await?;
                let pair = self.pair(view, revision).await?;
                let signed = signature(&pair, side, &symbol).await;
                trace::log(
                    || {
                        format!(
                            "signature {sym} (hover on the {side:?} side; the symbol's detail if the hover has none)"
                        )
                    },
                    started,
                );
                Ok(match signed {
                    Ok((signature, provenance)) => Answer {
                        provenance,
                        ..Answer::derived(revision, Ok(signature))
                    },
                    Err(e) => Answer::of(revision, Err(e)),
                })
            }),
        )
        .await
    }

    /// What became of the remaining references of a removed symbol.
    pub async fn removal_impact(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<Derived<RemovalImpact>> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let resolved = self.resolve(view, revision, sym).await?;
                if resolved.change != Some(Change::Removed) {
                    return Err(SessionError::Invalid("the symbol was not removed".into()));
                }
                let files = view.changed_files(revision).await?;
                let pair = self.pair(view, revision).await?;
                let removed = vet_semantic::derived::ChangedSymbol {
                    symbol: resolved.symbol,
                    change: Change::Removed,
                };
                let Derived { mut value, inputs } = removal_impact(&pair, &[removed], &files).await;
                let value = value.pop().expect("one symbol in, one impact out");
                trace::log(|| format!("removal impact {sym}"), started);
                Ok(Derived { value, inputs })
            }),
        )
        .await
    }

    /// The symbol `sym` of the target side, which a removed one is not.
    async fn on_target(&self, view: &RevisionView<G, S>, revision: u32, sym: &Sym) -> Fallible<Resolved> {
        let resolved = self.resolve(view, revision, sym).await?;
        if resolved.side == Side::Base {
            return Err(SessionError::Invalid("the symbol was removed".into()));
        }
        Ok(resolved)
    }

    /// The changed callers and callees of a symbol of the target side, and its unchanged direct
    /// callers if it changed; the symbol is returned with its position.
    pub async fn calls(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<(SymbolInfo, Derived<Calls>)> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let resolved = self.on_target(view, revision, sym).await?;
                let files = view.changed_files(revision).await?;
                let pair = self.pair(view, revision).await?;
                let changed = resolved.change.is_some();
                let derived = calls(
                    &pair,
                    &self.content(view),
                    &files,
                    &resolved.symbol,
                    resolved.base.as_ref(),
                    changed,
                    CALLS_BOUNDS,
                )
                .await;
                trace::log(
                    || format!("calls {sym} (incoming calls level by level, then outgoing; target and base)"),
                    started,
                );
                Ok((resolved.symbol, derived))
            }),
        )
        .await
    }

    /// The references to a symbol of the target side; the symbol is returned with its position.
    pub async fn usages(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<(SymbolInfo, Derived<Usages>)> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let resolved = self.on_target(view, revision, sym).await?;
                let pair = self.pair(view, revision).await?;
                let derived = usages(&pair, &resolved.symbol).await;
                trace::log(|| format!("usages {sym} (target: references)"), started);
                Ok((resolved.symbol, derived))
            }),
        )
        .await
    }

    /// Whether a symbol of the target side is reached from an entry point; the
    /// symbol is returned with its position.
    pub async fn reachability(
        &self,
        view: &RevisionView<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<(SymbolInfo, Derived<Reach>)> {
        self.current(revision, Box::pin(async move {
            let started = Instant::now();
            let resolved = self.on_target(view, revision, sym).await?;
            let pair = self.pair(view, revision).await?;
            let derived = reachability(&pair, &resolved.symbol, &EntryPoints::default(), REACH_BOUNDS).await;
            trace::log(
                || {
                    format!(
                        "reachability {sym} (target: incoming calls level by level, one after the other, at most {} deep)",
                        REACH_BOUNDS.max_depth
                    )
                },
                started,
            );
            Ok((resolved.symbol, derived))
        }))
        .await
    }

    /// Drops the servers of `language`: their executable or settings changed.
    pub async fn restart(&self, language: &BackendId) {
        self.semantic.pool().restart(language).await;
    }

    pub async fn shutdown(&self) {
        self.semantic.pool().shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vet_core::content::{Hash, SnapshotId};
    use vet_semantic::join::join;
    use vet_semantic::policy::AnalysisPolicy;
    use vet_semantic::ports::{
        Diagnostic, LanguageServer, LaunchError, Materialized, NotMaterializable, Pos, QueryError, Readiness,
        ServerConfig, ServerIdentity, Span, StartProgress, Started, SymbolKind, Tree, Warmup,
    };

    use vet_semantic::derived::ChangedSymbol;

    use super::*;
    use crate::changeset::{CommitId, RemoteKey, Source};
    use crate::pull_request::PullRequestRefs;
    use crate::session::tests::{FakeGit, Fixture, MemStore};
    use crate::trust::{Origin, PullRequestTrust};

    fn sides() -> SemanticSides {
        SemanticSides {
            base: SnapshotId(Hash([1; 32])),
            base_commit: CommitId("base".into()),
            target: SnapshotId(Hash([2; 32])),
            head: CommitId("head".into()),
            live_target: true,
            source: Source::WorkingTree,
        }
    }

    #[test]
    fn hints_are_not_counted_among_a_files_diagnostics() {
        let at = |severity| Diagnostic {
            span: Span {
                start: Pos { line: 0, col: 0 },
                end: Pos { line: 0, col: 1 },
            },
            severity,
            message: String::new(),
            source: None,
            code: None,
        };
        let all = [
            Severity::Error,
            Severity::Warning,
            Severity::Info,
            Severity::Hint,
            Severity::Hint,
        ]
        .map(at);
        assert_eq!(countable(&all), 3);
        assert_eq!(countable(&[]), 0);
    }

    #[test]
    fn a_side_built_on_an_untrusted_commit_refuses_executing_servers() {
        let local = Trust {
            project: false,
            origin: Origin::Local,
        };
        let (base, target) = served(&local, &sides());
        assert!(base.policy.allow_executing && target.policy.allow_executing);

        let pull_request = |trusted_head| Trust {
            project: false,
            origin: Origin::PullRequest(PullRequestTrust {
                fork: true,
                trusted_head,
            }),
        };
        let (base, target) = served(&pull_request(Some(CommitId("head".into()))), &sides());
        assert!(
            base.policy.allow_executing,
            "the base is an ancestor of the trusted head"
        );
        assert!(target.policy.allow_executing);

        let (base, target) = served(&pull_request(Some(CommitId("older".into()))), &sides());
        assert!(!base.policy.allow_executing && !target.policy.allow_executing);

        let (base, target) = served(&pull_request(None), &sides());
        assert!(!base.policy.allow_executing && !target.policy.allow_executing);
        assert_eq!(target.target, Target::Live);
    }

    #[derive(Default)]
    struct Counters {
        starting: AtomicUsize,
        peak_starting: AtomicUsize,
        started: AtomicUsize,
        /// Live-tree servers running.
        live_started: AtomicUsize,
        /// Snapshot servers that began starting with fewer than two live-tree servers running.
        snapshot_started_early: AtomicUsize,
        /// Symbol requests on the live side, which no cache answers.
        live_symbol_requests: AtomicUsize,
        /// Symbol requests for a text under a file's name.
        text_symbol_requests: AtomicUsize,
        /// Hovers are asked and never answered.
        hang_hover: std::sync::atomic::AtomicBool,
        hovers_asked: AtomicUsize,
        /// Hovers asked and given up on.
        hovers_dropped: AtomicUsize,
        /// Trees made for snapshots.
        materialized: AtomicUsize,
        /// A tree of a snapshot is on disk: made by this run, or kept by an earlier one.
        tree_kept: std::sync::atomic::AtomicBool,
        /// Added to what the server's binary is: another number is another binary.
        binary: std::sync::atomic::AtomicU8,
        /// Added to what the restore of a tree was: another number is another restore.
        restore: std::sync::atomic::AtomicU8,
        /// The tree's restore left no record.
        unrecorded: std::sync::atomic::AtomicBool,
        /// Identities worked out: each stands for a `--version` probe, a settings read and more.
        identified: AtomicUsize,
        /// The language whose server is not installed.
        missing: Mutex<Option<String>>,
        /// A search of symbols by name waits for another server's search to begin, as far as the
        /// other one is asked at all.
        await_peer: std::sync::atomic::AtomicBool,
        workspace_asked: AtomicUsize,
        /// Two searches of symbols by name were under way together.
        workspace_together: std::sync::atomic::AtomicBool,
        /// Starts of a server that may execute project code during analysis.
        executing_started: AtomicUsize,
        /// The languages of the servers that were stopped, in order.
        stopped: Mutex<Vec<String>>,
    }

    /// Counts a hover as given up on when its request is dropped.
    struct HoverAsked(Arc<Counters>);

    impl Drop for HoverAsked {
        fn drop(&mut self) {
            self.0.hovers_dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeLauncher(Arc<Counters>);

    /// A server of the language named last.
    struct FakeServer(Arc<Counters>, Tree, String);

    impl ServerLauncher for FakeLauncher {
        type Server = FakeServer;

        /// Slow, so starts that overlap are seen to.
        async fn start(
            &self,
            root: &Path,
            tree: Tree,
            backend: &Backend,
            policy: AnalysisPolicy,
            _: &dyn StartProgress,
        ) -> Result<Started<FakeServer>, LaunchError> {
            let c = &self.0;
            let language = &backend.config().backend.0;
            if c.missing.lock().unwrap().as_ref() == Some(language) {
                return Err(LaunchError::NotInstalled(format!("no server for {language} was found")));
            }
            if backend.config().executes_project_code_during_analysis {
                c.executing_started.fetch_add(1, Ordering::SeqCst);
            }
            if tree == Tree::Immutable && c.live_started.load(Ordering::SeqCst) < 2 {
                c.snapshot_started_early.fetch_add(1, Ordering::SeqCst);
            }
            let now = c.starting.fetch_add(1, Ordering::SeqCst) + 1;
            c.peak_starting.fetch_max(now, Ordering::SeqCst);
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            c.starting.fetch_sub(1, Ordering::SeqCst);
            c.started.fetch_add(1, Ordering::SeqCst);
            if tree == Tree::Live {
                c.live_started.fetch_add(1, Ordering::SeqCst);
            }
            Ok(Started {
                server: FakeServer(c.clone(), tree, language.clone()),
                identity: self.identify(root, backend, policy).await?,
                restore: match tree {
                    Tree::Immutable => self.recorded_restore(root, backend, policy).await,
                    Tree::Live => None,
                },
            })
        }

        async fn identify(
            &self,
            _: &Path,
            backend: &Backend,
            _: AnalysisPolicy,
        ) -> Result<ServerIdentity, LaunchError> {
            self.0.identified.fetch_add(1, Ordering::SeqCst);
            Ok(ServerIdentity {
                backend: backend.config().backend.clone(),
                fingerprint: Hash([7 + self.0.binary.load(Ordering::SeqCst); 32]),
                label: format!("fake {}", backend.config().binary),
            })
        }

        async fn recorded_restore(&self, _: &Path, _: &Backend, _: AnalysisPolicy) -> Option<Hash> {
            let c = &self.0;
            (!c.unrecorded.load(Ordering::SeqCst)).then(|| Hash([9 + c.restore.load(Ordering::SeqCst); 32]))
        }
    }

    fn symbol(file: &RepoPath) -> SymbolInfo {
        SymbolInfo {
            sym: Sym {
                file: file.clone(),
                name: vec!["f".into()],
            },
            span: Span {
                start: Pos { line: 0, col: 0 },
                end: Pos { line: 5, col: 0 },
            },
            at: Pos { line: 0, col: 0 },
            detail: None,
            kind: SymbolKind::Function,
            modules: 0,
        }
    }

    impl LanguageServer for FakeServer {
        fn readiness(&self) -> Readiness {
            Readiness { idle: true, epoch: 0 }
        }
        fn features(&self) -> Features {
            Features::ALL
        }
        async fn document_symbols(&self, file: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
            if self.1 == Tree::Live {
                self.0.live_symbol_requests.fetch_add(1, Ordering::SeqCst);
            }
            Ok(vec![symbol(file)])
        }
        async fn text_symbols(&self, file: &RepoPath, text: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            self.0.text_symbol_requests.fetch_add(1, Ordering::SeqCst);
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            let mut symbol = symbol(file);
            symbol.detail = Some(text.trim().to_owned());
            Ok(vec![symbol])
        }
        async fn definition(&self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        async fn references(&self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        async fn implementations(&self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        async fn hover(&self, _: &At) -> Result<Option<String>, QueryError> {
            if self.0.hang_hover.load(Ordering::SeqCst) {
                self.0.hovers_asked.fetch_add(1, Ordering::SeqCst);
                let _asked = HoverAsked(self.0.clone());
                std::future::pending::<()>().await;
            }
            Ok(None)
        }
        async fn incoming_calls(&self, _: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn outgoing_calls(&self, _: &At) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            if self.0.await_peer.load(Ordering::SeqCst) {
                let first = self.0.workspace_asked.fetch_add(1, Ordering::SeqCst) == 0;
                for _ in 0..100 {
                    if first && self.0.workspace_asked.load(Ordering::SeqCst) >= 2 {
                        self.0.workspace_together.store(true, Ordering::SeqCst);
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }
            let mut found = symbol(&RepoPath(b"a.rs".to_vec()));
            found.sym.name = vec![query.to_owned()];
            Ok(vec![found])
        }
        async fn diagnostics(&self, _: &RepoPath) -> Result<Vec<Diagnostic>, QueryError> {
            Ok(vec![])
        }
        async fn shutdown(&self) {
            self.0.stopped.lock().unwrap().push(self.2.clone());
        }
    }

    /// Serves every snapshot from one fixed directory.
    struct FakeMaterializer(Arc<Counters>);

    impl Materializer for FakeMaterializer {
        type Guard = ();

        async fn materialize(&self, _: SnapshotId, _: AnalysisPolicy) -> Result<Materialized<()>, NotMaterializable> {
            self.0.materialized.fetch_add(1, Ordering::SeqCst);
            self.0.tree_kept.store(true, Ordering::SeqCst);
            Ok(Materialized {
                root: "/snap".into(),
                guard: (),
            })
        }

        async fn kept(&self, _: SnapshotId, _: AnalysisPolicy) -> Option<std::path::PathBuf> {
            self.0.tree_kept.load(Ordering::SeqCst).then(|| "/snap".into())
        }
    }

    fn backend(id: &str, extension: &str) -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId(id.into()),
            binary: id.into(),
            args: vec![],
            filetypes: vec![extension.into()],
            file_names: vec![],
            requires: vec![],
            root_markers: vec![],
            executes_project_code_during_analysis: id == "go",
            loads_project_on_open: false,
            warmup: Warmup::Quick,
        })
    }

    type Harness = (
        Fixture,
        Semantics<FakeGit, MemStore, FakeLauncher, FakeMaterializer>,
        Arc<Counters>,
    );

    /// Servers for Rust, Python and Go; Markdown has none.
    async fn semantics_over(base: &[(&str, &str)], target: &[(&str, &str)]) -> Harness {
        semantics_keeping(vet_semantic::reuse::KEPT, base, target).await
    }

    /// Like `semantics_over`, keeping at most `limit` answers.
    async fn semantics_keeping(limit: usize, base: &[(&str, &str)], target: &[(&str, &str)]) -> Harness {
        let fx = Fixture::open(base, target).await;
        let counters = Arc::new(Counters::default());
        let semantics = Semantics::new(
            FakeMaterializer(counters.clone()),
            FakeLauncher(counters.clone()),
            "/live".into(),
            vec![backend("rust", "rs"), backend("python", "py"), backend("go", "go")],
            Links {
                revisions: fx.session.revisions(),
                limits: RestoreLimits::default(),
            },
        )
        .keeping(limit);
        (fx, semantics, counters)
    }

    /// Changes in a Rust, a Python and a Markdown file.
    async fn semantics() -> Harness {
        semantics_over(
            &[("a.rs", "one\n"), ("b.py", "one\n"), ("c.md", "one\n")],
            &[("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")],
        )
        .await
    }

    #[tokio::test]
    async fn the_target_servers_of_every_language_start_at_once_and_the_base_side_after_them() {
        let (fx, semantics, counters) = semantics().await;
        semantics.start_servers(&fx.view, 0).await.unwrap();
        assert_eq!(counters.started.load(Ordering::SeqCst), 4);
        assert_eq!(counters.peak_starting.load(Ordering::SeqCst), 2);
        assert_eq!(counters.snapshot_started_early.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_target_of_a_branch_is_served_from_a_snapshot_and_never_from_the_live_tree() {
        let (fx, semantics, counters) = semantics().await;
        let base = [("a.rs", "one\n"), ("b.py", "one\n"), ("c.md", "one\n")];
        let tip = [("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")];
        fx.git.feature(&base, &tip);

        let revision = fx
            .session
            .open_changeset(Source::Branch { name: "feature".into() })
            .await
            .unwrap()
            .unwrap();
        semantics.start_servers(&fx.view, revision).await.unwrap();

        assert_eq!(counters.live_started.load(Ordering::SeqCst), 0);
        assert_eq!(
            counters.started.load(Ordering::SeqCst),
            4,
            "Rust and Python, both sides"
        );
    }

    #[tokio::test]
    async fn servers_start_for_languages_that_are_in_the_repository_but_not_in_the_diff() {
        let (fx, semantics, counters) = semantics_over(
            &[("a.rs", "one\n"), ("main.go", "same\n")],
            &[("a.rs", "two\n"), ("main.go", "same\n")],
        )
        .await;
        semantics.start_servers(&fx.view, 0).await.unwrap();
        // Rust on both sides, Go on the target side only.
        assert_eq!(counters.started.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn starting_servers_again_leaves_the_running_ones_alone() {
        let (fx, semantics, counters) = semantics().await;
        semantics.start_servers(&fx.view, 0).await.unwrap();
        semantics.start_servers(&fx.view, 0).await.unwrap();
        assert_eq!(counters.started.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_search_across_the_languages_stops_servers_of_unchanged_languages_and_not_those_the_change_needs() {
        let languages: Vec<String> = (0..7).map(|n| format!("l{n}")).collect();
        let file = |n: usize, text: &'static str| -> (String, &'static str) { (format!("f.l{n}"), text) };
        let base: Vec<_> = (0..7).map(|n| file(n, "one\n")).collect();
        let target: Vec<_> = (0..7).map(|n| file(n, if n < 5 { "two\n" } else { "one\n" })).collect();
        let (base, target): (Vec<_>, Vec<_>) = (
            base.iter().map(|(p, t)| (p.as_str(), *t)).collect(),
            target.iter().map(|(p, t)| (p.as_str(), *t)).collect(),
        );
        let fx = Fixture::open(&base, &target).await;
        let counters = Arc::new(Counters::default());
        let semantics = Semantics::new(
            FakeMaterializer(counters.clone()),
            FakeLauncher(counters.clone()),
            "/live".into(),
            languages.iter().map(|l| backend(l, l)).collect(),
            Links {
                revisions: fx.session.revisions(),
                limits: RestoreLimits::default(),
            },
        );
        // Five changed languages on two sides want ten servers, and the pool holds eight.
        semantics.start_servers(&fx.view, 0).await.unwrap();
        assert!(counters.stopped.lock().unwrap().is_empty());

        semantics.workspace_symbols(&fx.view, 0, "x").await.unwrap();
        let mut stopped = counters.stopped.lock().unwrap().clone();
        stopped.sort();
        assert_eq!(stopped, ["l4", "l5", "l6"]);
    }

    #[tokio::test]
    async fn what_the_servers_of_the_revision_on_view_do_is_reported_per_language_and_side() {
        let fx = Fixture::open(
            &[("a.l0", "one\n"), ("a.l1", "one\n")],
            &[("a.l0", "two\n"), ("a.l1", "two\n")],
        )
        .await;
        let counters = Arc::new(Counters::default());
        let told = Arc::new(Mutex::new(Vec::<ServerRuntimes>::new()));
        let semantics = Semantics::new(
            FakeMaterializer(counters.clone()),
            FakeLauncher(counters.clone()),
            "/live".into(),
            vec![backend("l0", "l0"), backend("l1", "l1")],
            Links {
                revisions: fx.session.revisions(),
                limits: RestoreLimits::default(),
            },
        )
        .observing({
            let told = told.clone();
            Arc::new(move |runtimes| told.lock().unwrap().push(runtimes))
        });
        assert!(semantics.runtimes().runtimes.is_empty());

        semantics.start_servers(&fx.view, 0).await.unwrap();
        let told = told.lock().unwrap();
        assert!(told.windows(2).all(|w| w[0].serial < w[1].serial), "{told:?}");
        let last = told.last().unwrap();
        let shown: Vec<_> = last
            .runtimes
            .iter()
            .map(|r| (r.language.0.as_str(), r.side, r.phase.clone()))
            .collect();
        assert_eq!(
            shown,
            [
                ("l0", Side::Base, Phase::Ready),
                ("l0", Side::Target, Phase::Ready),
                ("l1", Side::Base, Phase::Ready),
                ("l1", Side::Target, Phase::Ready),
            ]
        );
        assert_eq!(semantics.runtimes(), *last);
    }

    #[tokio::test]
    async fn an_answer_names_the_server_that_gave_it_and_a_derived_result_shows_it() {
        let (fx, semantics, _) = semantics().await;
        let at = At {
            file: RepoPath(b"a.rs".to_vec()),
            pos: Pos { line: 0, col: 0 },
        };
        let hover = semantics.hover(&fx.view, 0, Side::Target, &at).await.unwrap();
        let by = hover.provenance.unwrap();
        assert_eq!(by.server.label, "fake rust");
        assert!(!by.from_cache);

        let path = |p: &str| RepoPath(p.as_bytes().to_vec());
        let rust = semantics.file_symbols(&fx.view, 0, &path("a.rs")).await.unwrap();
        assert_eq!(
            rust.changes.inputs.primitives,
            [
                "document_symbols(base:a.rs) [fake rust]",
                "document_symbols(target:a.rs) [fake rust]",
            ]
        );
    }

    #[tokio::test]
    async fn symbols_by_name_come_from_the_servers_of_the_projects_languages_only() {
        // Rust and Python are in the project; a Go server is configured, but no Go file is there.
        let (fx, semantics, counters) = semantics().await;
        let found = semantics.workspace_symbols(&fx.view, 0, "retry").await.unwrap();
        let answers: Vec<_> = found
            .languages
            .iter()
            .map(|l| {
                (
                    l.language.0.as_str(),
                    l.result.as_ref().map(|s| s[0].sym.name.clone()).ok(),
                )
            })
            .collect();
        assert_eq!(
            answers,
            [
                ("rust", Some(vec!["retry".to_string()])),
                ("python", Some(vec!["retry".to_string()]))
            ]
        );
        assert_eq!(found.revision, 0);
        assert_eq!(counters.started.load(Ordering::SeqCst), 2, "no Go server was started");
    }

    #[tokio::test]
    async fn a_language_whose_server_cannot_start_is_reported_and_the_others_still_answer() {
        let (fx, semantics, counters) = semantics().await;
        *counters.missing.lock().unwrap() = Some("python".into());
        let found = semantics.workspace_symbols(&fx.view, 0, "retry").await.unwrap();
        let [rust, python] = &found.languages[..] else {
            panic!("{} languages", found.languages.len())
        };
        assert_eq!(rust.result.as_ref().map(Vec::len).ok(), Some(1));
        assert_eq!(
            python.result.as_ref().err().map(ToString::to_string),
            Some("no server for python was found".to_string())
        );
    }

    #[tokio::test]
    async fn the_servers_of_the_languages_are_asked_together() {
        let (fx, semantics, counters) = semantics().await;
        counters.await_peer.store(true, Ordering::SeqCst);
        semantics.workspace_symbols(&fx.view, 0, "retry").await.unwrap();
        assert!(counters.workspace_together.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_pull_request_that_is_not_trusted_is_not_searched_with_a_server_that_runs_its_code() {
        let (fx, semantics, counters) = semantics_over(
            &[("a.rs", "one\n"), ("main.go", "one\n")],
            &[("a.rs", "one\n"), ("main.go", "one\n")],
        )
        .await;
        let source = Source::PullRequest {
            remote: RemoteKey {
                host: "github.com".into(),
                path: "acme/app".into(),
            },
            number: 7,
        };
        let Source::PullRequest { remote, number } = &source else {
            unreachable!()
        };
        let refs = PullRequestRefs::of(remote, *number);
        let files = [("a.rs", "two\n"), ("main.go", "two\n")];
        fx.git.add_commit(&refs.head, "pr1", &files);
        fx.git.add_commit(&format!("merge-base {}", refs.target), "m1", &files);
        let revision = fx.session.open_changeset(source).await.unwrap().unwrap();

        let found = semantics.workspace_symbols(&fx.view, revision, "retry").await.unwrap();
        let [rust, go] = &found.languages[..] else {
            panic!("{} languages", found.languages.len())
        };
        assert!(rust.result.is_ok());
        assert!(matches!(
            go.result,
            Err(SemanticError::Pool(vet_semantic::pool::PoolError::AnalysisLimited(_)))
        ));
        assert_eq!(counters.executing_started.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_diagnostics_of_an_untrusted_pull_request_are_not_judged_where_its_server_runs_its_code() {
        let files = [("a.rs", "one\n"), ("main.go", "one\n")];
        let (fx, semantics, _) = semantics_over(&files, &files).await;
        let remote = RemoteKey {
            host: "github.com".into(),
            path: "acme/app".into(),
        };
        let refs = PullRequestRefs::of(&remote, 7);
        let changed = [("a.rs", "two\n"), ("main.go", "two\n")];
        fx.git.add_commit(&refs.head, "pr1", &changed);
        fx.git
            .add_commit(&format!("merge-base {}", refs.target), "m1", &changed);
        let revision = fx
            .session
            .open_changeset(Source::PullRequest { remote, number: 7 })
            .await
            .unwrap()
            .unwrap();
        let go = semantics
            .judged_diagnostics(&fx.view, revision, &RepoPath::from("main.go"))
            .await
            .unwrap();
        assert!(matches!(
            go.result,
            Err(SemanticError::Pool(vet_semantic::pool::PoolError::AnalysisLimited(_)))
        ));
        let rust = semantics
            .judged_diagnostics(&fx.view, revision, &RepoPath::from("a.rs"))
            .await
            .unwrap();
        assert!(rust.result.unwrap().base.is_none());
    }

    #[tokio::test]
    async fn symbols_by_name_for_a_revision_that_is_not_current_are_refused() {
        let (fx, semantics, _) = semantics().await;
        assert!(semantics.workspace_symbols(&fx.view, 9, "retry").await.is_err());
    }

    /// What happens to the session that moves its revision on.
    enum Bump {
        Settle,
        Filter,
        FinishReview,
        OpenChangeset,
    }

    impl Bump {
        async fn happens(&self, fx: &Fixture) {
            match self {
                Self::Settle => {
                    fx.settle(&[("a.rs", "three\n"), ("b.py", "two\n"), ("c.md", "two\n")])
                        .await;
                }
                Self::Filter => {
                    fx.session.exclude_path("c.md", false).await.unwrap();
                }
                Self::FinishReview => {
                    fx.session.finish_review().await.unwrap();
                }
                Self::OpenChangeset => {
                    let base = [("a.rs", "one\n"), ("b.py", "one\n"), ("c.md", "one\n")];
                    let tip = [("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")];
                    fx.git.feature(&base, &tip);
                    fx.session
                        .open_changeset(Source::Branch { name: "feature".into() })
                        .await
                        .unwrap();
                }
            }
        }
    }

    #[tokio::test]
    async fn every_way_the_session_moves_its_revision_drops_the_work_for_the_older_one_with_its_server_request() {
        for bump in [Bump::Settle, Bump::Filter, Bump::FinishReview, Bump::OpenChangeset] {
            let (fx, semantics, counters) = semantics().await;
            fx.begin().await;
            counters.hang_hover.store(true, Ordering::SeqCst);
            let at = At {
                file: RepoPath(b"a.rs".to_vec()),
                pos: Pos { line: 0, col: 0 },
            };
            let hover = semantics.hover(&fx.view, 0, Side::Target, &at);
            let moved = async {
                while counters.hovers_asked.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
                bump.happens(&fx).await;
            };
            let (hover, ()) = join(hover, moved).await;
            let now = fx.session.revision().await;
            assert!(
                matches!(hover, Err(SessionError::Stale { current }) if current == now && now > 0),
                "{hover:?}"
            );
            assert_eq!(counters.hovers_dropped.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn derived_work_for_an_older_revision_is_refused_without_asking_a_server() {
        let (fx, semantics, counters) = semantics().await;
        fx.session.revisions().advance(3);
        let stale = semantics.file_symbols(&fx.view, 0, &RepoPath(b"a.rs".to_vec())).await;
        assert!(matches!(stale, Err(SessionError::Stale { current: 3 })));
        assert_eq!(counters.started.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_callers_for_the_symbols_of_a_file_ask_the_server_once_per_text() {
        let (fx, semantics, counters) = semantics().await;
        let path = RepoPath(b"a.rs".to_vec());
        let (first, second) = join(
            semantics.file_symbols(&fx.view, 0, &path),
            semantics.file_symbols(&fx.view, 0, &path),
        )
        .await;
        let (first, second) = (first.unwrap(), second.unwrap());
        assert_eq!(first.changes.value, second.changes.value);
        assert_eq!(first.changes.value.symbols.len(), 1);
        // Both sides are read from their texts, one request per text and not per caller;
        // the live files are not asked, and no server is started for the base.
        assert_eq!(counters.text_symbol_requests.load(Ordering::SeqCst), 2);
        assert_eq!(counters.live_symbol_requests.load(Ordering::SeqCst), 0);
        assert_eq!(counters.started.load(Ordering::SeqCst), 1);

        let third = semantics.file_symbols(&fx.view, 0, &path).await.unwrap();
        assert_eq!(third.changes.value, first.changes.value);
        assert_eq!(counters.text_symbol_requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_symbols_of_a_file_ask_a_server_for_that_files_texts_only() {
        let (fx, semantics, counters) = semantics().await;
        let path = |p: &str| RepoPath(p.as_bytes().to_vec());

        let found = semantics.file_symbols(&fx.view, 0, &path("a.rs")).await.unwrap();
        let changed: Vec<_> = found.changes.value.symbols.iter().map(|c| c.change).collect();
        assert_eq!((changed, found.gap), (vec![Change::Modified], None));
        assert_eq!(
            counters.text_symbol_requests.load(Ordering::SeqCst),
            2,
            "a.rs, both sides"
        );

        // No language handles Markdown: no symbols to know, and no gap.
        let notes = semantics.file_symbols(&fx.view, 0, &path("c.md")).await.unwrap();
        assert!(notes.changes.value.symbols.is_empty() && notes.gap.is_none());

        let unchanged = semantics.file_symbols(&fx.view, 0, &path("untouched.rs")).await;
        assert!(matches!(unchanged, Err(SessionError::NotFound(_))));
    }

    #[tokio::test]
    async fn the_outline_marks_a_changed_file_and_reads_any_other_from_its_text() {
        let (fx, semantics, counters) = semantics_over(
            &[("a.rs", "one\n"), ("u.rs", "same\n"), ("c.md", "one\n")],
            &[("a.rs", "two\n"), ("u.rs", "same\n"), ("c.md", "two\n")],
        )
        .await;
        let path = |p: &str| RepoPath(p.as_bytes().to_vec());
        let marks = |found: Answer<Vec<OutlineSymbol>>| -> Vec<Option<Change>> {
            found.result.unwrap().iter().map(|o| o.change).collect()
        };

        let changed = semantics.file_outline(&fx.view, 0, &path("a.rs")).await.unwrap();
        assert_eq!(marks(changed), [Some(Change::Modified)]);
        let unchanged = semantics.file_outline(&fx.view, 0, &path("u.rs")).await.unwrap();
        assert_eq!(marks(unchanged), [None]);
        // Both texts of a.rs and the one text of u.rs.
        assert_eq!(counters.text_symbol_requests.load(Ordering::SeqCst), 3);

        let notes = semantics.file_outline(&fx.view, 0, &path("c.md")).await.unwrap();
        assert!(matches!(notes.result, Err(SemanticError::NoBackend(_))));
        let missing = semantics.file_outline(&fx.view, 0, &path("gone.rs")).await;
        assert!(matches!(missing, Err(SessionError::NotFound(_))));
    }

    #[tokio::test]
    async fn the_outline_of_a_text_has_no_marks_and_a_stale_revision_is_refused() {
        let (fx, semantics, _) = semantics().await;
        let file = RepoPath(b"a.rs".to_vec());
        let found = semantics.text_outline(&fx.view, 0, &file, "text").await.unwrap();
        let found = found.result.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].change, None);

        fx.session.revisions().advance(3);
        let stale = semantics.text_outline(&fx.view, 0, &file, "text").await;
        assert!(matches!(stale, Err(SessionError::Stale { current: 3 })));
    }

    #[tokio::test]
    async fn the_symbol_at_a_position_of_a_changed_file_is_found_in_the_texts_of_the_sides() {
        let (fx, semantics, _) = semantics().await;
        let at = |line| At {
            file: RepoPath(b"a.rs".to_vec()),
            pos: Pos { line, col: 1 },
        };
        let inside = semantics.symbol_at(&fx.view, 0, Side::Target, &at(2)).await.unwrap();
        let inside = inside.result.unwrap().unwrap();
        assert_eq!(
            (inside.symbol.sym.name, inside.change),
            (vec!["f".to_string()], Some(Change::Modified))
        );
        let outside = semantics.symbol_at(&fx.view, 0, Side::Target, &at(9)).await.unwrap();
        assert_eq!(outside.result.unwrap(), None);
    }

    #[test]
    fn a_renamed_files_symbols_are_found_by_the_path_of_their_side() {
        let path = |p: &str| RepoPath(p.as_bytes().to_vec());
        let named = |file: &str, name: &str| {
            let mut info = symbol(&path(file));
            info.sym.name = vec![name.into()];
            info
        };
        let file = FileDiff {
            old_path: Some(path("a.rs")),
            new_path: Some(path("b.rs")),
            old_blob: None,
            new_blob: None,
            hunks: vec![],
        };
        let (kept, gone) = (named("b.rs", "keep"), named("a.rs", "gone"));
        let found = Compared {
            base: vec![named("a.rs", "keep"), gone.clone()],
            target: vec![kept.clone()],
            changes: FileChanges {
                symbols: vec![
                    ChangedSymbol {
                        symbol: kept,
                        change: Change::Modified,
                    },
                    ChangedSymbol {
                        symbol: gone,
                        change: Change::Removed,
                    },
                ],
                context: vec![],
            },
        };
        let find = |at: &str, name: &str| {
            locate(
                &file,
                &found,
                &Sym {
                    file: path(at),
                    name: vec![name.into()],
                },
            )
        };
        let kept = find("b.rs", "keep").unwrap();
        assert_eq!((kept.side, kept.change), (Side::Target, Some(Change::Modified)));
        assert!(kept.base.is_some());
        let gone = find("a.rs", "gone").unwrap();
        assert_eq!((gone.side, gone.change), (Side::Base, Some(Change::Removed)));
        // The old path names what the base side has and the target does not.
        assert!(find("a.rs", "keep").is_none());
        assert!(find("b.rs", "gone").is_none());
    }

    #[tokio::test]
    async fn an_unchanged_files_symbols_come_from_the_server_of_the_side_asked() {
        let (fx, semantics, counters) = semantics_over(
            &[("a.rs", "one\n"), ("u.rs", "same\n")],
            &[("a.rs", "two\n"), ("u.rs", "same\n")],
        )
        .await;
        let at = |line| At {
            file: RepoPath(b"u.rs".to_vec()),
            pos: Pos { line, col: 1 },
        };
        for side in [Side::Base, Side::Target] {
            let found = semantics.symbol_at(&fx.view, 0, side, &at(2)).await.unwrap();
            let found = found.result.unwrap().unwrap();
            assert_eq!(
                (found.symbol.sym.name, found.change),
                (vec!["f".to_string()], None),
                "{side:?}"
            );
            let outside = semantics.symbol_at(&fx.view, 0, side, &at(9)).await.unwrap();
            assert_eq!(outside.result.unwrap(), None, "{side:?}");
        }
        // Its symbols are not read from texts: nothing changed there to compare.
        assert_eq!(counters.text_symbol_requests.load(Ordering::SeqCst), 0);

        let sym = Sym {
            file: RepoPath(b"u.rs".to_vec()),
            name: vec!["f".into()],
        };
        let (info, around) = semantics.calls(&fx.view, 0, &sym).await.unwrap();
        assert_eq!(info.sym, sym);
        assert!(around.value.callers.is_empty() && around.value.affected.is_empty());
    }

    /// What the changeset did to the symbols of each of the revision's changed files with hunks.
    async fn changes_of_all(fx: &Fixture, semantics: &FakeSemantics, revision: u32) -> Vec<FileChanges> {
        let files = fx.view.changed_files(revision).await.unwrap();
        let mut all = Vec::new();
        for file in files.iter().filter(|f| !f.hunks.is_empty()) {
            let path = file.new_path.as_ref().or(file.old_path.as_ref()).unwrap();
            all.push(
                semantics
                    .file_symbols(&fx.view, revision, path)
                    .await
                    .unwrap()
                    .changes
                    .value,
            );
        }
        all
    }

    // Whether a kept answer can serve a text's symbols needs the server's identity, which on
    // the real launcher runs toolchain probes and reads the server settings: it is worked out
    // once, not per text.
    #[tokio::test]
    async fn the_identity_of_a_server_is_worked_out_independently_of_how_many_files_changed() {
        async fn identities(files: usize) -> usize {
            let names: Vec<_> = (0..files).map(|i| format!("r{i}.rs")).collect();
            let before: Vec<_> = names.iter().map(|p| (p.as_str(), "one\n")).collect();
            let after: Vec<_> = names.iter().map(|p| (p.as_str(), "two\n")).collect();
            let (fx, semantics, counters) = semantics_over(&before, &after).await;
            changes_of_all(&fx, &semantics, 0).await;
            counters.identified.load(Ordering::SeqCst)
        }
        assert_eq!(identities(2).await, identities(16).await);
    }

    #[tokio::test]
    async fn a_revision_asks_for_the_symbols_of_the_texts_that_are_new_and_of_no_other() {
        let (fx, semantics, counters) = semantics().await;
        changes_of_all(&fx, &semantics, 0).await;
        let asked = || counters.text_symbol_requests.load(Ordering::SeqCst);
        assert_eq!(asked(), 4);

        // One file is edited: its new text is asked, and nothing else.
        let revision = fx
            .settle(&[("a.rs", "three\n"), ("b.py", "two\n"), ("c.md", "two\n")])
            .await;
        changes_of_all(&fx, &semantics, revision).await;
        assert_eq!(asked(), 5);

        // Nothing about the content changes.
        let revision = fx.session.exclude_path("c.md", false).await.unwrap();
        changes_of_all(&fx, &semantics, revision).await;
        assert_eq!(asked(), 5);

        // The edit is undone: the text was seen before.
        let revision = fx
            .settle(&[("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")])
            .await;
        changes_of_all(&fx, &semantics, revision).await;
        assert_eq!(asked(), 5);
    }

    #[tokio::test]
    async fn the_changed_symbols_are_the_same_with_answers_kept_and_without() {
        async fn run(limit: usize) -> (Vec<Vec<FileChanges>>, usize) {
            let (fx, semantics, counters) = semantics_keeping(
                limit,
                &[("a.rs", "one\n"), ("b.py", "one\n"), ("c.md", "one\n")],
                &[("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")],
            )
            .await;
            let mut results = vec![changes_of_all(&fx, &semantics, 0).await];
            for edit in ["three\n", "two\n"] {
                let revision = fx.settle(&[("a.rs", edit), ("b.py", "two\n"), ("c.md", "two\n")]).await;
                results.push(changes_of_all(&fx, &semantics, revision).await);
            }
            let revision = fx.session.exclude_path("c.md", false).await.unwrap();
            results.push(changes_of_all(&fx, &semantics, revision).await);
            (results, counters.text_symbol_requests.load(Ordering::SeqCst))
        }
        let (kept, asked_kept) = run(vet_semantic::reuse::KEPT).await;
        let (not_kept, asked_not_kept) = run(0).await;
        assert_eq!(kept, not_kept);
        assert!(asked_kept < asked_not_kept, "{asked_kept} < {asked_not_kept}");
    }

    /// Answers kept by the key, as a stand-in for the file.
    #[derive(Default)]
    struct MemoryCache(Mutex<std::collections::HashMap<vet_semantic::reuse::Key, vet_semantic::reuse::Value>>);

    impl SemanticCache for MemoryCache {
        fn get<'a>(
            &'a self,
            key: &'a vet_semantic::reuse::Key,
        ) -> Pin<Box<dyn Future<Output = Option<vet_semantic::reuse::Value>> + Send + 'a>> {
            Box::pin(async move { self.0.lock().unwrap().get(key).cloned() })
        }

        fn put(&self, key: &vet_semantic::reuse::Key, value: &vet_semantic::reuse::Value) {
            self.0.lock().unwrap().insert(key.clone(), value.clone());
        }
    }

    #[tokio::test]
    async fn a_session_in_a_later_launch_gets_the_same_changes_without_asking_for_symbols_stored_before() {
        let before = [("a.rs", "one\n"), ("b.py", "one\n"), ("c.md", "one\n")];
        let after = [("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")];
        let (fx, plain, _) = semantics_over(&before, &after).await;
        let expected = changes_of_all(&fx, &plain, 0).await;

        let store = Arc::new(MemoryCache::default());
        let mut launched = Vec::new();
        for _ in 0..2 {
            let counters = Arc::new(Counters::default());
            let semantics = Semantics::new(
                FakeMaterializer(counters.clone()),
                FakeLauncher(counters.clone()),
                "/live".into(),
                vec![backend("rust", "rs"), backend("python", "py"), backend("go", "go")],
                Links {
                    revisions: fx.session.revisions(),
                    limits: RestoreLimits::default(),
                },
            )
            .persisting(store.clone());
            let changes = changes_of_all(&fx, &semantics, 0).await;
            assert_eq!(changes, expected);
            launched.push(counters.text_symbol_requests.load(Ordering::SeqCst));
        }
        assert_eq!(launched, [4, 0]);
    }

    type FakeSemantics = Semantics<FakeGit, MemStore, FakeLauncher, FakeMaterializer>;

    /// Asks the base side what a snapshot's server answers and a store keeps: the answer
    /// of each and whether it was reused.
    async fn ask_the_base(fx: &Fixture, semantics: &FakeSemantics) -> Vec<(String, bool)> {
        let file = RepoPath(b"a.rs".to_vec());
        let at = At {
            file: file.clone(),
            pos: Pos { line: 0, col: 0 },
        };
        let mut answers = Vec::new();
        let hover = semantics.hover(&fx.view, 0, Side::Base, &at).await.unwrap();
        answers.push((format!("{:?}", hover.result), hover.provenance.unwrap().from_cache));
        let definition = semantics.definition(&fx.view, 0, Side::Base, &at).await.unwrap();
        answers.push((
            format!("{:?}", definition.result),
            definition.provenance.unwrap().from_cache,
        ));
        let references = semantics.references(&fx.view, 0, Side::Base, &at).await.unwrap();
        answers.push((
            format!("{:?}", references.result),
            references.provenance.unwrap().from_cache,
        ));
        let pair = semantics.pair(&fx.view, 0).await.unwrap();
        let diagnostics = pair.diagnostics(Side::Base, &file).await.unwrap();
        answers.push((format!("{:?}", diagnostics.value), diagnostics.provenance.from_cache));
        let symbols = pair.document_symbols(Side::Base, &file).await.unwrap();
        answers.push((format!("{:?}", symbols.value), symbols.provenance.from_cache));
        answers
    }

    #[tokio::test]
    async fn reopening_a_review_answers_the_base_from_the_store_until_an_input_of_the_answers_changed() {
        type Change = fn(&Counters);
        // What changed since the first launch, how many servers the reopened one starts, and
        // how many of the five answers it reuses.
        let changes: [(&str, Change, usize, usize); 5] = [
            ("nothing", |_| {}, 0, 5),
            ("the server binary", |c| c.binary.store(1, Ordering::SeqCst), 1, 0),
            (
                "the restore of the tree",
                |c| c.restore.store(1, Ordering::SeqCst),
                1,
                0,
            ),
            (
                "the record of the restore",
                |c| c.unrecorded.store(true, Ordering::SeqCst),
                1,
                0,
            ),
            // The server that is started to ask finds the answers in the store too.
            ("the kept tree", |c| c.tree_kept.store(false, Ordering::SeqCst), 1, 5),
        ];
        for (name, change, starts, reused) in changes {
            let (fx, ..) = semantics().await;
            let store = Arc::new(MemoryCache::default());
            let launch = |counters: &Arc<Counters>| -> FakeSemantics {
                Semantics::new(
                    FakeMaterializer(counters.clone()),
                    FakeLauncher(counters.clone()),
                    "/live".into(),
                    vec![backend("rust", "rs")],
                    Links {
                        revisions: fx.session.revisions(),
                        limits: RestoreLimits::default(),
                    },
                )
                .persisting(store.clone())
            };

            let first = Arc::new(Counters::default());
            let answers = ask_the_base(&fx, &launch(&first)).await;
            assert!(answers.iter().all(|(_, reused)| !reused), "{name}");
            assert_eq!(
                first.started.load(Ordering::SeqCst),
                1,
                "{name}: the first launch starts"
            );
            assert_eq!(first.materialized.load(Ordering::SeqCst), 1, "{name}");

            let reopened = Arc::new(Counters::default());
            reopened.tree_kept.store(true, Ordering::SeqCst);
            change(&reopened);
            let again = ask_the_base(&fx, &launch(&reopened)).await;
            let values = |all: &[(String, bool)]| all.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
            assert_eq!(values(&again), values(&answers), "{name}");
            assert_eq!(
                reopened.started.load(Ordering::SeqCst),
                starts,
                "{name}: servers started"
            );
            assert_eq!(
                reopened.materialized.load(Ordering::SeqCst),
                starts,
                "{name}: snapshots materialized"
            );
            assert_eq!(
                again.iter().filter(|(_, reused)| *reused).count(),
                reused,
                "{name}: answers reused"
            );
        }
    }
}
