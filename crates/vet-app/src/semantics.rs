//! Semantic queries of a session (§7.3): the side being viewed is asked
//! directly, derived analyses combine the answers with the revision's diff.
//! Positions and results belong to a revision; a stale one is refused like any
//! other request. Base answers come from the session's base snapshot. Target
//! answers come from the live tree, which is the revision's target until the next
//! snapshot settles, or, when the target is a commit's tree, from a snapshot.

use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use vet_core::content::{FileDiff, RepoPath};
use vet_core::review::Side;
use vet_semantic::derived::{
    Bounds, Change, Changes, Derived, EntryPoints, Reach, RemovalImpact, SymbolImpact, Texts, changed_symbols, impact,
    reachability, removal_impact,
};
use vet_semantic::join::{bounded, join};
use vet_semantic::pool::{Pool, Target};
use vet_semantic::ports::{
    At, Backend, BackendId, Diagnostic, Location, Materializer, Provenance, RestoreFailure, ServerLauncher, Severity,
    Sym, SymbolInfo, prefer_openable,
};
use vet_semantic::query::{Asked, Pair, Primitives, Semantic, SemanticError, Served};
use vet_semantic::reuse::{Computed, Memo, SemanticCache};
use vet_semantic::signature::{Signature, signature};
use vet_semantic::trace;

use crate::acquisition::Acquisition;
use crate::limits::RestoreLimits;
use crate::ports::{BlobRead, BlobReader, GitRepo, LiveStore, Store};
use crate::progress::{TaskKind, Tasks};
use crate::revisions::Revisions;
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
    /// The server that answered and whether the answer was reused (I6); none if no
    /// server did, or the answer is made of more than one server's.
    pub provenance: Option<Provenance>,
}

impl<T> Answer<T> {
    fn of(revision: u32, asked: Asked<T>) -> Self {
        match asked {
            Ok(answered) => Self {
                revision,
                result: Ok(answered.value),
                provenance: Some(answered.provenance),
            },
            Err(e) => Self {
                revision,
                result: Err(e),
                provenance: None,
            },
        }
    }
}

type Fallible<T> = Result<T, SessionError>;

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
    /// Told how far the changed-symbols analysis is.
    pub tasks: Tasks,
}

pub struct Semantics<G, S, L, M>
where
    G: GitRepo + Send + Sync,
    S: Store + Send + Sync,
    L: ServerLauncher,
    M: Materializer,
{
    semantic: Semantic<M, L>,
    _session: PhantomData<fn(&LiveSession<G, S>)>,
    revisions: Arc<Revisions>,
    /// Kept current by the launcher's restorer (`Limiting`).
    limits: RestoreLimits,
    trust: Trust,
    /// Where the changed-symbols analysis reports its files done.
    tasks: Tasks,
    /// The changed symbols of the revision being viewed, keyed by it and by the
    /// `epoch` of the servers they were found with: every analysis of the revision
    /// joins one search. The memo keeps the latest revision only, so a session that
    /// moves on drops the older one's result; the search itself lives in the futures
    /// of its callers, so it ends with the last of them (see `changed_symbols`).
    changes: Memo<(u32, u64), Derived<Changes>>,
    /// Bumped when the servers or their answers are replaced: what was found with the
    /// old ones is not reused.
    epoch: AtomicU64,
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
/// project code as far as `trust` holds for the commit it is built on. The target
/// is served from the live tree only when it is the live tree; a commit's tree is
/// materialized like the base.
fn served(trust: &Trust, sides: &SemanticSides) -> (Served, Served) {
    (
        Served {
            target: Target::Snapshot(sides.base),
            policy: analysis_policy(trust, &sides.base_commit),
        },
        Served {
            target: if sides.live_target {
                Target::Live
            } else {
                Target::Snapshot(sides.target)
            },
            policy: analysis_policy(trust, &sides.head),
        },
    )
}

impl<G, S, L, M> Semantics<G, S, L, M>
where
    G: GitRepo + Send + Sync,
    S: LiveStore + Send + Sync,
    L: ServerLauncher,
    M: Materializer,
{
    /// `materializer` serves the session's snapshots as directories and
    /// `live_root` is the worktree the target side is served from. `trust`
    /// decides which sides may have their servers execute project code.
    pub fn new(
        materializer: M,
        launcher: L,
        live_root: std::path::PathBuf,
        backends: Vec<Backend>,
        trust: Trust,
        links: Links,
    ) -> Self {
        let pool = Pool::new(materializer, launcher, live_root, POOL_CAPACITY);
        Self {
            semantic: Semantic::new(pool, backends),
            _session: PhantomData,
            revisions: links.revisions,
            limits: links.limits,
            trust,
            tasks: links.tasks,
            changes: Memo::new(1),
            epoch: AtomicU64::new(0),
        }
    }

    /// Keeps at most `limit` answers of the servers for reuse; 0 keeps none.
    pub fn keeping(mut self, limit: usize) -> Self {
        self.semantic = self.semantic.keeping(limit);
        self
    }

    /// Keeps the answers that persist in `store`, behind the ones in memory.
    pub fn persisting(mut self, store: Arc<dyn SemanticCache>) -> Self {
        self.semantic = self.semantic.persisting(store);
        self
    }

    /// Drops the answers kept in memory; the next question is asked again (or read from the store).
    pub fn forget_answers(&self) {
        self.replaced();
        self.semantic.pool().forget_answers();
    }

    /// What was derived from the servers' answers so far is not reused.
    fn replaced(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.changes.clear();
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
        session: &LiveSession<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Option<String>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(session, revision).await?;
                Ok(Answer::of(revision, pair.hover(side, at).await))
            }),
        )
        .await
    }

    pub async fn definition(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Vec<Location>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(session, revision).await?;
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

    pub async fn references(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        side: Side,
        at: &At,
    ) -> Fallible<Answer<Vec<Location>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(session, revision).await?;
                Ok(Answer::of(revision, pair.references(side, at).await))
            }),
        )
        .await
    }

    /// Diagnostics of a file of the target side.
    pub async fn diagnostics(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        file: &RepoPath,
    ) -> Fallible<Answer<Vec<Diagnostic>>> {
        self.current(
            revision,
            Box::pin(async move {
                let pair = self.pair(session, revision).await?;
                Ok(Answer::of(revision, pair.diagnostics(Side::Target, file).await))
            }),
        )
        .await
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
    /// file. They are made from the symbols of the files' texts, which are asked of a
    /// server only if no answer for that text is kept (I2): a revision that changed
    /// one file asks for that one, however many it has, and one that changed none
    /// (a filter, a finished review) asks for none.
    ///
    /// All callers for a revision share one search with one progress task. The caller
    /// that came first runs it and the others wait for it; if that caller is dropped
    /// (its request was cancelled), a waiter runs it next, and when the last caller is
    /// dropped the search stops, keeping the answers it got per text (I2). A revision
    /// that is superseded drops every caller and with them the search.
    pub async fn changed_symbols(&self, session: &LiveSession<G, S>, revision: u32) -> Fallible<Derived<Changes>> {
        let key = (revision, self.epoch.load(Ordering::SeqCst));
        let work = async {
            let found = self
                .changes
                .get(&key, async {
                    self.find_changes(session, revision)
                        .await
                        .map(|value| Computed { value, keep: true })
                })
                .await;
            found.map(|(changes, _)| changes)
        };
        self.current(revision, Box::pin(work)).await
    }

    async fn find_changes(&self, session: &LiveSession<G, S>, revision: u32) -> Fallible<Derived<Changes>> {
        let pair = self.pair(session, revision).await?;
        let started = Instant::now();
        let files = session.changed_files(revision).await?;
        let texts = Content(session.acquisition());
        let analysis = self.tasks.start(TaskKind::Analyse, "Finding the changed symbols");
        // Only the target side's servers are queried. They start together, while
        // the files are queried, so a language late in the list does not wait for
        // the servers of the languages before it.
        let (_, changes) = join(
            self.start_servers_for(&pair, &files, &[Side::Target], &[]),
            changed_symbols(&pair, &texts, &files, &|done, total| {
                analysis.report(
                    Some((done as u32, total as u32)),
                    Some(format!("{done} of {total} files")),
                );
            }),
        )
        .await;
        analysis.finish();
        trace::log(
            || {
                format!(
                    "changed symbols of revision {revision} ({} files: the symbols of both sides' texts on the target server, in parallel per file; target servers start alongside)",
                    files.len()
                )
            },
            started,
        );
        Ok(changes)
    }

    /// The changed symbol `sym` and whether it was added, modified or removed,
    /// for the `analysis` that asks. The time it takes is the wait for the
    /// revision's changed symbols, which every analysis shares.
    async fn changed(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        analysis: &str,
        sym: &Sym,
    ) -> Fallible<vet_semantic::derived::ChangedSymbol> {
        let started = Instant::now();
        let changes = self.changed_symbols(session, revision).await?;
        trace::log(|| format!("{analysis} {sym}: wait for the changed symbols"), started);
        changes
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
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let changed = self.changed(session, revision, "signature", sym).await?;
                let side = if changed.change == Change::Removed {
                    Side::Base
                } else {
                    Side::Target
                };
                let pair = self.pair(session, revision).await?;
                let signed = signature(&pair, side, &changed.symbol).await;
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
                        revision,
                        result: Ok(signature),
                        provenance,
                    },
                    Err(e) => Answer::of(revision, Err(e)),
                })
            }),
        )
        .await
    }

    /// References and callers of an added or modified symbol, on the target side.
    pub async fn impact(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<Derived<SymbolImpact>> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let changed = self.changed(session, revision, "impact", sym).await?;
                if changed.change == Change::Removed {
                    return Err(SessionError::Invalid(
                        "the symbol was removed; ask for its removal impact".into(),
                    ));
                }
                let pair = self.pair(session, revision).await?;
                let Derived { mut value, inputs } = impact(&pair, &[changed]).await;
                let value = value.pop().expect("one symbol in, one impact out");
                trace::log(
                    || format!("impact {sym} (target: references, then incoming calls, one after the other)"),
                    started,
                );
                Ok(Derived { value, inputs })
            }),
        )
        .await
    }

    /// What became of the remaining references of a removed symbol.
    pub async fn removal_impact(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<Derived<RemovalImpact>> {
        self.current(
            revision,
            Box::pin(async move {
                let started = Instant::now();
                let changed = self.changed(session, revision, "removal impact", sym).await?;
                if changed.change != Change::Removed {
                    return Err(SessionError::Invalid("the symbol was not removed".into()));
                }
                let files = session.changed_files(revision).await?;
                let pair = self.pair(session, revision).await?;
                let Derived { mut value, inputs } = removal_impact(&pair, &[changed], &files).await;
                let value = value.pop().expect("one symbol in, one impact out");
                trace::log(|| format!("removal impact {sym}"), started);
                Ok(Derived { value, inputs })
            }),
        )
        .await
    }

    /// Whether an added or modified symbol is reached from an entry point; the
    /// symbol is returned with its position.
    pub async fn reachability(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        sym: &Sym,
    ) -> Fallible<(SymbolInfo, Derived<Reach>)> {
        self.current(revision, Box::pin(async move {
            let started = Instant::now();
            let changed = self.changed(session, revision, "reachability", sym).await?;
            if changed.change == Change::Removed {
                return Err(SessionError::Invalid("a removed symbol has nothing to reach".into()));
            }
            let pair = self.pair(session, revision).await?;
            let derived = reachability(&pair, &changed.symbol, &EntryPoints::default(), REACH_BOUNDS).await;
            trace::log(
                || {
                    format!(
                        "reachability {sym} (target: incoming calls level by level, one after the other, at most {} deep)",
                        REACH_BOUNDS.max_depth
                    )
                },
                started,
            );
            Ok((changed.symbol, derived))
        }))
        .await
    }

    /// Drops the servers of `language`: their executable or settings changed.
    pub async fn restart(&self, language: &BackendId) {
        self.replaced();
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
        ServerConfig, ServerIdentity, Span, Started, Tree,
    };

    use super::*;
    use crate::changeset::{BaseChoice, CommitId, Source};
    use crate::progress::TaskState;
    use crate::session::tests::{FakeGit, Fixture, MemStore};

    fn sides() -> SemanticSides {
        SemanticSides {
            base: SnapshotId(Hash([1; 32])),
            base_commit: CommitId("base".into()),
            target: SnapshotId(Hash([2; 32])),
            head: CommitId("head".into()),
            live_target: true,
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
    }

    /// Counts a hover as given up on when its request is dropped.
    struct HoverAsked(Arc<Counters>);

    impl Drop for HoverAsked {
        fn drop(&mut self) {
            self.0.hovers_dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeLauncher(Arc<Counters>);

    struct FakeServer(Arc<Counters>, Tree);

    impl ServerLauncher for FakeLauncher {
        type Server = FakeServer;

        /// Slow, so starts that overlap are seen to.
        async fn start(
            &self,
            root: &Path,
            tree: Tree,
            backend: &Backend,
            policy: AnalysisPolicy,
        ) -> Result<Started<FakeServer>, LaunchError> {
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
            Ok(Started {
                server: FakeServer(c.clone(), tree),
                identity: self.identify(root, backend).await?,
                restore: match tree {
                    Tree::Immutable => self.recorded_restore(root, backend, policy).await,
                    Tree::Live => None,
                },
            })
        }

        async fn identify(&self, _: &Path, backend: &Backend) -> Result<ServerIdentity, LaunchError> {
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
        }
    }

    impl LanguageServer for FakeServer {
        fn readiness(&self) -> Readiness {
            Readiness { idle: true, epoch: 0 }
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
            let started = self.0.started.load(Ordering::SeqCst);
            let _ = (self.0.started_at_first_answer).compare_exchange(0, started, Ordering::SeqCst, Ordering::SeqCst);
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
        async fn workspace_symbols(&self, _: &str) -> Result<Vec<SymbolInfo>, QueryError> {
            Ok(vec![])
        }
        async fn diagnostics(&self, _: &RepoPath) -> Result<Vec<Diagnostic>, QueryError> {
            Ok(vec![])
        }
        async fn shutdown(&self) {}
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
            root_markers: vec![],
            executes_project_code_during_analysis: false,
            loads_project_on_open: false,
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
            Trust::Local,
            Links {
                revisions: fx.session.revisions(),
                limits: RestoreLimits::default(),
                tasks: Tasks::default(),
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
        semantics.start_servers(&fx.session, 0).await.unwrap();
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
        semantics.start_servers(&fx.session, revision).await.unwrap();

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
    async fn the_analysis_reports_the_files_done_of_the_files_to_analyse_and_ends() {
        let fx = Fixture::open(
            &[("a.rs", "one\n"), ("b.py", "one\n"), ("c.md", "one\n")],
            &[("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")],
        )
        .await;
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let counters = Arc::<Counters>::default();
        let semantics = Semantics::new(
            FakeMaterializer(counters.clone()),
            FakeLauncher(counters),
            "/live".into(),
            vec![backend("rust", "rs"), backend("python", "py")],
            Trust::Local,
            Links {
                revisions: fx.session.revisions(),
                limits: RestoreLimits::default(),
                tasks: Tasks::new(
                    std::time::Duration::ZERO,
                    Box::new(move |t| sink.lock().unwrap().push(t)),
                ),
            },
        );
        semantics.changed_symbols(&fx.session, 0).await.unwrap();

        let heard = heard.lock().unwrap();
        let analysis: Vec<_> = heard.iter().filter(|t| t.kind == TaskKind::Analyse).collect();
        assert!(analysis.iter().all(|t| t.id == analysis[0].id));
        assert_eq!(analysis.first().map(|t| &t.state), Some(&TaskState::Running));
        let last = analysis.last().unwrap();
        assert_eq!(
            (&last.state, last.done, last.total),
            (&TaskState::Finished, Some(3), Some(3))
        );
        let progress: Vec<_> = analysis.iter().filter_map(|t| t.done).collect();
        assert!(progress.windows(2).all(|w| w[0] <= w[1]), "{progress:?}");
    }

    #[tokio::test]
    async fn an_answer_names_the_server_that_gave_it_and_a_derived_result_shows_it() {
        let (fx, semantics, _) = semantics().await;
        let at = At {
            file: RepoPath(b"a.rs".to_vec()),
            pos: Pos { line: 0, col: 0 },
        };
        let hover = semantics.hover(&fx.session, 0, Side::Target, &at).await.unwrap();
        let by = hover.provenance.unwrap();
        assert_eq!(by.server.label, "fake rust");
        assert!(!by.from_cache);

        let changes = semantics.changed_symbols(&fx.session, 0).await.unwrap();
        assert_eq!(
            changes.inputs.primitives,
            [
                "document_symbols(base:a.rs) [fake rust]",
                "document_symbols(target:a.rs) [fake rust]",
                "document_symbols(base:b.py) [fake python]",
                "document_symbols(target:b.py) [fake python]",
                // No server for Markdown.
                "document_symbols(base:c.md)",
                "document_symbols(target:c.md)",
            ]
        );
    }

    /// What happens to the session that moves its revision on.
    enum Bump {
        Settle,
        Filter,
        FinishReview,
        SetBase,
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
                Self::SetBase => {
                    fx.git.add_commit("c1", "c1-full", &[("a.rs", "zero\n")]);
                    fx.session
                        .set_base(BaseChoice::Commit {
                            id: CommitId("c1".into()),
                        })
                        .await
                        .unwrap();
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
        for bump in [
            Bump::Settle,
            Bump::Filter,
            Bump::FinishReview,
            Bump::SetBase,
            Bump::OpenChangeset,
        ] {
            let (fx, semantics, counters) = semantics().await;
            fx.begin().await;
            counters.hang_hover.store(true, Ordering::SeqCst);
            let at = At {
                file: RepoPath(b"a.rs".to_vec()),
                pos: Pos { line: 0, col: 0 },
            };
            let hover = semantics.hover(&fx.session, 0, Side::Target, &at);
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
        let stale = semantics.changed_symbols(&fx.session, 0).await;
        assert!(matches!(stale, Err(SessionError::Stale { current: 3 })));
        assert_eq!(counters.started.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_callers_of_the_changed_symbols_ask_each_server_once() {
        let (fx, semantics, counters) = semantics().await;
        let (first, second) = join(
            semantics.changed_symbols(&fx.session, 0),
            semantics.changed_symbols(&fx.session, 0),
        )
        .await;
        let (first, second) = (first.unwrap(), second.unwrap());
        assert_eq!(first.value, second.value);
        assert_eq!(first.value.symbols.len(), 2);
        assert!(first.value.gaps.is_empty());
        // Both sides are read from their texts, one request per text and not per caller;
        // the live files are not asked, and no server is started for the base.
        assert_eq!(counters.text_symbol_requests.load(Ordering::SeqCst), 4);
        assert_eq!(counters.live_symbol_requests.load(Ordering::SeqCst), 0);
        assert_eq!(counters.started.load(Ordering::SeqCst), 2);

        let third = semantics.changed_symbols(&fx.session, 0).await.unwrap();
        assert_eq!(third.value, first.value);
        assert_eq!(counters.text_symbol_requests.load(Ordering::SeqCst), 4);
    }

    // Every analysis of a symbol (signature, impact, reachability) needs the revision's
    // changed symbols; they are found once, under one "Finding the changed symbols" task.
    #[tokio::test]
    async fn the_analyses_of_one_revision_share_one_search_for_its_changed_symbols() {
        let fx = Fixture::open(
            &[("a.rs", "one\n"), ("b.py", "one\n")],
            &[("a.rs", "two\n"), ("b.py", "two\n")],
        )
        .await;
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let counters = Arc::<Counters>::default();
        let semantics = Semantics::new(
            FakeMaterializer(counters.clone()),
            FakeLauncher(counters.clone()),
            "/live".into(),
            vec![backend("rust", "rs"), backend("python", "py")],
            Trust::Local,
            Links {
                revisions: fx.session.revisions(),
                limits: RestoreLimits::default(),
                tasks: Tasks::new(
                    std::time::Duration::ZERO,
                    Box::new(move |t| sink.lock().unwrap().push(t)),
                ),
            },
        );
        // Four selections of a symbol, three analyses each.
        let all = bounded(12, 0..12, |_| semantics.changed_symbols(&fx.session, 0)).await;
        assert!(all.iter().all(Result::is_ok));

        let heard = heard.lock().unwrap();
        let searches: std::collections::BTreeSet<_> = heard
            .iter()
            .filter(|t| t.kind == TaskKind::Analyse)
            .map(|t| t.id)
            .collect();
        assert_eq!(searches.len(), 1, "one search, not one per caller");
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
            semantics.changed_symbols(&fx.session, 0).await.unwrap();
            counters.identified.load(Ordering::SeqCst)
        }
        assert_eq!(identities(2).await, identities(16).await);
    }

    #[tokio::test]
    async fn a_revision_asks_for_the_symbols_of_the_texts_that_are_new_and_of_no_other() {
        let (fx, semantics, counters) = semantics().await;
        semantics.changed_symbols(&fx.session, 0).await.unwrap();
        let asked = || counters.text_symbol_requests.load(Ordering::SeqCst);
        assert_eq!(asked(), 4);

        // One file is edited: its new text is asked, and nothing else.
        let revision = fx
            .settle(&[("a.rs", "three\n"), ("b.py", "two\n"), ("c.md", "two\n")])
            .await;
        semantics.changed_symbols(&fx.session, revision).await.unwrap();
        assert_eq!(asked(), 5);

        // Nothing about the content changes.
        let revision = fx.session.exclude_path("c.md", false).await.unwrap();
        semantics.changed_symbols(&fx.session, revision).await.unwrap();
        assert_eq!(asked(), 5);

        // The edit is undone: the text was seen before.
        let revision = fx
            .settle(&[("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")])
            .await;
        semantics.changed_symbols(&fx.session, revision).await.unwrap();
        assert_eq!(asked(), 5);
    }

    #[tokio::test]
    async fn the_changed_symbols_are_the_same_with_answers_kept_and_without() {
        async fn run(limit: usize) -> (Vec<Derived<Changes>>, usize) {
            let (fx, semantics, counters) = semantics_keeping(
                limit,
                &[("a.rs", "one\n"), ("b.py", "one\n"), ("c.md", "one\n")],
                &[("a.rs", "two\n"), ("b.py", "two\n"), ("c.md", "two\n")],
            )
            .await;
            let mut results = vec![semantics.changed_symbols(&fx.session, 0).await.unwrap()];
            for edit in ["three\n", "two\n"] {
                let revision = fx.settle(&[("a.rs", edit), ("b.py", "two\n"), ("c.md", "two\n")]).await;
                results.push(semantics.changed_symbols(&fx.session, revision).await.unwrap());
            }
            let revision = fx.session.exclude_path("c.md", false).await.unwrap();
            results.push(semantics.changed_symbols(&fx.session, revision).await.unwrap());
            (results, counters.text_symbol_requests.load(Ordering::SeqCst))
        }
        let (kept, asked_kept) = run(vet_semantic::reuse::KEPT).await;
        let (not_kept, asked_not_kept) = run(0).await;
        let symbols = |all: &[Derived<Changes>]| all.iter().map(|d| d.value.clone()).collect::<Vec<_>>();
        assert_eq!(symbols(&kept), symbols(&not_kept));
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
        let expected = plain.changed_symbols(&fx.session, 0).await.unwrap();

        let store = Arc::new(MemoryCache::default());
        let mut launched = Vec::new();
        for _ in 0..2 {
            let counters = Arc::new(Counters::default());
            let semantics = Semantics::new(
                FakeMaterializer(counters.clone()),
                FakeLauncher(counters.clone()),
                "/live".into(),
                vec![backend("rust", "rs"), backend("python", "py"), backend("go", "go")],
                Trust::Local,
                Links {
                    revisions: fx.session.revisions(),
                    limits: RestoreLimits::default(),
                    tasks: Tasks::default(),
                },
            )
            .persisting(store.clone());
            let changes = semantics.changed_symbols(&fx.session, 0).await.unwrap();
            assert_eq!(changes.value, expected.value);
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
        let hover = semantics.hover(&fx.session, 0, Side::Base, &at).await.unwrap();
        answers.push((format!("{:?}", hover.result), hover.provenance.unwrap().from_cache));
        let definition = semantics.definition(&fx.session, 0, Side::Base, &at).await.unwrap();
        answers.push((
            format!("{:?}", definition.result),
            definition.provenance.unwrap().from_cache,
        ));
        let references = semantics.references(&fx.session, 0, Side::Base, &at).await.unwrap();
        answers.push((
            format!("{:?}", references.result),
            references.provenance.unwrap().from_cache,
        ));
        let pair = semantics.pair(&fx.session, 0).await.unwrap();
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
                    Trust::Local,
                    Links {
                        revisions: fx.session.revisions(),
                        limits: RestoreLimits::default(),
                        tasks: Tasks::default(),
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
