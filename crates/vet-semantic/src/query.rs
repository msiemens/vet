//! Primitive queries (§7.3): the side being viewed is asked directly; base and
//! target are two sides of a [`Pair`], queried in parallel with [`crate::join`].
//! Derived analyses consume the [`Primitives`] trait, not the pool.

use std::fmt;

use vet_core::content::RepoPath;

use crate::policy::AnalysisPolicy;
use crate::pool::{Lease, Pool, PoolError, PoolKey, Target};
use crate::ports::{
    At, Backend, BackendId, Diagnostic, LanguageServer, Location, Materializer, QueryError, ServerLauncher,
    SymbolInfo, backend_for,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Which {
    Base,
    Target,
}

/// What a side of a comparison is served from, and what analysis it may run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Side {
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
            Self::NoBackend(file) => write!(f, "no language backend for {}", String::from_utf8_lossy(&file.0)),
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
        which: Which,
        file: &RepoPath,
    ) -> impl Future<Output = Result<Vec<SymbolInfo>, SemanticError>> + Send;
    fn definition(&self, which: Which, at: &At) -> impl Future<Output = Result<Vec<Location>, SemanticError>> + Send;
    fn references(&self, which: Which, at: &At) -> impl Future<Output = Result<Vec<Location>, SemanticError>> + Send;
    fn hover(&self, which: Which, at: &At) -> impl Future<Output = Result<Option<String>, SemanticError>> + Send;
    fn incoming_calls(
        &self,
        which: Which,
        at: &At,
    ) -> impl Future<Output = Result<Vec<SymbolInfo>, SemanticError>> + Send;
    fn outgoing_calls(
        &self,
        which: Which,
        at: &At,
    ) -> impl Future<Output = Result<Vec<SymbolInfo>, SemanticError>> + Send;
    fn diagnostics(
        &self,
        which: Which,
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

    pub fn backends(&self) -> &[Backend] {
        &self.backends
    }

    pub fn pair(&self, base: Side, target: Side) -> Pair<'_, M, L> {
        Pair {
            semantic: self,
            base,
            target,
        }
    }

    /// Search Everywhere: `workspace/symbol` on `side`, per backend, because
    /// each language has its own server. A backend that cannot serve is
    /// reported, not skipped.
    pub async fn workspace_symbols(
        &self,
        side: &Side,
        query: &str,
    ) -> Vec<(BackendId, Result<Vec<SymbolInfo>, SemanticError>)> {
        let mut results = Vec::new();
        for backend in &self.backends {
            let id = backend.config().backend.clone();
            let result = match self.lease(side, backend).await {
                Ok(mut lease) => {
                    let r = lease.workspace_symbols(query).await;
                    self.pool.release(lease).await;
                    r.map_err(SemanticError::Query)
                }
                Err(e) => Err(e),
            };
            results.push((id, result));
        }
        results
    }

    async fn lease(&self, side: &Side, backend: &Backend) -> Result<Lease<L::Server, M::Guard>, SemanticError> {
        let key = PoolKey {
            target: side.target,
            backend: backend.config().backend.clone(),
            policy: side.policy,
        };
        self.pool.lease(&key, backend).await.map_err(SemanticError::Pool)
    }

    async fn lease_for(&self, side: &Side, file: &RepoPath) -> Result<Lease<L::Server, M::Guard>, SemanticError> {
        let backend = backend_for(&self.backends, file).ok_or_else(|| SemanticError::NoBackend(file.clone()))?;
        self.lease(side, backend).await
    }
}

/// The two sides of a changeset.
pub struct Pair<'a, M: Materializer, L: ServerLauncher> {
    semantic: &'a Semantic<M, L>,
    pub base: Side,
    pub target: Side,
}

impl<M: Materializer, L: ServerLauncher> Pair<'_, M, L> {
    fn side(&self, which: Which) -> &Side {
        match which {
            Which::Base => &self.base,
            Which::Target => &self.target,
        }
    }
}

macro_rules! ask {
    ($self:ident, $which:ident, $file:expr, $method:ident($($arg:expr),*)) => {{
        let mut lease = $self.semantic.lease_for($self.side($which), $file).await?;
        let answer = lease.$method($($arg),*).await;
        $self.semantic.pool.release(lease).await;
        answer.map_err(SemanticError::Query)
    }};
}

impl<M, L> Primitives for Pair<'_, M, L>
where
    M: Materializer + Sync,
    L: ServerLauncher + Sync,
    L::Server: Send,
{
    async fn document_symbols(&self, which: Which, file: &RepoPath) -> Result<Vec<SymbolInfo>, SemanticError> {
        ask!(self, which, file, document_symbols(file))
    }

    async fn definition(&self, which: Which, at: &At) -> Result<Vec<Location>, SemanticError> {
        ask!(self, which, &at.file, definition(at))
    }

    async fn references(&self, which: Which, at: &At) -> Result<Vec<Location>, SemanticError> {
        ask!(self, which, &at.file, references(at))
    }

    async fn hover(&self, which: Which, at: &At) -> Result<Option<String>, SemanticError> {
        ask!(self, which, &at.file, hover(at))
    }

    async fn incoming_calls(&self, which: Which, at: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
        ask!(self, which, &at.file, incoming_calls(at))
    }

    async fn outgoing_calls(&self, which: Which, at: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
        ask!(self, which, &at.file, outgoing_calls(at))
    }

    async fn diagnostics(&self, which: Which, file: &RepoPath) -> Result<Vec<Diagnostic>, SemanticError> {
        ask!(self, which, file, diagnostics(file))
    }
}
