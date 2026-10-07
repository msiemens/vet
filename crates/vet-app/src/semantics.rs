//! Semantic queries of a session (§7.3): the side being viewed is asked
//! directly, derived analyses combine the answers with the revision's diff.
//! Positions and results belong to a revision; a stale one is refused like any
//! other request. Base answers come from the session's base snapshot, target
//! answers from the live tree, which is the revision's target until the next
//! snapshot settles.

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;
use vet_core::content::{FileDiff, RepoPath};
use vet_core::review::Side;
use vet_semantic::derived::{
    BaseTexts, Bounds, Change, Changes, Derived, EntryPoints, Reach, RemovalImpact, SymbolImpact, changed_symbols,
    impact, reachability, removal_impact,
};
use vet_semantic::join::{bounded, join};
use vet_semantic::pool::{Pool, Target};
use vet_semantic::ports::{
    At, Backend, BackendId, Diagnostic, Location, Materializer, RestoreFailure, ServerLauncher, Severity, Sym,
    SymbolInfo, prefer_openable,
};
use vet_semantic::query::{Pair, Primitives, Semantic, SemanticError, Served};
use vet_semantic::signature::{Signature, signature};

use crate::acquisition::Acquisition;
use crate::limits::RestoreLimits;
use crate::ports::{BlobRead, BlobReader, GitRepo, ReviewStore, Store};
use crate::session::{LiveSession, SemanticSides, SessionError};
use crate::trust::{Trust, analysis_policy};

/// Servers kept warm per session: base and target of each language.
const POOL_CAPACITY: usize = 8;

/// Reachability stops at the first entry point within this many calls and symbols.
const REACH_BOUNDS: Bounds = Bounds {
    max_depth: 8,
    max_nodes: 200,
};

/// An answer to a primitive query, at a revision. The query itself can fail
/// without the request failing: the language may have no usable server.
#[derive(Debug)]
pub struct Answer<T> {
    pub revision: u32,
    pub result: Result<T, SemanticError>,
}

type Fallible<T> = Result<T, SessionError>;

/// How many of a file's diagnostics count towards what a viewer is told: hints are not counted.
pub fn countable(diagnostics: &[Diagnostic]) -> u32 {
    diagnostics.iter().filter(|d| d.severity != Severity::Hint).count() as u32
}

type ChangesCell = OnceCell<Arc<Derived<Changes>>>;

pub struct Semantics<G, S, L, M>
where
    G: GitRepo + Send + Sync,
    S: Store + Send + Sync,
    L: ServerLauncher,
    M: Materializer,
{
    semantic: Semantic<M, L>,
    _session: PhantomData<fn(&LiveSession<G, S>)>,
    /// The changed symbols of one revision, which every derived analysis starts
    /// from. Computed once, however many callers ask while it runs.
    changes: Mutex<Option<(u32, Arc<ChangesCell>)>>,
    /// Kept current by the launcher's restorer (`Limiting`).
    limits: RestoreLimits,
    trust: Trust,
}

/// The base side's file contents, as the acquisition keeps them.
struct BaseContent<G, S>(Arc<Acquisition<G, S>>);

impl<G: BlobReader + Send + Sync, S: Store + Send + Sync> BaseTexts for BaseContent<G, S> {
    async fn base_text(&self, file: &FileDiff) -> Option<String> {
        let (path, blob) = (file.old_path.as_ref()?, file.old_blob?);
        match self.0.blob(path, blob, true).await {
            Ok(BlobRead::Found(bytes)) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            _ => None,
        }
    }
}

/// The base and the target side as the pool serves them: each may execute
/// project code as far as `trust` holds for the commit it is built on.
fn served(trust: &Trust, sides: &SemanticSides) -> (Served, Served) {
    (
        Served {
            target: Target::Snapshot(sides.base),
            policy: analysis_policy(trust, &sides.base_commit),
        },
        Served {
            target: Target::Live,
            policy: analysis_policy(trust, &sides.head),
        },
    )
}

impl<G, S, L, M> Semantics<G, S, L, M>
where
    G: GitRepo + Send + Sync,
    S: Store + ReviewStore + Send + Sync,
    L: ServerLauncher + Sync,
    L::Server: Send,
    M: Materializer + Sync,
{
    /// `materializer` serves the session's snapshots as directories and
    /// `live_root` is the worktree the target side is served from. `trust`
    /// decides which sides may have their servers execute project code.
    /// `limits` is the one the launcher's restorer updates.
    pub fn new(
        materializer: M,
        launcher: L,
        live_root: std::path::PathBuf,
        backends: Vec<Backend>,
        limits: RestoreLimits,
        trust: Trust,
    ) -> Self {
        let pool = Pool::new(materializer, launcher, live_root, POOL_CAPACITY);
        Self {
            semantic: Semantic::new(pool, backends),
            _session: PhantomData,
            changes: Mutex::new(None),
            limits,
            trust,
        }
    }

    /// The language that serves `file`; `None`: it is unsupported.
    pub fn language_of(&self, file: &RepoPath) -> Option<&BackendId> {
        self.semantic.language_of(file)
    }

    /// Restores that failed and have not succeeded since.
    pub fn restore_failures(&self) -> Vec<RestoreFailure> {
        self.limits.all()
    }

    async fn pair(&self, session: &LiveSession<G, S>, revision: u32) -> Fallible<vet_semantic::query::Pair<'_, M, L>> {
        let (base, target) = served(&self.trust, &session.semantic_sides(revision).await?);
        Ok(self.semantic.pair(base, target))
    }

    pub async fn hover(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Option<String>>> {
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: pair.hover(side, at).await,
        })
    }

    pub async fn definition(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Vec<Location>>> {
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: pair.definition(side, at).await.map(|mut found| {
                prefer_openable(&mut found);
                found
            }),
        })
    }

    pub async fn references(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Vec<Location>>> {
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: pair.references(side, at).await,
        })
    }

    /// Diagnostics of a file of the target side.
    pub async fn diagnostics(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        file: &RepoPath,
    ) -> Fallible<Answer<Vec<Diagnostic>>> {
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: pair.diagnostics(Side::Target, file).await,
        })
    }

    /// Starts the servers for every language among the revision's changed
    /// files on the target side and for the other languages of the repository,
    /// all at once, then those of the base side, and returns when they are running.
    /// A server already running or starting is left alone. A server that
    /// fails to start is not reported here: the query that needs it reports it.
    pub async fn start_servers(&self, session: &LiveSession<G, S>, revision: u32) -> Fallible<()> {
        let pair = self.pair(session, revision).await?;
        let files = session.changed_files(revision).await?;
        let repo = session
            .target_file_per_key(revision, POOL_CAPACITY, |path| self.language_of(path))
            .await?;
        let others: Vec<_> = repo.into_iter().map(|(_, path)| path).collect();
        self.start_servers_for(&pair, &files, &[Side::Base, Side::Target], &others)
            .await;
        Ok(())
    }

    /// Starts what the changed `files` need on `sides`, and the target side's
    /// servers for the languages of `others`. Changed languages come first,
    /// because only that many fit in the pool.
    async fn start_servers_for(&self, pair: &Pair<'_, M, L>, files: &[FileDiff], sides: &[Side], others: &[RepoPath]) {
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
        candidates.extend(others.iter().map(|path| (Side::Target, &pair.target, path)));

        let mut wanted: Vec<(Side, &Served, &RepoPath)> = Vec::new();
        let mut languages: Vec<(Side, &BackendId)> = Vec::new();
        for (side, served, path) in candidates {
            let Some(language) = self.language_of(path) else {
                continue;
            };
            if !languages.contains(&(side, language)) {
                languages.push((side, language));
                wanted.push((side, served, path));
            }
        }
        // Servers beyond what the pool keeps warm would only evict each other.
        wanted.truncate(POOL_CAPACITY);
        // The base side's servers restore and index a snapshot, which slows the
        // target side's startup down, and the target side is what the first
        // views need. So they start once the target side's are running, and
        // are warm by the time anyone hovers the base side.
        for phase in [Side::Target, Side::Base] {
            let phase: Vec<_> = wanted.iter().filter(|(side, ..)| *side == phase).collect();
            let _ = bounded(POOL_CAPACITY, 0..phase.len(), |at| {
                let (_, served, file) = phase[at];
                self.semantic.start_for(served, file)
            })
            .await;
        }
    }

    /// The symbols the revision's diff touches, on both sides of each changed
    /// file. Callers asking for the same revision share one computation.
    pub async fn changed_symbols(&self, session: &LiveSession<G, S>, revision: u32) -> Fallible<Arc<Derived<Changes>>> {
        let pair = self.pair(session, revision).await?;
        let computation = {
            let mut slot = self.changes.lock().expect("not poisoned");
            match &*slot {
                Some((cached, computation)) if *cached == revision => computation.clone(),
                _ => {
                    let computation = Arc::default();
                    *slot = Some((revision, Arc::clone(&computation)));
                    computation
                }
            }
        };
        computation
            .get_or_try_init(|| async {
                let files = session.changed_files(revision).await?;
                // The base side's symbols come from the target side's server, so
                // the base side's servers, which restore the base first, are not waited for.
                let texts = BaseContent(session.acquisition());
                // Only the target side's servers are queried. They start together, while
                // the files are queried, so a language late in the list does not wait for
                // the servers of the languages before it.
                let (_, changes) = join(
                    self.start_servers_for(&pair, &files, &[Side::Target], &[]),
                    changed_symbols(&pair, &texts, &files),
                )
                .await;
                Ok::<_, SessionError>(Arc::new(changes))
            })
            .await
            .cloned()
    }

    /// The changed symbol `sym` and whether it was added, modified or removed.
    async fn changed(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<vet_semantic::derived::ChangedSymbol> {
        self.changed_symbols(session, revision)
            .await?
            .value
            .symbols
            .iter()
            .find(|c| &c.symbol.sym == sym)
            .cloned()
            .ok_or_else(|| SessionError::NotFound(format!("changed symbol {sym}")))
    }

    /// The signature of a changed symbol, from the side it exists on: the base for a removed one.
    pub async fn signature(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<Answer<Option<Signature>>> {
        let changed = self.changed(session, revision, sym).await?;
        let side = if changed.change == Change::Removed {
            Side::Base
        } else {
            Side::Target
        };
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: signature(&pair, side, &changed.symbol).await,
        })
    }

    /// References and callers of an added or modified symbol, on the target side.
    pub async fn impact(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<Derived<SymbolImpact>> {
        let changed = self.changed(session, revision, sym).await?;
        if changed.change == Change::Removed {
            return Err(SessionError::Invalid(
                "the symbol was removed; ask for its removal impact".into(),
            ));
        }
        let pair = self.pair(session, revision).await?;
        let Derived { mut value, inputs } = impact(&pair, &[changed]).await;
        let value = value.pop().expect("one symbol in, one impact out");
        Ok(Derived { value, inputs })
    }

    /// What became of the remaining references of a removed symbol.
    pub async fn removal_impact(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<Derived<RemovalImpact>> {
        let changed = self.changed(session, revision, sym).await?;
        if changed.change != Change::Removed {
            return Err(SessionError::Invalid("the symbol was not removed".into()));
        }
        let files = session.changed_files(revision).await?;
        let pair = self.pair(session, revision).await?;
        let Derived { mut value, inputs } = removal_impact(&pair, &[changed], &files).await;
        let value = value.pop().expect("one symbol in, one impact out");
        Ok(Derived { value, inputs })
    }

    /// Whether an added or modified symbol is reached from an entry point; the
    /// symbol is returned with its position.
    pub async fn reachability(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<(SymbolInfo, Derived<Reach>)> {
        let changed = self.changed(session, revision, sym).await?;
        if changed.change == Change::Removed {
            return Err(SessionError::Invalid("a removed symbol has nothing to reach".into()));
        }
        let pair = self.pair(session, revision).await?;
        let derived = reachability(&pair, &changed.symbol, &EntryPoints::default(), REACH_BOUNDS).await;
        Ok((changed.symbol, derived))
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vet_core::content::{Hash, SnapshotId};
    use vet_semantic::join::join;
    use vet_semantic::policy::AnalysisPolicy;
    use vet_semantic::ports::{
        Diagnostic, LanguageServer, LaunchError, Materialized, NotMaterializable, Pos, QueryError, ServerConfig, Span,
        Tree,
    };

    use super::*;
    use crate::pin::CommitId;
    use crate::session::tests::{FakeGit, Fixture, MemStore};

    fn sides() -> SemanticSides {
        SemanticSides {
            base: SnapshotId(Hash([1; 32])),
            base_commit: CommitId("base".into()),
            target: SnapshotId(Hash([2; 32])),
            head: CommitId("head".into()),
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
    fn a_local_tree_may_execute_on_both_sides() {
        let (base, target) = served(&Trust::Local, &sides());
        assert!(base.policy.allow_executing && target.policy.allow_executing);
    }

    #[test]
    fn a_side_built_on_an_untrusted_commit_refuses_executing_servers() {
        let trust = Trust::Contributor {
            trusted_head: Some(CommitId("head".into())),
        };
        let (base, target) = served(&trust, &sides());
        assert!(!base.policy.allow_executing);
        assert!(target.policy.allow_executing);

        let (_, target) = served(&Trust::Contributor { trusted_head: None }, &sides());
        assert!(!target.policy.allow_executing);
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
        /// The servers running when the first symbol request was answered; 0 before.
        started_at_first_answer: AtomicUsize,
        /// Symbol requests on the live side, which no cache answers.
        live_symbol_requests: AtomicUsize,
        /// Symbol requests for a text under a file's name.
        text_symbol_requests: AtomicUsize,
    }

    struct FakeLauncher(Arc<Counters>);

    struct FakeServer(Arc<Counters>, Tree);

    impl ServerLauncher for FakeLauncher {
        type Server = FakeServer;

        /// Slow, so starts that overlap are seen to.
        async fn start(&self, _: &Path, tree: Tree, _: &Backend, _: AnalysisPolicy) -> Result<FakeServer, LaunchError> {
            let c = &self.0;
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
            Ok(FakeServer(c.clone(), tree))
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
        }
    }

    impl LanguageServer for FakeServer {
        async fn document_symbols(&self, file: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
            if self.1 == Tree::Live {
                self.0.live_symbol_requests.fetch_add(1, Ordering::SeqCst);
            }
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            let started = self.0.started.load(Ordering::SeqCst);
            let _ = (self.0.started_at_first_answer).compare_exchange(0, started, Ordering::SeqCst, Ordering::SeqCst);
            Ok(vec![symbol(file)])
        }
        async fn text_symbols(&self, file: &RepoPath, _: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            assert_eq!(self.1, Tree::Live, "a text is asked of the live tree's server");
            self.0.text_symbol_requests.fetch_add(1, Ordering::SeqCst);
            Ok(vec![symbol(file)])
        }
        async fn definition(&self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        async fn references(&self, _: &At) -> Result<Vec<Location>, QueryError> {
            Ok(vec![])
        }
        async fn hover(&self, _: &At) -> Result<Option<String>, QueryError> {
            Ok(None)
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
        async fn shutdown(&self) {}
    }

    /// Serves every snapshot from one fixed directory.
    struct FakeMaterializer;

    impl Materializer for FakeMaterializer {
        type Guard = ();

        async fn materialize(&self, _: SnapshotId) -> Result<Materialized<()>, NotMaterializable> {
            Ok(Materialized {
                root: "/snap".into(),
                guard: (),
            })
        }
    }

    fn backend(id: &str, extension: &str) -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId(id.into()),
            binary: id.into(),
            args: vec![],
            filetypes: vec![extension.into()],
            root_markers: vec![],
            executes_project_code_during_analysis: false,
        })
    }

    type Harness = (
        Fixture,
        Semantics<FakeGit, MemStore, FakeLauncher, FakeMaterializer>,
        Arc<Counters>,
    );

    /// Servers for Rust, Python and Go; Markdown has none.
    async fn semantics_over(base: &[(&str, &str)], target: &[(&str, &str)]) -> Harness {
        let fx = Fixture::open(base, target).await;
        let counters = Arc::new(Counters::default());
        let semantics = Semantics::new(
            FakeMaterializer,
            FakeLauncher(counters.clone()),
            "/live".into(),
            vec![backend("rust", "rs"), backend("python", "py"), backend("go", "go")],
            RestoreLimits::default(),
            Trust::Local,
        );
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
        semantics.start_servers(&fx.session, 0).await.unwrap();
        assert_eq!(counters.started.load(Ordering::SeqCst), 4);
        assert_eq!(counters.peak_starting.load(Ordering::SeqCst), 2);
        assert_eq!(counters.snapshot_started_early.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn servers_start_for_languages_that_are_in_the_repository_but_not_in_the_diff() {
        let (fx, semantics, counters) = semantics_over(
            &[("a.rs", "one\n"), ("main.go", "same\n")],
            &[("a.rs", "two\n"), ("main.go", "same\n")],
        )
        .await;
        semantics.start_servers(&fx.session, 0).await.unwrap();
        // Rust on both sides, Go on the target side only.
        assert_eq!(counters.started.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn starting_servers_again_leaves_the_running_ones_alone() {
        let (fx, semantics, counters) = semantics().await;
        semantics.start_servers(&fx.session, 0).await.unwrap();
        semantics.start_servers(&fx.session, 0).await.unwrap();
        assert_eq!(counters.started.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn changed_symbols_starts_the_servers_of_later_languages_before_the_first_file_is_answered() {
        // More Rust files than are queried at once, so the Python file waits its turn.
        let rust: Vec<_> = (0..8).map(|i| format!("r{i}.rs")).collect();
        let before: Vec<_> = rust
            .iter()
            .map(|p| (p.as_str(), "one\n"))
            .chain([("z.py", "one\n")])
            .collect();
        let after: Vec<_> = rust
            .iter()
            .map(|p| (p.as_str(), "two\n"))
            .chain([("z.py", "two\n")])
            .collect();
        let (fx, semantics, counters) = semantics_over(&before, &after).await;
        semantics.changed_symbols(&fx.session, 0).await.unwrap();
        assert_eq!(counters.started_at_first_answer.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_computation_of_the_changed_symbols() {
        let (fx, semantics, counters) = semantics().await;
        let (first, second) = join(
            semantics.changed_symbols(&fx.session, 0),
            semantics.changed_symbols(&fx.session, 0),
        )
        .await;
        let (first, second) = (first.unwrap(), second.unwrap());
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.value.symbols.len(), 2);
        assert_eq!(first.value.unsupported.len(), 1);
        // One request per live file, not one per caller.
        assert_eq!(counters.live_symbol_requests.load(Ordering::SeqCst), 2);
        // The base side is read from its text; no server is started for it.
        assert_eq!(counters.text_symbol_requests.load(Ordering::SeqCst), 2);
        assert_eq!(counters.started.load(Ordering::SeqCst), 2);

        let third = semantics.changed_symbols(&fx.session, 0).await.unwrap();
        assert!(Arc::ptr_eq(&first, &third));
    }
}
