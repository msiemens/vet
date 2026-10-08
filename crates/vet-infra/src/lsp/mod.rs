//! `vet_semantic::ports::ServerLauncher` and `LanguageServer`: one concrete
//! stdio LSP client (hand-rolled `Content-Length` framing over `tokio::process`,
//! with `lsp-types` for the protocol types), plus the `ServerCapabilities`
//! assertion on start.

mod capabilities;
pub mod config;
pub(crate) mod dotnet;
#[cfg(test)]
mod fake;
mod identity;
mod rpc;
mod server;
mod uri;

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lsp_types as lsp;
use lsp_types::request::Initialize;
use serde_json::{Value, json};
use vet_app::ports::{ServerCommand, ServerLocator};
use vet_app::progress::{TaskKind, Tasks};
use vet_core::content::Hash;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{
    Backend, BackendId, LaunchError, Restored, Restorer, ServerConfig, ServerIdentity, ServerLauncher, Started, Tree,
};

use crate::budget::{Budget, Budgets, SEMANTIC_IO};
use crate::git::list_files;
use crate::language::{self, Compiler};
use crate::paths;
use crate::restore::discover;
use capabilities::Diagnostics;
use rpc::Connection;
use uri::{directory_uri, file_uri};

pub use identity::{Identities, UserEnv};
pub use server::LspServer;

/// No server message for this long, none in progress: indexing is done.
const QUIET: Duration = Duration::from_millis(750);

pub struct LspLauncher<L, R> {
    /// How long to wait for indexing after `initialized`.
    pub ready_timeout: Duration,
    /// Where each language's server is, asked on every start.
    pub locator: L,
    /// Restores the dependencies of an immutable tree before its server starts.
    pub restorer: R,
    /// Where starting and indexing report.
    pub tasks: Tasks,
    /// Tells servers apart (I2), and remembers what that cost.
    pub identities: Identities,
    /// Where the files of a start are looked at.
    pub semantic_io: Budget,
}

impl<L, R> LspLauncher<L, R> {
    pub fn new(locator: L, restorer: R) -> Self {
        Self {
            ready_timeout: Duration::from_secs(180),
            locator,
            restorer,
            tasks: Tasks::default(),
            identities: Identities::new(UserEnv::current()),
            semantic_io: Budget::new(SEMANTIC_IO),
        }
    }

    /// Looks at files and runs probes on `budgets`.
    pub fn budgeted(mut self, budgets: &Budgets) -> Self {
        self.identities = self.identities.budgeted(budgets);
        self.semantic_io = budgets.semantic_io.clone();
        self
    }

    /// `path` with links resolved, on the lane's blocking threads. Servers and the
    /// tools they run get the plain form: many reject a verbatim path on Windows.
    async fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        let path = path.to_owned();
        self.semantic_io
            .blocking(move || paths::canonical(&path))
            .await
            .map_err(io::Error::other)?
    }

    /// Reports every start to `tasks`.
    pub fn reporting(mut self, tasks: Tasks) -> Self {
        self.tasks = tasks;
        self
    }
}

/// Restores nothing: for trees that already have their dependencies.
pub struct NoRestore;

impl Restorer for NoRestore {
    async fn restore(&self, _root: &Path, _backend: &Backend, _policy: AnalysisPolicy) -> Restored {
        Restored::default()
    }

    async fn recorded(&self, _root: &Path, _backend: &Backend, _policy: AnalysisPolicy) -> Option<Hash> {
        None
    }
}

/// Runs the backend's own `binary`, found on `PATH`, with no settings. For tests
/// and tools that have no settings to consult.
pub struct BinaryLocator;

impl ServerLocator for BinaryLocator {
    async fn locate(&self, config: &ServerConfig, _project: &Path) -> Result<ServerCommand, String> {
        Ok(ServerCommand {
            program: config.binary.clone(),
            compiler: None,
        })
    }
}

/// What a server needs before it starts.
struct Prepared {
    command: ServerCommand,
    restored: Restored,
    folders: Vec<lsp::WorkspaceFolder>,
    /// How long each step took, in order.
    phases: Phases,
}

/// How long each phase of a server start took, for the timing lines of the log.
#[derive(Default)]
struct Phases(Vec<(&'static str, Duration)>);

impl Phases {
    /// Runs `step`, recording how long it took as `name`.
    async fn run<T>(&mut self, name: &'static str, step: impl Future<Output = T>) -> T {
        let started = Instant::now();
        let out = step.await;
        self.mark(name, started);
        out
    }

    /// Records the time since `started` as `name`.
    fn mark(&mut self, name: &'static str, started: Instant) {
        self.0.push((name, started.elapsed()));
    }

    fn describe(&self) -> String {
        let phases: Vec<_> = self
            .0
            .iter()
            .map(|(name, took)| format!("{name} {} ms", took.as_millis()))
            .collect();
        phases.join(", ")
    }
}

impl<L: ServerLocator + Sync, R: Restorer + Sync> LspLauncher<L, R> {
    /// Restores the tree's dependencies if it is immutable, finds the server and
    /// the projects it is to serve. A restore that fails is not an error here:
    /// the failures come back with the result and reach the user through the
    /// restore limits.
    async fn prepare(
        &self,
        root: &Path,
        tree: Tree,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> Result<Prepared, LaunchError> {
        let config = backend.config();
        let mut phases = Phases::default();
        let restored = match tree {
            Tree::Immutable => {
                phases
                    .run("restore", self.restorer.restore(root, backend, policy))
                    .await
            }
            Tree::Live => Restored::default(),
        };
        let command = phases
            .run("locate binary", self.locator.locate(config, root))
            .await
            .map_err(LaunchError::NotInstalled)?;
        let folders = phases
            .run("workspace folders", workspace_folders(&self.semantic_io, root, config))
            .await
            .iter()
            .filter_map(|dir| {
                let name = dir
                    .file_name()
                    .map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned());
                Some(lsp::WorkspaceFolder {
                    uri: directory_uri(dir)?,
                    name,
                })
            })
            .collect();
        Ok(Prepared {
            command,
            restored,
            folders,
            phases,
        })
    }
}

/// A server that has answered `initialize` and, for those that need it, loaded its projects.
struct Launched {
    conn: Connection,
    root: PathBuf,
    identity: ServerIdentity,
    /// The fingerprint of the restore its tree was started on.
    restore: Option<Hash>,
    diagnostics: Diagnostics,
    phases: Phases,
}

/// A server that did not come up. The message can carry what the server wrote to stderr, which
/// can quote code, so only its first line is logged above `debug`.
fn warn_not_started(server: &str, tree: Tree, why: &LaunchError) {
    let message = why.to_string();
    tracing::warn!(
        server,
        ?tree,
        reason = vet_log::first_line(&message),
        "the language server did not start"
    );
    tracing::debug!(server, ?tree, reason = %message, "the language server's failure in full");
}

/// The server's name as a person knows it.
fn server_name(config: &ServerConfig) -> &str {
    language::by_id(&config.backend.0).map_or(config.binary.as_str(), |l| l.display_name)
}

impl<L: ServerLocator + Sync, R: Restorer + Sync> LspLauncher<L, R> {
    /// Starts the server's process and brings it through the handshake, which is
    /// the part of a start that does not index.
    async fn launch(
        &self,
        root: &Path,
        tree: Tree,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> Result<Launched, LaunchError> {
        let config = backend.config();
        let root = self
            .canonical(root)
            .await
            .map_err(|e| LaunchError::Spawn(format!("{}: {e}", root.display())))?;
        let Prepared {
            command,
            restored,
            folders,
            mut phases,
        } = self.prepare(&root, tree, backend, policy).await?;
        let identity = phases
            .run("identity", self.identities.of(&root, backend, &command))
            .await?;
        let folders_json = serde_json::to_value(&folders).map_err(|e| LaunchError::Protocol(e.to_string()))?;
        let phase = Instant::now();
        let conn = Connection::spawn(&command.program, &config.args, &root, &restored.env, folders_json)
            .map_err(LaunchError::Spawn)?;

        let options = initialization_options(config, &command);
        let diagnostics = handshake(&conn, &root, folders, options).await?;
        phases.mark("spawn and handshake", phase);

        if matches!(backend, Backend::Roslyn(_)) {
            let phase = Instant::now();
            load_dotnet_projects(&self.semantic_io, &conn, &root).await?;
            if !conn.wait_for(self.ready_timeout, |s| s.projects_loaded).await {
                return Err(LaunchError::NotReady(conn.describe(format!(
                    "{} did not load the projects within {}s",
                    config.binary,
                    self.ready_timeout.as_secs()
                ))));
            }
            phases.mark("project load", phase);
        }
        Ok(Launched {
            conn,
            root,
            identity,
            restore: restored.fingerprint,
            diagnostics,
            phases,
        })
    }
}

impl<L: ServerLocator + Send + Sync + 'static, R: Restorer + Send + Sync + 'static> ServerLauncher
    for LspLauncher<L, R>
{
    type Server = LspServer;

    /// Reports the start as one task and the indexing after it as another, which
    /// shows what the server says about its progress.
    async fn start(
        &self,
        root: &Path,
        tree: Tree,
        backend: &Backend,
        policy: AnalysisPolicy,
    ) -> Result<Started<LspServer>, LaunchError> {
        let started = Instant::now();
        let config = backend.config();
        let side = match tree {
            Tree::Live => "",
            Tree::Immutable => " for the base",
        };
        let name = server_name(config);
        let starting = self
            .tasks
            .start(TaskKind::ServerStart, format!("Starting {name}{side}"));
        let launched = self.launch(root, tree, backend, policy).await;
        if let Err(why) = &launched {
            warn_not_started(name, tree, why);
        }
        starting.conclude(&launched);
        let Launched {
            conn,
            root,
            identity,
            restore,
            diagnostics,
            mut phases,
        } = launched?;

        let phase = Instant::now();
        let indexing = self
            .tasks
            .start(TaskKind::ServerIndex, format!("Indexing {name}{side}"));
        conn.with_state(|state| {
            state.index = Some(indexing);
            state.touch();
        });
        let ready = conn.wait_for(self.ready_timeout, |s| s.is_idle(QUIET)).await;
        let indexing = conn.with_state(|state| state.index.take());
        if ready {
            drop(indexing);
        } else {
            let why = format!(
                "{} did not finish indexing within {}s",
                config.binary,
                self.ready_timeout.as_secs()
            );
            if let Some(indexing) = indexing {
                indexing.fail(why.clone());
            }
            let why = LaunchError::NotReady(conn.describe(why));
            warn_not_started(name, tree, &why);
            return Err(why);
        }
        phases.mark("ready wait", phase);
        tracing::info!(
            server = name,
            ?tree,
            took_ms = started.elapsed().as_millis() as u64,
            "language server ready"
        );
        vet_semantic::trace::log(
            || {
                format!(
                    "starting {} on {} ({tree:?}: {})",
                    config.binary,
                    root.display(),
                    phases.describe()
                )
            },
            started,
        );
        Ok(Started {
            server: LspServer::new(
                conn,
                root,
                tree,
                diagnostics,
                config.loads_project_on_open,
                self.semantic_io.clone(),
            ),
            identity,
            restore,
        })
    }

    async fn identify(&self, root: &Path, backend: &Backend) -> Result<ServerIdentity, LaunchError> {
        let root = self.canonical(root).await.unwrap_or_else(|_| root.to_owned());
        let command = (self.locator.locate(backend.config(), &root).await).map_err(LaunchError::NotInstalled)?;
        self.identities.of(&root, backend, &command).await
    }

    async fn recorded_restore(&self, root: &Path, backend: &Backend, policy: AnalysisPolicy) -> Option<Hash> {
        let root = self.canonical(root).await.ok()?;
        self.restorer.recorded(&root, backend, policy).await
    }

    fn restarted(&self, backend: &BackendId) {
        self.identities.renewed(backend);
    }
}

/// What a language's server is told with `initialize`, beyond the protocol:
/// where its compiler is, for those that bring none.
fn initialization_options(config: &ServerConfig, command: &ServerCommand) -> Option<Value> {
    let compiler = command.compiler.as_ref()?;
    let language = language::by_id(&config.backend.0)?;
    (language.compiler == Compiler::TypeScript).then(|| json!({"tsserver": {"path": compiler.to_string_lossy()}}))
}

/// `initialize`, the check of what the server can do, and `initialized`.
async fn handshake(
    conn: &Connection,
    root: &Path,
    folders: Vec<lsp::WorkspaceFolder>,
    initialization_options: Option<Value>,
) -> Result<Diagnostics, LaunchError> {
    #[allow(deprecated)]
    let params = lsp::InitializeParams {
        process_id: Some(std::process::id()),
        root_uri: directory_uri(root),
        initialization_options,
        capabilities: client_capabilities(),
        workspace_folders: Some(folders),
        client_info: Some(lsp::ClientInfo {
            name: "vet".into(),
            version: None,
        }),
        ..Default::default()
    };
    let init = conn
        .request::<Initialize>(params)
        .await
        .map_err(|e| LaunchError::Protocol(conn.describe(e.message)))?;
    let diagnostics = capabilities::assert_capabilities(&init.capabilities)?;
    conn.notify::<lsp::notification::Initialized>(lsp::InitializedParams {})
        .map_err(LaunchError::Protocol)?;
    Ok(diagnostics)
}

/// Tells roslyn what to load: it does not discover projects itself.
async fn load_dotnet_projects(io: &Budget, conn: &Connection, root: &Path) -> Result<(), LaunchError> {
    let uri = |path: &Path| {
        file_uri(path)
            .map(|u| u.as_str().to_owned())
            .ok_or_else(|| LaunchError::Protocol(format!("{} is not an absolute path", path.display())))
    };
    let load = dotnet::find(io, root).await.map_err(LaunchError::Protocol)?;
    let (method, params) = match load {
        Some(dotnet::Load::Solution(solution)) => ("solution/open", json!({"solution": uri(&solution)?})),
        Some(dotnet::Load::Projects(projects)) => {
            let uris = projects.iter().map(|p| uri(p)).collect::<Result<Vec<_>, _>>()?;
            ("project/open", json!({"projects": uris}))
        }
        None => {
            return Err(LaunchError::Protocol(
                "no solution or C# project found in the repository".into(),
            ));
        }
    };
    conn.notify_raw(method, params).map_err(LaunchError::Protocol)
}

/// The root if it holds a root marker (or the config has none); else the
/// outermost projects below it, found as restoring finds them, so a repository
/// with the project in a subdirectory still works.
async fn workspace_folders(io: &Budget, root: &Path, config: &ServerConfig) -> Vec<PathBuf> {
    let (at, markers) = (root.to_owned(), config.root_markers.clone());
    let has_marker = io
        .blocking(move || markers.iter().any(|m| at.join(m).exists()))
        .await
        .unwrap_or(false);
    if config.root_markers.is_empty() || has_marker {
        return vec![root.to_owned()];
    }
    let markers: Vec<&str> = config.root_markers.iter().map(String::as_str).collect();
    let files = list_files(io, root).await.unwrap_or_default();
    let mut found: Vec<PathBuf> = discover::outermost(discover::candidates(&files, &markers))
        .iter()
        .map(|dir| root.join(dir))
        .collect();
    if found.is_empty() {
        found.push(root.to_owned());
    }
    found.sort();
    found
}

fn client_capabilities() -> lsp::ClientCapabilities {
    // No `didChangeWatchedFiles`: servers then watch the filesystem themselves.
    lsp::ClientCapabilities {
        general: Some(lsp::GeneralClientCapabilities {
            position_encodings: Some(vec![lsp::PositionEncodingKind::UTF16]),
            ..Default::default()
        }),
        workspace: Some(lsp::WorkspaceClientCapabilities {
            workspace_folders: Some(true),
            configuration: Some(true),
            symbol: Some(Default::default()),
            ..Default::default()
        }),
        window: Some(lsp::WindowClientCapabilities {
            work_done_progress: Some(true),
            ..Default::default()
        }),
        text_document: Some(lsp::TextDocumentClientCapabilities {
            synchronization: Some(Default::default()),
            hover: Some(lsp::HoverClientCapabilities {
                content_format: Some(vec![lsp::MarkupKind::Markdown, lsp::MarkupKind::PlainText]),
                ..Default::default()
            }),
            definition: Some(Default::default()),
            references: Some(Default::default()),
            document_symbol: Some(lsp::DocumentSymbolClientCapabilities {
                hierarchical_document_symbol_support: Some(true),
                ..Default::default()
            }),
            call_hierarchy: Some(Default::default()),
            publish_diagnostics: Some(Default::default()),
            diagnostic: Some(lsp::DiagnosticClientCapabilities::default()),
            ..Default::default()
        }),
        experimental: Some(json!({"serverStatusNotification": true})),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(compiler: Option<&str>) -> ServerCommand {
        ServerCommand {
            program: "server".into(),
            compiler: compiler.map(PathBuf::from),
        }
    }

    #[test]
    fn typescript_is_told_where_its_compiler_is_and_other_languages_are_told_nothing() {
        let typescript = language::TYPESCRIPT.backend();
        let options = initialization_options(typescript.config(), &command(Some("/p/typescript/lib/tsserver.js")));
        assert_eq!(
            options,
            Some(json!({"tsserver": {"path": "/p/typescript/lib/tsserver.js"}}))
        );
        assert_eq!(initialization_options(typescript.config(), &command(None)), None);

        let rust = language::RUST.backend();
        assert_eq!(initialization_options(rust.config(), &command(Some("/x"))), None);
    }

    fn touch(root: &Path, name: &str) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[tokio::test]
    async fn the_root_with_a_marker_is_the_only_folder() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Cargo.toml");
        touch(dir.path(), "sub/Cargo.toml");
        let config = language::RUST.backend().config().clone();
        assert_eq!(
            workspace_folders(&Budget::new(1), dir.path(), &config).await,
            [dir.path().to_owned()]
        );
    }

    #[tokio::test]
    async fn without_a_marker_at_the_root_the_outermost_projects_below_it_are_the_folders() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "svc/b/Cargo.toml");
        touch(dir.path(), "svc/a/Cargo.toml");
        touch(dir.path(), "svc/a/inner/Cargo.toml");
        touch(dir.path(), "svc/a/node_modules/dep/Cargo.toml");
        touch(dir.path(), ".hidden/Cargo.toml");
        touch(dir.path(), "a/b/c/d/e/Cargo.toml");
        let config = language::RUST.backend().config().clone();
        let io = Budget::new(1);
        assert_eq!(
            workspace_folders(&io, dir.path(), &config).await,
            [dir.path().join("svc/a"), dir.path().join("svc/b")]
        );

        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            workspace_folders(&io, empty.path(), &config).await,
            [empty.path().to_owned()]
        );
    }
}
