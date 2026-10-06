//! Traits the semantic layer consumes. Async methods return `impl Future` so
//! callers stay generic (no boxing on hot query paths) and futures are `Send`.

use std::path::PathBuf;

use vet_core::content::{BlobId, LineRange, RepoPath, SnapshotId};

use crate::policy::AnalysisPolicy;

/// A directory holding a complete snapshot. Dropping `guard` deletes it (§7.2).
pub struct Materialized<G> {
    pub root: PathBuf,
    pub guard: G,
}

#[derive(Debug)]
pub enum NotMaterializable {
    /// Blobs git no longer has and the content store never retained (§6.1).
    MissingBlobs(Vec<BlobId>),
    Other(String),
}

/// Implemented by Acquisition in `vet-app`.
pub trait Materializer {
    type Guard: Send + Sync + 'static;

    fn materialize(
        &self,
        snapshot: SnapshotId,
    ) -> impl Future<Output = Result<Materialized<Self::Guard>, NotMaterializable>> + Send;
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BackendId(pub String);

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub backend: BackendId,
    pub binary: String,
    pub filetypes: Vec<String>,
    pub root_markers: Vec<String>,
    /// Default `true` (conservative).
    pub executes_project_code_during_analysis: bool,
}

/// Quirk arms are added reluctantly, per observed incompatibility. No registry,
/// no plugin surface.
#[derive(Clone, Debug)]
pub enum Backend {
    GenericLsp(ServerConfig),
}

impl Backend {
    pub fn config(&self) -> &ServerConfig {
        match self {
            Backend::GenericLsp(config) => config,
        }
    }
}

#[derive(Debug)]
pub enum LaunchError {
    /// A required operation is missing: the language is unsupported (I6).
    MissingCapability(&'static str),
    Spawn(String),
}

/// A symbol, identified by its container-qualified name.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Sym(pub String);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Location {
    pub file: RepoPath,
    pub range: LineRange,
}

#[derive(Debug)]
pub struct QueryError(pub String);

/// The required operations (§7.1). Whether the server's answers come from the
/// live worktree or an immutable snapshot is the pool's concern, not this trait's.
pub trait LanguageServer: Send {
    fn document_symbols(&mut self, file: &RepoPath) -> impl Future<Output = Result<Vec<(Sym, LineRange)>, QueryError>> + Send;
    fn definition(&mut self, at: &Location) -> impl Future<Output = Result<Vec<Location>, QueryError>> + Send;
    fn references(&mut self, sym: &Sym) -> impl Future<Output = Result<Vec<Location>, QueryError>> + Send;
    fn hover(&mut self, at: &Location) -> impl Future<Output = Result<Option<String>, QueryError>> + Send;
    fn incoming_calls(&mut self, sym: &Sym) -> impl Future<Output = Result<Vec<Sym>, QueryError>> + Send;
    fn outgoing_calls(&mut self, sym: &Sym) -> impl Future<Output = Result<Vec<Sym>, QueryError>> + Send;
    fn workspace_symbols(&mut self, query: &str) -> impl Future<Output = Result<Vec<(Sym, Location)>, QueryError>> + Send;
    fn diagnostics(&mut self, file: &RepoPath) -> impl Future<Output = Result<Vec<String>, QueryError>> + Send;
}

/// Implemented by `vet-infra::lsp`. The pool needs a launcher, not a server
/// instance, because the pool owns lifecycle: eviction, restart on settings
/// change, and the capability assertion (which the launcher performs on start).
pub trait ServerLauncher {
    type Server: LanguageServer;

    fn start(
        &self,
        root: &std::path::Path,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> impl Future<Output = Result<Self::Server, LaunchError>> + Send;
}
