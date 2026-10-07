//! Semantic queries of a session (§7.3): the side being viewed is asked
//! directly, derived analyses combine the answers with the revision's diff.
//! Positions and results belong to a revision; a stale one is refused like any
//! other request. Base answers come from the session's base snapshot, target
//! answers from the live tree, which is the revision's target until the next
//! snapshot settles.

use std::sync::{Arc, Mutex};

use vet_core::content::RepoPath;
use vet_semantic::derived::{
    Bounds, Change, Changes, Derived, EntryPoints, Reach, RemovalImpact, SymbolImpact, changed_symbols, impact,
    reachability, removal_impact,
};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::pool::{Pool, Target};
use vet_semantic::ports::{
    At, Backend, BackendId, Diagnostic, Location, RestoreFailure, ServerLauncher, Sym, SymbolInfo, prefer_openable,
};
use vet_semantic::query::{Primitives, Semantic, SemanticError, Side, Which};
use vet_semantic::signature::{Signature, signature};

use crate::acquisition::SharedAcquisition;
use crate::limits::RestoreLimits;
use crate::ports::{GitRepo, ReviewStore, Store};
use crate::session::{LiveSession, SessionError};

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

pub struct Semantics<G, S, L>
where
    G: GitRepo + Send + Sync,
    S: Store + Send + Sync,
    L: ServerLauncher,
{
    semantic: Semantic<SharedAcquisition<G, S>, L>,
    /// The changed symbols of one revision, which every derived analysis starts from.
    changes: Mutex<Option<(u32, Arc<Derived<Changes>>)>>,
    /// Kept current by the launcher's restorer (`Limiting`).
    limits: RestoreLimits,
}

impl<G, S, L> Semantics<G, S, L>
where
    G: GitRepo + Send + Sync,
    S: Store + ReviewStore + Send + Sync,
    L: ServerLauncher + Sync,
    L::Server: Send,
{
    /// `live_root` is the worktree the target side is served from. The dirty
    /// tree is the user's own, so servers may execute project code. `limits` is
    /// the one the launcher's restorer updates.
    pub fn new(
        session: &LiveSession<G, S>,
        launcher: L,
        live_root: std::path::PathBuf,
        backends: Vec<Backend>,
        limits: RestoreLimits,
    ) -> Self {
        let pool = Pool::new(session.materializer(), launcher, live_root, POOL_CAPACITY);
        Self {
            semantic: Semantic::new(pool, backends),
            changes: Mutex::new(None),
            limits,
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

    const POLICY: AnalysisPolicy = AnalysisPolicy { allow_executing: true };

    async fn pair(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
    ) -> Fallible<vet_semantic::query::Pair<'_, SharedAcquisition<G, S>, L>> {
        let sides = session.semantic_sides(revision).await?;
        Ok(self.semantic.pair(
            Side {
                target: Target::Snapshot(sides.base),
                policy: Self::POLICY,
            },
            Side {
                target: Target::Live,
                policy: Self::POLICY,
            },
        ))
    }

    pub async fn hover(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        which: Which,
        at: &At,
    ) -> Fallible<Answer<Option<String>>> {
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: pair.hover(which, at).await,
        })
    }

    pub async fn definition(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        which: Which,
        at: &At,
    ) -> Fallible<Answer<Vec<Location>>> {
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: pair.definition(which, at).await.map(|mut found| {
                prefer_openable(&mut found);
                found
            }),
        })
    }

    pub async fn references(
        &self,
        session: &LiveSession<G, S>,
        revision: u32,
        which: Which,
        at: &At,
    ) -> Fallible<Answer<Vec<Location>>> {
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: pair.references(which, at).await,
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
            result: pair.diagnostics(Which::Target, file).await,
        })
    }

    /// The symbols the revision's diff touches, on both sides of each changed file.
    pub async fn changed_symbols(&self, session: &LiveSession<G, S>, revision: u32) -> Fallible<Arc<Derived<Changes>>> {
        let pair = self.pair(session, revision).await?;
        if let Some((cached, changes)) = &*self.changes.lock().expect("not poisoned")
            && *cached == revision
        {
            return Ok(changes.clone());
        }
        let files = session.changed_files(revision).await?;
        let changes = Arc::new(changed_symbols(&pair, &files).await);
        *self.changes.lock().expect("not poisoned") = Some((revision, changes.clone()));
        Ok(changes)
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
        let which = if changed.change == Change::Removed {
            Which::Base
        } else {
            Which::Target
        };
        let pair = self.pair(session, revision).await?;
        Ok(Answer {
            revision,
            result: signature(&pair, which, &changed.symbol).await,
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
