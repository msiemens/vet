//! Traits the semantic layer consumes. Async methods return `impl Future` so
//! callers stay generic (no boxing on hot query paths) and futures are `Send`.

use std::fmt;
use std::path::PathBuf;
use std::pin::Pin;

use vet_core::content::{BlobId, Hash, LineRange, RepoPath, SnapshotId};

use crate::policy::AnalysisPolicy;

/// A directory holding a complete snapshot. It stays as it is for as long as
/// `guard` lives; afterwards the materializer may keep it or remove it (§7.2).
pub struct Materialized<G> {
    pub root: PathBuf,
    pub guard: G,
}

#[derive(Clone, Debug)]
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

/// Implemented by Acquisition in `vet-app`. The pool's flights own it, so it is
/// `'static`.
pub trait Materializer: Send + Sync + 'static {
    type Guard: Send + Sync + 'static;

    /// The directory differs by policy: a restore may run code only where the policy allows.
    fn materialize(
        &self,
        snapshot: SnapshotId,
        policy: AnalysisPolicy,
    ) -> impl Future<Output = Result<Materialized<Self::Guard>, NotMaterializable>> + Send;

    /// The directory of a tree that is kept already, or `None` if `materialize` would
    /// have to build one. Nothing is built, locked or used: the tree may be gone by the
    /// time the caller reads it.
    fn kept(&self, snapshot: SnapshotId, policy: AnalysisPolicy) -> impl Future<Output = Option<PathBuf>> + Send;
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

/// An operation a server may not offer. A query for one it lacks is answered with
/// [`crate::query::SemanticError::NotOffered`], never with an empty answer (I6).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Feature {
    DocumentSymbols,
    Definition,
    References,
    Implementations,
    Hover,
    CallHierarchy,
    WorkspaceSymbols,
}

impl Feature {
    pub const ALL: [Feature; 7] = [
        Self::DocumentSymbols,
        Self::Definition,
        Self::References,
        Self::Implementations,
        Self::Hover,
        Self::CallHierarchy,
        Self::WorkspaceSymbols,
    ];

    /// What a person calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::DocumentSymbols => "document symbols",
            Self::Definition => "go to definition",
            Self::References => "find references",
            Self::Implementations => "implementations",
            Self::Hover => "hover",
            Self::CallHierarchy => "call hierarchy",
            Self::WorkspaceSymbols => "workspace symbol search",
        }
    }
}

/// The operations a started server offers (diagnostics are not among them: servers
/// publish them without announcing it, so they cannot be told from none).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Features {
    offered: [bool; Feature::ALL.len()],
}

impl Features {
    pub const NONE: Self = Self {
        offered: [false; Feature::ALL.len()],
    };

    pub const ALL: Self = Self {
        offered: [true; Feature::ALL.len()],
    };

    fn index(feature: Feature) -> usize {
        Feature::ALL.iter().position(|f| *f == feature).unwrap_or(0)
    }

    pub fn with(mut self, feature: Feature, offered: bool) -> Self {
        self.offered[Self::index(feature)] = offered;
        self
    }

    pub fn offers(self, feature: Feature) -> bool {
        self.offered[Self::index(feature)]
    }
}

#[derive(Clone, Debug)]
pub enum LaunchError {
    /// The server cannot work with Vet at all: it does not take documents (I6).
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

/// What a symbol is, as far as a list needs to tell them apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SymbolKind {
    /// A class, struct, enum, interface or the like; rust-analyzer lists an `impl` as one.
    Type,
    /// A function, method, constructor or operator.
    Function,
    /// A module, namespace or package.
    Module,
    /// A variable, field, constant or anything else.
    Other,
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
    pub kind: SymbolKind,
    /// How many of the leading names of `sym` are modules or namespaces (`HES.Domain` in
    /// `HES.Domain` › `Device` › `Create`), which a list leaves out. Display only: it is no part of
    /// the identity.
    pub modules: usize,
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

/// Whether a server is at work on its own, apart from the queries it is asked. An
/// answer given while it is, or after it began, may be a partial one (I2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Readiness {
    /// No work in progress: indexing and loading are done.
    pub idle: bool,
    /// Changes whenever the server begins work, so two equal values taken before and
    /// after a query tell that it did not begin any in between.
    pub epoch: u64,
}

/// The required operations (§7.1). Whether the server's answers come from the
/// live worktree or an immutable snapshot is the pool's concern, not this trait's.
///
/// Methods take `&self`: the pool lets several callers use one server at once.
pub trait LanguageServer: Send + Sync {
    fn readiness(&self) -> Readiness;
    /// The operations the server advertised. The pool does not ask for one it lacks.
    fn features(&self) -> Features;
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
    /// The implementations of the interface, trait or abstract member at `at`. A server that
    /// does not offer the operation fails the query.
    fn implementations(&self, at: &At) -> impl Future<Output = Result<Vec<Location>, QueryError>> + Send;
    fn hover(&self, at: &At) -> impl Future<Output = Result<Option<String>, QueryError>> + Send;
    /// Callers of the symbol at `at` that are inside the root.
    fn incoming_calls(&self, at: &At) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn outgoing_calls(&self, at: &At) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    /// Matches inside the root.
    fn workspace_symbols(&self, query: &str) -> impl Future<Output = Result<Vec<SymbolInfo>, QueryError>> + Send;
    fn diagnostics(&self, file: &RepoPath) -> impl Future<Output = Result<Vec<Diagnostic>, QueryError>> + Send;
    /// Completes with what is known of why when the server's process is gone or its channel
    /// closed, whoever ended it; never for a server that cannot tell. Owns what it waits on,
    /// so it does not keep the server alive.
    fn exited(&self) -> Exited {
        Box::pin(std::future::pending())
    }
    /// Ask the server to exit. It is killed if it does not.
    fn shutdown(&self) -> impl Future<Output = ()> + Send;
}

/// See [`LanguageServer::exited`].
pub type Exited = Pin<Box<dyn Future<Output = String> + Send>>;

/// Runs work on its own, since the semantic layer has no runtime of its own.
pub trait Detach: Send + Sync {
    fn detach(&self, work: Pin<Box<dyn Future<Output = ()> + Send>>);
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
    /// Hash of the restore record of the kept tree: tool versions, each step's
    /// argv and environment and its outcome. Equal fingerprints mean equal
    /// dependencies; `None` for a restore nothing was recorded of.
    pub fingerprint: Option<Hash>,
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

    /// The fingerprint that `restore` of the same arguments would return if it took
    /// the restore from the tree's record: nothing is restored, and nothing is written.
    /// `None` if it would restore, or the tree has no record.
    fn recorded(
        &self,
        root: &std::path::Path,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> impl Future<Output = Option<Hash>> + Send;
}

/// Who answered (I2): two servers with equal identities answer equal questions
/// equally. Everything the answers of a server depend on, apart from the tree it
/// runs on, is in `fingerprint`: its binary and version, its arguments and
/// initialization options, the compiler it is told about, and the user-level
/// configuration and environment it reads outside the repository.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ServerIdentity {
    pub backend: BackendId,
    pub fingerprint: Hash,
    /// Fit to show, e.g. "rust-analyzer 1.91.0 (2026-09-01)".
    pub label: String,
}

/// Where an answer came from (I6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Provenance {
    pub server: ServerIdentity,
    /// The server was not asked: an earlier answer to the same question was reused.
    pub from_cache: bool,
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.server.label)?;
        if self.from_cache {
            f.write_str(", cached")?;
        }
        Ok(())
    }
}

/// An answer and where it came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Answered<T> {
    pub value: T,
    pub provenance: Provenance,
}

impl<T> Answered<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Answered<U> {
        Answered {
            value: f(self.value),
            provenance: self.provenance,
        }
    }
}

/// A started server and the identity it started with.
pub struct Started<S> {
    pub server: S,
    pub identity: ServerIdentity,
    /// The fingerprint of the restore its immutable tree was started on (see
    /// [`Restored::fingerprint`]); `None` if none was recorded, or the tree is live.
    pub restore: Option<Hash>,
}

/// Implemented by `vet-infra::lsp`. The pool needs a launcher, not a server
/// instance, because the pool owns lifecycle: eviction, restart on settings
/// change, and the capability assertion (which the launcher performs on start).
/// On an `Immutable` tree the launcher restores the dependencies first.
pub trait ServerLauncher: Send + Sync + 'static {
    type Server: LanguageServer + 'static;

    fn start(
        &self,
        root: &std::path::Path,
        tree: Tree,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> impl Future<Output = Result<Started<Self::Server>, LaunchError>> + Send;

    /// The identity a server started on `root` now would have, without starting
    /// it. A binary replaced between this and a start shows in the identity the
    /// start returns, which is the one that counts.
    fn identify(
        &self,
        root: &std::path::Path,
        backend: &Backend,
    ) -> impl Future<Output = Result<ServerIdentity, LaunchError>> + Send;

    /// The fingerprint of the restore a server started now on the immutable tree
    /// `root` would be started on, without starting it or restoring anything (see
    /// [`Restorer::recorded`]). `None`: it would restore, or nothing is recorded.
    fn recorded_restore(
        &self,
        root: &std::path::Path,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> impl Future<Output = Option<Hash>> + Send;

    /// The servers of `backend` are replaced: what the launcher observed of its
    /// environment (toolchains, settings) is observed anew by the next `identify`.
    fn restarted(&self, _backend: &BackendId) {}
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
