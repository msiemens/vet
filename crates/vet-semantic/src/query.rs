//! Primitive queries (§7.3): the side being viewed is asked directly; base and
//! target are two sides of a [`Pair`], queried in parallel with [`crate::join`].
//! Derived analyses consume the [`Primitives`] trait, not the pool.

use std::fmt;
use std::sync::Arc;

use vet_core::content::RepoPath;
use vet_core::review::Side;

use crate::policy::AnalysisPolicy;
use crate::pool::{Lease, Pool, PoolError, PoolKey, Target};
use crate::ports::{
    Answered, At, Backend, BackendId, Diagnostic, Location, Materializer, QueryError, ServerLauncher, SymbolInfo,
    backend_for,
};
use crate::reuse::{ProjectQuery, Reused, SemanticCache};

/// What a side of a comparison is served from, and what analysis it may run.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Served {
    pub target: Target,
    pub policy: AnalysisPolicy,
}

#[derive(Debug)]
pub enum SemanticError {
    /// No configured backend handles the file: the language is unsupported.
    NoBackend(RepoPath),
    Pool(PoolError),
    Query(QueryError),
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBackend(file) => write!(f, "no language backend for {}", file.to_string_lossy()),
            Self::Pool(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
        }
    }
}

/// What a primitive query yields: the answer and the server it came from.
pub type Asked<T> = Result<Answered<T>, SemanticError>;

/// The primitive queries on either side of a comparison. Fakes implement this
/// in tests; [`Pair`] implements it over the pool.
pub trait Primitives {
    fn document_symbols(&self, side: Side, file: &RepoPath) -> impl Future<Output = Asked<Vec<SymbolInfo>>> + Send;
    /// The symbols `text` declares as the content of `file`, from the target
    /// side's server: they depend on the text alone, so no server of the side
    /// the text belongs to is needed.
    fn text_symbols(&self, file: &RepoPath, text: &str) -> impl Future<Output = Asked<Vec<SymbolInfo>>> + Send;
    fn definition(&self, side: Side, at: &At) -> impl Future<Output = Asked<Vec<Location>>> + Send;
    fn references(&self, side: Side, at: &At) -> impl Future<Output = Asked<Vec<Location>>> + Send;
    fn hover(&self, side: Side, at: &At) -> impl Future<Output = Asked<Option<String>>> + Send;
    fn incoming_calls(&self, side: Side, at: &At) -> impl Future<Output = Asked<Vec<SymbolInfo>>> + Send;
    fn outgoing_calls(&self, side: Side, at: &At) -> impl Future<Output = Asked<Vec<SymbolInfo>>> + Send;
    fn diagnostics(&self, side: Side, file: &RepoPath) -> impl Future<Output = Asked<Vec<Diagnostic>>> + Send;
}

pub struct Semantic<M: Materializer, L: ServerLauncher> {
    pool: Pool<M, L>,
    backends: Vec<Backend>,
}

impl<M: Materializer, L: ServerLauncher> Semantic<M, L> {
    pub fn new(pool: Pool<M, L>, backends: Vec<Backend>) -> Self {
        Self { pool, backends }
    }

    /// Keeps at most `limit` answers of the servers for reuse; 0 keeps none.
    pub fn keeping(mut self, limit: usize) -> Self {
        self.pool = self.pool.keeping(limit);
        self
    }

    /// Keeps the answers that persist in `store`, behind the ones in memory.
    pub fn persisting(mut self, store: Arc<dyn SemanticCache>) -> Self {
        self.pool = self.pool.persisting(store);
        self
    }

    pub fn pool(&self) -> &Pool<M, L> {
        &self.pool
    }

    /// The language of the backend serving `file`; `None`: it is unsupported.
    pub fn language_of(&self, file: &RepoPath) -> Option<&BackendId> {
        backend_for(&self.backends, file).map(|b| &b.config().backend)
    }

    /// The two sides, owning what they query with, so that work for them can outlive its caller.
    pub fn pair(self: &Arc<Self>, base: Served, target: Served) -> Pair<M, L> {
        Pair {
            semantic: self.clone(),
            base,
            target,
        }
    }

    /// Search Everywhere: `workspace/symbol` on `served`, per backend, because
    /// each language has its own server. A backend that cannot serve is
    /// reported, not skipped.
    pub async fn workspace_symbols(
        &self,
        served: &Served,
        query: &str,
    ) -> Vec<(BackendId, Result<Vec<SymbolInfo>, SemanticError>)> {
        let mut results = Vec::new();
        for backend in &self.backends {
            let id = backend.config().backend.clone();
            let result = match self.lease(served, backend).await {
                Ok(lease) => {
                    let r = lease.workspace_symbols(query).await;
                    lease.release().await;
                    r.map(|answered| answered.value).map_err(SemanticError::Query)
                }
                Err(e) => Err(e),
            };
            results.push((id, result));
        }
        results
    }

    async fn lease(&self, served: &Served, backend: &Backend) -> Result<Lease<M, L>, SemanticError> {
        self.pool
            .lease(&self.pool_key(served, backend), backend)
            .await
            .map_err(SemanticError::Pool)
    }

    fn pool_key(&self, served: &Served, backend: &Backend) -> PoolKey {
        PoolKey {
            target: served.target,
            backend: backend.config().backend.clone(),
            policy: served.policy,
        }
    }

    /// The answer to a question about the whole tree of `served` if one is kept, found
    /// without leasing a server.
    async fn recall<T: Reused>(&self, served: &Served, file: &RepoPath, query: ProjectQuery) -> Option<Answered<T>> {
        let backend = backend_for(&self.backends, file)?;
        self.pool.recall(&self.pool_key(served, backend), backend, query).await
    }

    async fn recall_text_symbols(
        &self,
        served: &Served,
        file: &RepoPath,
        text: &str,
    ) -> Option<Answered<Vec<SymbolInfo>>> {
        let backend = backend_for(&self.backends, file)?;
        self.pool
            .recall_text_symbols(&self.pool_key(served, backend), backend, file, text)
            .await
    }

    async fn lease_for(&self, served: &Served, file: &RepoPath) -> Result<Lease<M, L>, SemanticError> {
        let backend = backend_for(&self.backends, file).ok_or_else(|| SemanticError::NoBackend(file.clone()))?;
        self.lease(served, backend).await
    }

    /// Starts the server that serves `file` on `served`, or waits for its start
    /// to finish, so a later query finds it running. The server is not kept
    /// beyond what the pool keeps warm.
    pub async fn start_for(&self, served: &Served, file: &RepoPath) -> Result<(), SemanticError> {
        self.lease_for(served, file).await?.release().await;
        Ok(())
    }
}

/// The two sides of a changeset.
pub struct Pair<M: Materializer, L: ServerLauncher> {
    semantic: Arc<Semantic<M, L>>,
    pub base: Served,
    pub target: Served,
}

impl<M: Materializer, L: ServerLauncher> Pair<M, L> {
    fn served(&self, side: Side) -> &Served {
        match side {
            Side::Base => &self.base,
            Side::Target => &self.target,
        }
    }

    pub fn semantic(&self) -> &Semantic<M, L> {
        &self.semantic
    }
}

/// The answer kept for `$query` if there is one, else the server's.
macro_rules! ask {
    ($self:ident, $side:expr, $file:expr, $query:expr, $method:ident($($arg:expr),*)) => {{
        if let Some(kept) = $self.semantic.recall($self.served($side), $file, $query).await {
            return Ok(kept);
        }
        let lease = $self.semantic.lease_for($self.served($side), $file).await?;
        let answer = lease.$method($($arg),*).await;
        lease.release().await;
        answer.map_err(SemanticError::Query)
    }};
}

impl<M: Materializer, L: ServerLauncher> Primitives for Pair<M, L> {
    async fn document_symbols(&self, side: Side, file: &RepoPath) -> Asked<Vec<SymbolInfo>> {
        ask!(
            self,
            side,
            file,
            ProjectQuery::DocumentSymbols(file.clone()),
            document_symbols(file)
        )
    }

    async fn text_symbols(&self, file: &RepoPath, text: &str) -> Asked<Vec<SymbolInfo>> {
        let served = self.served(Side::Target);
        if let Some(kept) = self.semantic.recall_text_symbols(served, file, text).await {
            return Ok(kept);
        }
        let lease = self.semantic.lease_for(served, file).await?;
        let answer = lease.text_symbols(file, text).await;
        lease.release().await;
        answer.map_err(SemanticError::Query)
    }

    async fn definition(&self, side: Side, at: &At) -> Asked<Vec<Location>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Definition(at.clone()),
            definition(at)
        )
    }

    async fn references(&self, side: Side, at: &At) -> Asked<Vec<Location>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::References(at.clone()),
            references(at)
        )
    }

    async fn hover(&self, side: Side, at: &At) -> Asked<Option<String>> {
        ask!(self, side, &at.file, ProjectQuery::Hover(at.clone()), hover(at))
    }

    async fn incoming_calls(&self, side: Side, at: &At) -> Asked<Vec<SymbolInfo>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Incoming(at.clone()),
            incoming_calls(at)
        )
    }

    async fn outgoing_calls(&self, side: Side, at: &At) -> Asked<Vec<SymbolInfo>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Outgoing(at.clone()),
            outgoing_calls(at)
        )
    }

    async fn diagnostics(&self, side: Side, file: &RepoPath) -> Asked<Vec<Diagnostic>> {
        ask!(
            self,
            side,
            file,
            ProjectQuery::Diagnostics(file.clone()),
            diagnostics(file)
        )
    }
}
