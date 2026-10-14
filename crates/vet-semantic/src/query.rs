//! Primitive queries (§7.3): the side being viewed is asked directly; base and
//! target are two sides of a [`Pair`], queried in parallel with [`crate::join`].
//! Derived analyses consume the [`Primitives`] trait, not the pool.

use std::fmt;
use std::sync::Arc;

use vet_core::content::RepoPath;
use vet_core::review::Side;

use crate::join::bounded;
use crate::policy::AnalysisPolicy;
use crate::pool::{Lease, Pool, PoolError, PoolKey, Supervision, Target};
use crate::ports::{
    Answered, At, Backend, BackendId, Diagnostic, Feature, Location, Materializer, QueryError, ServerLauncher,
    SymbolInfo, backend_for,
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
    /// The language's server does not offer the operation, so it was not asked (I6).
    NotOffered {
        feature: Feature,
        /// The server as a person knows it.
        server: String,
    },
    /// The server was at work on its own while it gave the answer that was to be compared with:
    /// what it had not got to yet is unknown, not absent.
    Incomplete {
        /// The server as a person knows it.
        server: String,
    },
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOffered { feature, server } => write!(f, "{server} does not offer {}", feature.name()),
            Self::Incomplete { server } => write!(f, "{server} is still at work, so its answer is not final"),
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
    fn implementations(&self, side: Side, at: &At) -> impl Future<Output = Asked<Vec<Location>>> + Send;
    fn hover(&self, side: Side, at: &At) -> impl Future<Output = Asked<Option<String>>> + Send;
    fn incoming_calls(&self, side: Side, at: &At) -> impl Future<Output = Asked<Vec<SymbolInfo>>> + Send;
    fn outgoing_calls(&self, side: Side, at: &At) -> impl Future<Output = Asked<Vec<SymbolInfo>>> + Send;
    fn diagnostics(&self, side: Side, file: &RepoPath) -> impl Future<Output = Asked<Vec<Diagnostic>>> + Send;
}

/// Tells whether a file exists in the project, which decides what serves files of kinds that
/// several projects share (HTML).
pub type Finds = Arc<dyn Fn(&RepoPath) -> bool + Send + Sync>;

pub struct Semantic<M: Materializer, L: ServerLauncher> {
    pool: Pool<M, L>,
    backends: Vec<Backend>,
    finds: Finds,
}

impl<M: Materializer, L: ServerLauncher> Semantic<M, L> {
    pub fn new(pool: Pool<M, L>, backends: Vec<Backend>) -> Self {
        Self {
            pool,
            backends,
            finds: Arc::new(|_| true),
        }
    }

    /// Looks up the files that servers require (`ServerConfig::requires`) with `finds`; without it
    /// every one is taken to be there.
    pub fn finding(mut self, finds: Finds) -> Self {
        self.finds = finds;
        self
    }

    fn backend_for(&self, file: &RepoPath) -> Option<&Backend> {
        backend_for(&self.backends, file, &*self.finds)
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

    /// Starts a server that dies while running anew once, and tells `supervision` what became of it.
    pub fn supervised(mut self, supervision: Supervision) -> Self {
        self.pool = self.pool.supervised(supervision);
        self
    }

    pub fn pool(&self) -> &Pool<M, L> {
        &self.pool
    }

    /// The language of the backend serving `file`; `None`: it is unsupported.
    pub fn language_of(&self, file: &RepoPath) -> Option<&BackendId> {
        self.backend_for(file).map(|b| &b.config().backend)
    }

    /// The two sides, owning what they query with, so that work for them can outlive its caller.
    pub fn pair(self: &Arc<Self>, base: Served, target: Served) -> Pair<M, L> {
        Pair {
            semantic: self.clone(),
            base,
            target,
        }
    }

    /// Search Everywhere: `workspace/symbol` on `served`, per backend of `languages`, because
    /// each language has its own server. The servers are asked together, so a cold one does not
    /// hold the others back. A backend that cannot serve is reported, not skipped.
    pub async fn workspace_symbols(
        &self,
        served: &Served,
        languages: &[BackendId],
        query: &str,
    ) -> Vec<(BackendId, Result<Vec<SymbolInfo>, SemanticError>)> {
        let asked: Vec<&Backend> = self
            .backends
            .iter()
            .filter(|b| languages.contains(&b.config().backend))
            .collect();
        bounded(asked.len().max(1), asked, |backend| async move {
            let result = match self.lease(served, backend).await {
                Ok(lease) => {
                    let r = match offered(&lease, Feature::WorkspaceSymbols) {
                        Ok(()) => lease
                            .workspace_symbols(query)
                            .await
                            .map(|answered| answered.value)
                            .map_err(SemanticError::Query),
                        Err(e) => Err(e),
                    };
                    lease.release().await;
                    r
                }
                Err(e) => Err(e),
            };
            (backend.config().backend.clone(), result)
        })
        .await
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
        let backend = self.backend_for(file)?;
        self.pool.recall(&self.pool_key(served, backend), backend, query).await
    }

    async fn recall_text_symbols(
        &self,
        served: &Served,
        file: &RepoPath,
        text: &str,
    ) -> Option<Answered<Vec<SymbolInfo>>> {
        let backend = self.backend_for(file)?;
        self.pool
            .recall_text_symbols(&self.pool_key(served, backend), backend, file, text)
            .await
    }

    async fn lease_for(&self, served: &Served, file: &RepoPath) -> Result<Lease<M, L>, SemanticError> {
        let backend = self
            .backend_for(file)
            .ok_or_else(|| SemanticError::NoBackend(file.clone()))?;
        self.lease(served, backend).await
    }

    /// The servers that serve the files `wanted`, each on its `Served`, are not stopped to make
    /// room for others, in place of those kept before: they are what the changeset on view needs.
    pub fn keep_warm(&self, wanted: &[(&Served, &RepoPath)]) {
        self.pool.pin(wanted.iter().filter_map(|(served, file)| {
            let backend = self.backend_for(file)?;
            Some(self.pool_key(served, backend))
        }));
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

/// Whether the leased server offers `feature`; the error says which server does not.
fn offered<M: Materializer, L: ServerLauncher>(lease: &Lease<M, L>, feature: Feature) -> Result<(), SemanticError> {
    if lease.features().offers(feature) {
        return Ok(());
    }
    Err(SemanticError::NotOffered {
        feature,
        server: lease.identity().label.clone(),
    })
}

/// The answer kept for `$query` if there is one, else the server's, if it offers `$feature`
/// (`None`: the operation is one every server has).
macro_rules! ask {
    ($self:ident, $side:expr, $file:expr, $query:expr, $feature:expr, $method:ident($($arg:expr),*)) => {{
        if let Some(kept) = $self.semantic.recall($self.served($side), $file, $query).await {
            return Ok(kept);
        }
        let lease = $self.semantic.lease_for($self.served($side), $file).await?;
        let feature: Option<Feature> = $feature;
        let answer = match feature.map_or(Ok(()), |f| offered(&lease, f)) {
            Ok(()) => lease.$method($($arg),*).await.map_err(SemanticError::Query),
            Err(e) => Err(e),
        };
        lease.release().await;
        answer
    }};
}

impl<M: Materializer, L: ServerLauncher> Primitives for Pair<M, L> {
    async fn document_symbols(&self, side: Side, file: &RepoPath) -> Asked<Vec<SymbolInfo>> {
        ask!(
            self,
            side,
            file,
            ProjectQuery::DocumentSymbols(file.clone()),
            Some(Feature::DocumentSymbols),
            document_symbols(file)
        )
    }

    async fn text_symbols(&self, file: &RepoPath, text: &str) -> Asked<Vec<SymbolInfo>> {
        let served = self.served(Side::Target);
        if let Some(kept) = self.semantic.recall_text_symbols(served, file, text).await {
            return Ok(kept);
        }
        let lease = self.semantic.lease_for(served, file).await?;
        let answer = match offered(&lease, Feature::DocumentSymbols) {
            Ok(()) => lease.text_symbols(file, text).await.map_err(SemanticError::Query),
            Err(e) => Err(e),
        };
        lease.release().await;
        answer
    }

    async fn definition(&self, side: Side, at: &At) -> Asked<Vec<Location>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Definition(at.clone()),
            Some(Feature::Definition),
            definition(at)
        )
    }

    async fn references(&self, side: Side, at: &At) -> Asked<Vec<Location>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::References(at.clone()),
            Some(Feature::References),
            references(at)
        )
    }

    async fn implementations(&self, side: Side, at: &At) -> Asked<Vec<Location>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Implementations(at.clone()),
            Some(Feature::Implementations),
            implementations(at)
        )
    }

    async fn hover(&self, side: Side, at: &At) -> Asked<Option<String>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Hover(at.clone()),
            Some(Feature::Hover),
            hover(at)
        )
    }

    async fn incoming_calls(&self, side: Side, at: &At) -> Asked<Vec<SymbolInfo>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Incoming(at.clone()),
            Some(Feature::CallHierarchy),
            incoming_calls(at)
        )
    }

    async fn outgoing_calls(&self, side: Side, at: &At) -> Asked<Vec<SymbolInfo>> {
        ask!(
            self,
            side,
            &at.file,
            ProjectQuery::Outgoing(at.clone()),
            Some(Feature::CallHierarchy),
            outgoing_calls(at)
        )
    }

    async fn diagnostics(&self, side: Side, file: &RepoPath) -> Asked<Vec<Diagnostic>> {
        ask!(
            self,
            side,
            file,
            ProjectQuery::Diagnostics(file.clone()),
            None,
            diagnostics(file)
        )
    }
}
