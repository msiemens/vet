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
    /// Opening a file can start loading its project, and answers before that
    /// finishes resolve nothing outside the file. Servers that load their
    /// projects before they are ready (all others) answer at once.
    pub loads_project_on_open: bool,
}

/// Quirk arms are added reluctantly, per observed incompatibility. No registry,
/// no plugin surface.
#[derive(Clone, Debug)]
pub enum Backend {
    GenericLsp(ServerConfig),
    /// `roslyn-language-server`: told which solution or projects to load with
    /// its own `solution/open` and `project/open` notifications, and ready when
    /// it sends `workspace/projectInitializationComplete`.
    Roslyn(ServerConfig),
}

impl Backend {
    pub fn config(&self) -> &ServerConfig {
        match self {
            Backend::GenericLsp(config) | Backend::Roslyn(config) => config,
        }
    }
}

/// The backend serving `file`, by extension. `None`: the language is unsupported.
pub fn backend_for<'a>(backends: &'a [Backend], file: &RepoPath) -> Option<&'a Backend> {
    let ext = file.extension()?;
    backends
        .iter()
        .find(|b| b.config().filetypes.iter().any(|t| t.as_bytes() == ext))
}

#[derive(Debug)]
pub enum LaunchError {
    /// A required operation is missing: the language is unsupported (I6).
    MissingCapability(&'static str),
    /// No server for the language was found; the reason says where Vet looked.
    NotInstalled(String),
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
            Self::NotInstalled(why) => f.write_str(why),
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

/// Puts the locations inside the root, the only ones that can be opened, before the others; each
/// group keeps the server's order. The first is then the one to go to.
pub fn prefer_openable(locations: &mut [Location]) {
    locations.sort_by_key(|l| l.external);
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
        f.write_str(&self.file.to_string_lossy())?;
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
    /// What the server says about it in one line (`documentSymbol.detail`), often its signature.
    pub detail: Option<String>,
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
///
/// Methods take `&self`: the pool lets several callers use one server at once.
pub trait LanguageServer: Send + Sync {
    fn document_symbols(&self, file: &RepoPath) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    /// The symbols `text` declares if it were the content of `file`, which need
    /// not exist in the tree. They depend on the text alone, so any server of
    /// the file's language can answer. Nothing of the text stays in the server.
    fn text_symbols(
        &self,
        file: &RepoPath,
        text: &str,
    ) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn definition(&self, at: &At) -> impl Future<Output = Result<Vec<Location>, QueryError>> + Send;
    /// Without the declaration itself.
    fn references(&self, at: &At) -> impl Future<Output = Result<Vec<Location>, QueryError>> + Send;
    fn hover(&self, at: &At) -> impl Future<Output = Result<Option<String>, QueryError>> + Send;
    /// Callers of the symbol at `at` that are inside the root.
    fn incoming_calls(&self, at: &At) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn outgoing_calls(&self, at: &At) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    /// Matches inside the root.
    fn workspace_symbols(&self, query: &str) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn diagnostics(&self, file: &RepoPath) -> impl Future<Output = Result<Vec<Diagnostic>, QueryError>> + Send;
    /// Ask the server to exit. It is killed if it does not.
    fn shutdown(&self) -> impl Future<Output = ()> + Send;
}

/// Whether the files a server sees can change under it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tree {
    /// The dirty worktree: the server must follow edits on disk.
    Live,
    /// A materialized snapshot: files never change.
    Immutable,
}

/// A restore step that failed: the analysis of that project is limited to what
/// does not need its dependencies.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RestoreFailure {
    /// The project directory, relative to the restored root with `/`; `.` is the root.
    pub project: String,
    /// The program that failed, e.g. `pnpm`.
    pub tool: String,
    /// Fit to show.
    pub message: String,
}

/// What a restore leaves for the server that starts on the directory.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Restored {
    /// Environment variables for the server process.
    pub env: Vec<(String, String)>,
    /// Steps that failed. The server still starts: what does not need the
    /// dependencies still works.
    pub failures: Vec<RestoreFailure>,
    /// `(project, tool)` of every step that ran, failed or not: a success is
    /// what lifts an earlier failure of the same step.
    pub ran: Vec<(String, String)>,
}

/// Installs the dependencies of a materialized snapshot without executing
/// project code (§7.2): fetch only. A step that would execute project code is
/// skipped unless the policy allows executing (§7.5). Implemented in `vet-infra`.
pub trait Restorer {
    fn restore(
        &self,
        root: &std::path::Path,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> impl Future<Output = Restored> + Send;
}

/// Implemented by `vet-infra::lsp`. The pool needs a launcher, not a server
/// instance, because the pool owns lifecycle: eviction, restart on settings
/// change, and the capability assertion (which the launcher performs on start).
/// On an `Immutable` tree the launcher restores the dependencies first.
pub trait ServerLauncher {
    type Server: LanguageServer;

    fn start(
        &self,
        root: &std::path::Path,
        tree: Tree,
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
            loads_project_on_open: false,
        })
    }

    #[test]
    fn backend_is_chosen_by_extension() {
        let backends = [rust()];
        let path = |p: &str| RepoPath::from(p);
        assert!(backend_for(&backends, &path("src/lib.rs")).is_some());
        assert!(backend_for(&backends, &path("README.md")).is_none());
        assert!(backend_for(&backends, &path("dir.rs/Makefile")).is_none());
        assert!(backend_for(&backends, &path("rs")).is_none());
    }

    #[test]
    fn definitions_inside_the_root_come_first_in_the_servers_order() {
        let at = |file: &str, external| Location {
            file: RepoPath::from(file),
            span: Span {
                start: Pos { line: 0, col: 0 },
                end: Pos { line: 0, col: 1 },
            },
            external,
        };
        let mut found = vec![
            at("/dep/a.rs", true),
            at("b.rs", false),
            at("/dep/c.rs", true),
            at("d.rs", false),
        ];
        prefer_openable(&mut found);
        let files: Vec<_> = found.iter().map(|l| l.file.to_string_lossy()).collect();
        assert_eq!(files, ["b.rs", "d.rs", "/dep/a.rs", "/dep/c.rs"]);
        // Nothing to prefer, nothing to do.
        prefer_openable(&mut []);
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
