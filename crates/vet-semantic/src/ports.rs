//! Traits the semantic layer consumes. Async methods return `impl Future` so
//! callers stay generic (no boxing on hot query paths) and futures are `Send`.

use std::fmt;
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

impl fmt::Display for NotMaterializable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBlobs(blobs) => write!(f, "{} blobs are no longer available", blobs.len()),
            Self::Other(why) => f.write_str(why),
        }
    }
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
    pub args: Vec<String>,
    /// File extensions without the dot.
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

/// The backend serving `file`, by extension. `None`: the language is unsupported.
pub fn backend_for<'a>(backends: &'a [Backend], file: &RepoPath) -> Option<&'a Backend> {
    let name = file.0.rsplit(|b| *b == b'/').next()?;
    let ext = name
        .rsplit(|b| *b == b'.')
        .next()
        .filter(|ext| ext.len() < name.len())?;
    backends
        .iter()
        .find(|b| b.config().filetypes.iter().any(|t| t.as_bytes() == ext))
}

#[derive(Debug)]
pub enum LaunchError {
    /// A required operation is missing: the language is unsupported (I6).
    MissingCapability(&'static str),
    Spawn(String),
    /// The server broke the protocol or failed to initialize.
    Protocol(String),
    /// The server did not finish indexing in time.
    NotReady(String),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCapability(c) => write!(f, "language server lacks required capability: {c}"),
            Self::Spawn(why) => write!(f, "cannot start language server: {why}"),
            Self::Protocol(why) => write!(f, "language server protocol error: {why}"),
            Self::NotReady(why) => write!(f, "language server not ready: {why}"),
        }
    }
}

/// 0-based line and UTF-16 column, as in LSP.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

/// End exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

impl Span {
    /// The lines the span touches; a span ending at column 0 does not touch its last line.
    pub fn lines(self) -> LineRange {
        let end = if self.end.col == 0 && self.end.line > self.start.line {
            self.end.line
        } else {
            self.end.line + 1
        };
        LineRange {
            start: self.start.line,
            end,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct At {
    pub file: RepoPath,
    pub pos: Pos,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Location {
    /// Inside the root, repository-relative. If `external`, an absolute path.
    pub file: RepoPath,
    pub span: Span,
    /// Outside the root: a dependency or toolchain source. Where something
    /// resolves to matters even when Vet has no snapshot of it.
    pub external: bool,
}

/// A symbol, identified by its file and container chain (outermost first).
/// Moves and renames change the identity; Vet does not infer them (§7.3).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Sym {
    pub file: RepoPath,
    pub name: Vec<String>,
}

impl fmt::Display for Sym {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", String::from_utf8_lossy(&self.file.0))?;
        for part in &self.name {
            write!(f, "::{part}")?;
        }
        Ok(())
    }
}

/// A symbol with where it is.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SymbolInfo {
    pub sym: Sym,
    /// The whole definition, including its body.
    pub span: Span,
    /// Where its name is: the position to ask references and calls about.
    pub at: Pos,
}

impl SymbolInfo {
    pub fn location(&self) -> At {
        At {
            file: self.sym.file.clone(),
            pos: self.at,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QueryError(pub String);

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The required operations (§7.1). Whether the server's answers come from the
/// live worktree or an immutable snapshot is the pool's concern, not this trait's.
pub trait LanguageServer: Send {
    fn document_symbols(&mut self, file: &RepoPath)
    -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn definition(&mut self, at: &At) -> impl Future<Output = Result<Vec<Location>, QueryError>> + Send;
    /// Without the declaration itself.
    fn references(&mut self, at: &At) -> impl Future<Output = Result<Vec<Location>, QueryError>> + Send;
    fn hover(&mut self, at: &At) -> impl Future<Output = Result<Option<String>, QueryError>> + Send;
    /// Callers of the symbol at `at` that are inside the root.
    fn incoming_calls(&mut self, at: &At) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn outgoing_calls(&mut self, at: &At) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    /// Matches inside the root.
    fn workspace_symbols(&mut self, query: &str) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn diagnostics(&mut self, file: &RepoPath) -> impl Future<Output = Result<Vec<Diagnostic>, QueryError>> + Send;
    /// Ask the server to exit. It is killed if it does not.
    fn shutdown(&mut self) -> impl Future<Output = ()> + Send;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rust() -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId("rust".into()),
            binary: "rust-analyzer".into(),
            args: vec![],
            filetypes: vec!["rs".into()],
            root_markers: vec![],
            executes_project_code_during_analysis: true,
        })
    }

    #[test]
    fn backend_is_chosen_by_extension() {
        let backends = [rust()];
        let path = |p: &str| RepoPath(p.as_bytes().to_vec());
        assert!(backend_for(&backends, &path("src/lib.rs")).is_some());
        assert!(backend_for(&backends, &path("README.md")).is_none());
        assert!(backend_for(&backends, &path("dir.rs/Makefile")).is_none());
        assert!(backend_for(&backends, &path("rs")).is_none());
    }

    #[test]
    fn span_lines_exclude_a_trailing_column_zero() {
        let span = |sl, sc, el, ec| Span {
            start: Pos { line: sl, col: sc },
            end: Pos { line: el, col: ec },
        };
        assert_eq!(span(2, 4, 2, 9).lines(), LineRange { start: 2, end: 3 });
        assert_eq!(span(2, 0, 5, 0).lines(), LineRange { start: 2, end: 5 });
        assert_eq!(span(2, 0, 5, 1).lines(), LineRange { start: 2, end: 6 });
    }
}
