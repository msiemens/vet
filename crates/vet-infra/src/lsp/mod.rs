//! `vet_semantic::ports::ServerLauncher` and `LanguageServer`: one concrete
//! stdio LSP client (hand-rolled `Content-Length` framing over `tokio::process`,
//! with `lsp-types` for the protocol types), plus the `ServerCapabilities`
//! assertion on start.

mod capabilities;
pub mod config;
mod rpc;
mod server;

use std::path::{Path, PathBuf};
use std::time::Duration;

use lsp_types as lsp;
use lsp_types::request::Initialize;
use serde_json::json;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Backend, LaunchError, ServerConfig, ServerLauncher, Tree};

use rpc::Connection;

pub use server::LspServer;

/// No server message for this long, none in progress: indexing is done.
const QUIET: Duration = Duration::from_millis(750);

pub struct LspLauncher {
    /// How long to wait for indexing after `initialized`.
    pub ready_timeout: Duration,
}

impl Default for LspLauncher {
    fn default() -> Self {
        Self {
            ready_timeout: Duration::from_secs(180),
        }
    }
}

impl ServerLauncher for LspLauncher {
    type Server = LspServer;

    async fn start(
        &self,
        root: &Path,
        tree: Tree,
        backend: &Backend,
        _policy: AnalysisPolicy,
    ) -> Result<LspServer, LaunchError> {
        let config = backend.config();
        let root = std::fs::canonicalize(root).map_err(|e| LaunchError::Spawn(format!("{}: {e}", root.display())))?;
        let folders = workspace_folders(&root, config);
        let folder_values: Vec<_> = folders
            .iter()
            .filter_map(|f| {
                let uri = url::Url::from_directory_path(f).ok()?;
                let name = f
                    .file_name()
                    .map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned());
                Some(json!({"uri": uri.as_str(), "name": name}))
            })
            .collect();
        let conn =
            Connection::spawn(&config.binary, &config.args, &root, json!(folder_values)).map_err(LaunchError::Spawn)?;

        #[allow(deprecated)]
        let params = lsp::InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: url::Url::from_directory_path(&root)
                .ok()
                .and_then(|u| u.as_str().parse().ok()),
            capabilities: client_capabilities(),
            workspace_folders: Some(
                serde_json::from_value(json!(folder_values)).map_err(|e| LaunchError::Protocol(e.to_string()))?,
            ),
            client_info: Some(lsp::ClientInfo {
                name: "vet".into(),
                version: None,
            }),
            ..Default::default()
        };
        let init = conn
            .request::<Initialize>(params)
            .await
            .map_err(|e| LaunchError::Protocol(conn.describe(e)))?;
        let diagnostics = capabilities::assert_capabilities(&init.capabilities)?;
        conn.notify::<lsp::notification::Initialized>(lsp::InitializedParams {})
            .await
            .map_err(LaunchError::Protocol)?;

        conn.with_state(rpc::State::touch);
        if !conn.wait_for(self.ready_timeout, |s| s.is_idle(QUIET)).await {
            return Err(LaunchError::NotReady(conn.describe(format!(
                "{} did not finish indexing within {}s",
                config.binary,
                self.ready_timeout.as_secs()
            ))));
        }
        Ok(LspServer::new(conn, root, tree, diagnostics))
    }
}

/// The root if it holds a root marker (or the config has none); else the
/// directories below it that do, so a repository with the project in a
/// subdirectory still works.
fn workspace_folders(root: &Path, config: &ServerConfig) -> Vec<PathBuf> {
    let has_marker = |dir: &Path| config.root_markers.iter().any(|m| dir.join(m).exists());
    if config.root_markers.is_empty() || has_marker(root) {
        return vec![root.to_owned()];
    }
    let mut found = Vec::new();
    let mut level = vec![root.to_owned()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for dir in level {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let skip = name.starts_with('.') || matches!(name.as_ref(), "node_modules" | "target" | "vendor");
                if entry.file_type().is_ok_and(|t| t.is_dir()) && !skip {
                    if has_marker(&entry.path()) {
                        found.push(entry.path());
                    } else {
                        next.push(entry.path());
                    }
                }
            }
        }
        level = next;
    }
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
