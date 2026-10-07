//! Primitive queries (§7.3): the side being viewed is asked directly; base and
//! target are two sides of a [`Pair`], queried in parallel with [`crate::join`].
//! Derived analyses consume the [`Primitives`] trait, not the pool.

use std::fmt;

use vet_core::content::RepoPath;
use vet_core::review::Side;

use crate::policy::AnalysisPolicy;
use crate::pool::{Lease, Pool, PoolError, PoolKey, Target};
use crate::ports::{
    At, Backend, BackendId, Diagnostic, LanguageServer, Location, Materializer, QueryError, ServerLauncher, SymbolInfo,
    backend_for,
};

/// What a side of a comparison is served from, and what analysis it may run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// The primitive queries on either side of a comparison. Fakes implement this
/// in tests; [`Pair`] implements it over the pool.
pub trait Primitives {
    fn document_symbols(
        &self,
        side: Side,
        file: &RepoPath,
    ) -> impl Future<Output = Result<Vec<SymbolInfo>, SemanticError>> + Send;
    fn definition(&self, side: Side, at: &At) -> impl Future<Output = Result<Vec<Location>, SemanticError>> + Send;
    fn references(&self, side: Side, at: &At) -> impl Future<Output = Result<Vec<Location>, SemanticError>> + Send;
    fn hover(&self, side: Side, at: &At) -> impl Future<Output = Result<Option<String>, SemanticError>> + Send;
    fn incoming_calls(
        &self,
        side: Side,
        at: &At,
    ) -> impl Future<Output = Result<Vec<SymbolInfo>, SemanticError>> + Send;
    fn outgoing_calls(
        &self,
        side: Side,
        at: &At,
    ) -> impl Future<Output = Result<Vec<SymbolInfo>, SemanticError>> + Send;
    fn diagnostics(
        &self,
        side: Side,
        file: &RepoPath,
    ) -> impl Future<Output = Result<Vec<Diagnostic>, SemanticError>> + Send;
}

pub struct Semantic<M: Materializer, L: ServerLauncher> {
    pool: Pool<M, L>,
    backends: Vec<Backend>,
}

impl<M, L> Semantic<M, L>
where
    M: Materializer + Sync,
    L: ServerLauncher + Sync,
    L::Server: Send,
{
    pub fn new(pool: Pool<M, L>, backends: Vec<Backend>) -> Self {
        Self { pool, backends }
    }

    pub fn pool(&self) -> &Pool<M, L> {
        &self.pool
    }

    /// The language of the backend serving `file`; `None`: it is unsupported.
    pub fn language_of(&self, file: &RepoPath) -> Option<&BackendId> {
        backend_for(&self.backends, file).map(|b| &b.config().backend)
    }

    pub fn pair(&self, base: Served, target: Served) -> Pair<'_, M, L> {
        Pair {
            semantic: self,
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
                    r.map_err(SemanticError::Query)
                }
                Err(e) => Err(e),
            };
            results.push((id, result));
        }
        results
    }

    async fn lease(&self, served: &Served, backend: &Backend) -> Result<Lease<'_, M, L>, SemanticError> {
        let key = PoolKey {
            target: served.target,
            backend: backend.config().backend.clone(),
            policy: served.policy,
        };
        self.pool.lease(&key, backend).await.map_err(SemanticError::Pool)
    }

    async fn lease_for(&self, served: &Served, file: &RepoPath) -> Result<Lease<'_, M, L>, SemanticError> {
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
pub struct Pair<'a, M: Materializer, L: ServerLauncher> {
    semantic: &'a Semantic<M, L>,
    pub base: Served,
    pub target: Served,
}

impl<M: Materializer, L: ServerLauncher> Pair<'_, M, L> {
    fn served(&self, side: Side) -> &Served {
        match side {
            Side::Base => &self.base,
            Side::Target => &self.target,
        }
    }
}

macro_rules! ask {
    ($self:ident, $side:ident, $file:expr, $method:ident($($arg:expr),*)) => {{
        let lease = $self.semantic.lease_for($self.served($side), $file).await?;
        let answer = lease.$method($($arg),*).await;
        lease.release().await;
        answer.map_err(SemanticError::Query)
    }};
}

impl<M, L> Primitives for Pair<'_, M, L>
where
    M: Materializer + Sync,
    L: ServerLauncher + Sync,
    L::Server: Send,
{
    async fn document_symbols(&self, side: Side, file: &RepoPath) -> Result<Vec<SymbolInfo>, SemanticError> {
        ask!(self, side, file, document_symbols(file))
    }

    async fn definition(&self, side: Side, at: &At) -> Result<Vec<Location>, SemanticError> {
        ask!(self, side, &at.file, definition(at))
    }

    async fn references(&self, side: Side, at: &At) -> Result<Vec<Location>, SemanticError> {
        ask!(self, side, &at.file, references(at))
    }

    async fn hover(&self, side: Side, at: &At) -> Result<Option<String>, SemanticError> {
        ask!(self, side, &at.file, hover(at))
    }

    async fn incoming_calls(&self, side: Side, at: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
        ask!(self, side, &at.file, incoming_calls(at))
    }

    async fn outgoing_calls(&self, side: Side, at: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
        ask!(self, side, &at.file, outgoing_calls(at))
    }

    async fn diagnostics(&self, side: Side, file: &RepoPath) -> Result<Vec<Diagnostic>, SemanticError> {
        ask!(self, side, file, diagnostics(file))
    }
}
