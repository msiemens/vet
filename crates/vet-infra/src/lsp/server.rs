//! A started language server: the `LanguageServer` port over a [`Connection`].
//!
//! Open documents are mirrored from disk before every query (full-text sync),
//! so on the live worktree the server sees an agent's latest write. Nothing is
//! cached here; whether an answer may be reused is the pool's decision (I2).

use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use lsp_types as lsp;
use lsp_types::Uri;
use lsp_types::notification::{DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument};
use lsp_types::request::{
    CallHierarchyIncomingCalls, CallHierarchyOutgoingCalls, CallHierarchyPrepare, DocumentDiagnosticRequest,
    DocumentSymbolRequest, GotoDefinition, HoverRequest, References, WorkspaceSymbolRequest,
};
use vet_core::content::RepoPath;
use vet_semantic::ports::{
    At, Diagnostic, LanguageServer, Location, Pos, QueryError, Severity, Span, Sym, SymbolInfo, Tree,
};
use vet_semantic::symbols::{self, Outline};

use super::QUIET;
use super::capabilities::Diagnostics;
use super::rpc::{Connection, State};
use super::uri::{file_uri, key, path_of};
use crate::language::language_id;
use crate::{paths, timing};

/// Documents kept open at once; the oldest are closed beyond this.
const MAX_OPEN_DOCS: usize = 64;
const DIAGNOSTICS_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a newly opened document may keep the server busy before a query goes ahead.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

struct Doc {
    file: RepoPath,
    uri: Uri,
    version: i32,
    text: String,
    /// When the server was told about the document.
    opened: Instant,
}

/// The documents the server has open, and what is known about its progress
/// on them.
#[derive(Default)]
struct Docs {
    open: Vec<Doc>,
    /// When the last wait for the server to go quiet began, if it ended with
    /// the server quiet. Every document opened before that is settled: the
    /// server had processed it by then.
    quiet_from: Option<Instant>,
}

impl Docs {
    fn is_settled(&self, doc: &Doc) -> bool {
        self.quiet_from.is_some_and(|from| from >= doc.opened)
    }
}

/// What a query needs the server to have done with the document it names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// Its text only, as for the symbols a document declares.
    Text,
    /// Its project loaded, as for answers that resolve across files.
    Project,
}

/// What making the server see a file took and did, for `VET_TIMING`.
struct Synced {
    /// The file was opened or its text changed.
    changed: bool,
    /// Waiting for other callers' syncs to finish.
    lock: Duration,
    /// The wait for the server to go quiet, if this query had to make it.
    quiet: Option<Duration>,
}

impl Synced {
    /// `sync` is the whole time the query spent before it asked the server.
    fn describe(&self, sync: Duration) -> String {
        let mut parts = vec![format!("sync {} ms", sync.as_millis())];
        if self.lock >= Duration::from_millis(1) {
            parts.push(format!("lock {} ms", self.lock.as_millis()));
        }
        if self.changed {
            parts.push("opened or changed".into());
        }
        if let Some(quiet) = self.quiet {
            parts.push(format!("quiet wait {} ms", quiet.as_millis()));
        }
        parts.join(", ")
    }
}

/// A query that has made the server see its file: when it began and what that took.
struct Timed {
    started: Instant,
    sync: Duration,
    synced: Synced,
}

/// A position in a file, as the timing lines name it.
struct Place<'a>(&'a At);

impl Display for Place<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.0.file.to_string_lossy(),
            self.0.pos.line,
            self.0.pos.col
        )
    }
}

/// Shared by concurrent callers: requests go straight to the connection;
/// only document sync is serialized.
pub struct LspServer {
    conn: Connection,
    /// Canonical, so server URIs can be mapped back to repository paths.
    root: PathBuf,
    tree: Tree,
    diagnostics: Diagnostics,
    docs: tokio::sync::Mutex<Docs>,
    /// Numbers the in-memory documents of `text_symbols`, so concurrent calls
    /// for one file do not share a URI.
    virtual_docs: AtomicU64,
}

fn err(e: impl std::fmt::Display) -> QueryError {
    QueryError(e.to_string())
}

fn pos(p: lsp::Position) -> Pos {
    Pos {
        line: p.line,
        col: p.character,
    }
}

fn span(r: lsp::Range) -> Span {
    Span {
        start: pos(r.start),
        end: pos(r.end),
    }
}

fn position(p: Pos) -> lsp::Position {
    lsp::Position {
        line: p.line,
        character: p.col,
    }
}

impl LspServer {
    pub(super) fn new(conn: Connection, root: PathBuf, tree: Tree, diagnostics: Diagnostics) -> Self {
        Self {
            conn,
            root,
            tree,
            diagnostics,
            docs: tokio::sync::Mutex::new(Docs::default()),
            virtual_docs: AtomicU64::new(0),
        }
    }

    fn absolute(&self, file: &RepoPath) -> Result<PathBuf, QueryError> {
        Ok(self.root.join(paths::to_path(&file.0).map_err(err)?))
    }

    fn uri_of(&self, file: &RepoPath) -> Result<Uri, QueryError> {
        file_uri(&self.absolute(file)?).ok_or_else(|| err("path is not absolute"))
    }

    /// A URI for a document that exists only in the server's memory: a file that
    /// is not on disk next to `file`. Every server takes it as a loose file of
    /// the right language (verified for rust-analyzer, typescript-language-server,
    /// pyright and roslyn); `untitled:` is not used because rust-analyzer only maps
    /// `file:` URIs to its files.
    /// The name starts with a dot and the document is closed again right away,
    /// so it takes no part in a project for long.
    fn virtual_uri(&self, file: &RepoPath) -> Result<Uri, QueryError> {
        let n = self.virtual_docs.fetch_add(1, Ordering::Relaxed);
        let path = self.absolute(file)?;
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        file_uri(&path.with_file_name(format!(".vet-text-{n}-{name}"))).ok_or_else(|| err("path is not absolute"))
    }

    fn id(&self, file: &RepoPath) -> Result<lsp::TextDocumentIdentifier, QueryError> {
        Ok(lsp::TextDocumentIdentifier {
            uri: self.uri_of(file)?,
        })
    }

    fn position_params(&self, at: &At) -> Result<lsp::TextDocumentPositionParams, QueryError> {
        Ok(lsp::TextDocumentPositionParams {
            text_document: self.id(&at.file)?,
            position: position(at.pos),
        })
    }

    /// The repository path of an absolute path, or `None` outside the root.
    fn file_in_root(&self, path: &Path) -> Option<RepoPath> {
        paths::from_relative(paths::below(&self.root, path)?).ok()
    }

    /// The repository path of a server URI, or `None` outside the root.
    fn file_of(&self, uri: &Uri) -> Option<RepoPath> {
        self.file_in_root(&path_of(uri)?)
    }

    fn location(&self, uri: &Uri, range: lsp::Range) -> Location {
        let path = path_of(uri);
        match path.as_deref().and_then(|p| self.file_in_root(p)) {
            Some(file) => Location {
                file,
                span: span(range),
                external: false,
            },
            None => {
                let absolute = path
                    .and_then(|p| paths::from_absolute(&p).ok())
                    .unwrap_or_else(|| RepoPath(uri.as_str().as_bytes().to_vec()));
                Location {
                    file: absolute,
                    span: span(range),
                    external: true,
                }
            }
        }
    }

    /// Makes the server see `file` (see [`LspServer::sync`]) and starts the
    /// clock of the query's timing line.
    async fn begin(&self, file: &RepoPath, needs: Needs) -> Result<Timed, QueryError> {
        let started = Instant::now();
        let synced = self.sync(file, needs).await?;
        Ok(Timed {
            started,
            sync: started.elapsed(),
            synced,
        })
    }

    /// The timing line of a query: `what` names the thing asked about.
    fn log_query(&self, request: &str, what: impl Display, timed: &Timed, detail: &str) {
        timing::log(
            || {
                format!(
                    "{request} {:?} {what} ({}{detail})",
                    self.tree,
                    timed.synced.describe(timed.sync)
                )
            },
            timed.started,
        );
    }

    /// The timing line of a request that needs no sync of its own.
    fn log_request(&self, request: &str, what: impl Display, detail: &str, started: Instant) {
        timing::log(|| format!("{request} {:?} {what} ({detail})", self.tree), started);
    }

    /// Makes the server see `file`; on the live tree also mirrors every open
    /// document from disk. Snapshot files never change, so they are opened once
    /// and left alone.
    async fn sync(&self, file: &RepoPath, needs: Needs) -> Result<Synced, QueryError> {
        let asked = Instant::now();
        let mut docs = self.docs.lock().await;
        let lock = asked.elapsed();
        let changed = if self.tree == Tree::Immutable && docs.open.iter().any(|d| &d.file == file) {
            false
        } else if self.tree == Tree::Immutable {
            self.refresh(&mut docs.open, file).await?
        } else {
            let open: Vec<_> = docs.open.iter().map(|d| d.file.clone()).collect();
            for other in open.into_iter().filter(|f| f != file) {
                self.refresh(&mut docs.open, &other).await?;
            }
            self.refresh(&mut docs.open, file).await?
        };
        let quiet = if needs == Needs::Project {
            self.settle(&mut docs, file).await
        } else {
            None
        };
        Ok(Synced { changed, lock, quiet })
    }

    /// Opening a file can start loading its project (tsserver), and answers
    /// before that finishes resolve nothing outside the file. So a query that
    /// resolves across files waits for the server to go quiet once after the
    /// document was opened; one wait covers every document opened before it.
    /// A server that stays busy past the timeout is asked anyway. How long the
    /// wait took, if there was one.
    async fn settle(&self, docs: &mut Docs, file: &RepoPath) -> Option<Duration> {
        let doc = docs.open.iter().find(|d| &d.file == file)?;
        if docs.is_settled(doc) {
            return None;
        }
        let from = Instant::now();
        self.conn.with_state(State::touch);
        self.conn.wait_for(OPEN_TIMEOUT, |s| s.is_idle(QUIET)).await;
        docs.quiet_from = Some(from);
        Some(from.elapsed())
    }

    async fn refresh(&self, docs: &mut Vec<Doc>, file: &RepoPath) -> Result<bool, QueryError> {
        let path = self.absolute(file)?;
        let text = tokio::fs::read(&path)
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let index = docs.iter().position(|d| &d.file == file);
        let text = match (text, index) {
            (Ok(text), _) => text,
            (Err(_), Some(index)) => {
                let doc = docs.remove(index);
                self.conn
                    .notify::<DidCloseTextDocument>(lsp::DidCloseTextDocumentParams {
                        text_document: lsp::TextDocumentIdentifier { uri: doc.uri },
                    })
                    .await
                    .map_err(err)?;
                return Ok(false);
            }
            (Err(e), None) => return Err(err(format!("{}: {e}", path.display()))),
        };
        let Some(index) = index else {
            let uri = self.uri_of(file)?;
            self.conn
                .notify::<DidOpenTextDocument>(lsp::DidOpenTextDocumentParams {
                    text_document: lsp::TextDocumentItem {
                        uri: uri.clone(),
                        language_id: language_id(extension(file)).into(),
                        version: 1,
                        text: text.clone(),
                    },
                })
                .await
                .map_err(err)?;
            docs.push(Doc {
                file: file.clone(),
                uri,
                version: 1,
                text,
                opened: Instant::now(),
            });
            if docs.len() > MAX_OPEN_DOCS {
                let doc = docs.remove(0);
                self.conn
                    .notify::<DidCloseTextDocument>(lsp::DidCloseTextDocumentParams {
                        text_document: lsp::TextDocumentIdentifier { uri: doc.uri },
                    })
                    .await
                    .map_err(err)?;
            }
            return Ok(true);
        };
        if docs[index].text == text {
            return Ok(false);
        }
        let doc = &mut docs[index];
        doc.version += 1;
        doc.text = text.clone();
        let params = lsp::DidChangeTextDocumentParams {
            text_document: lsp::VersionedTextDocumentIdentifier {
                uri: doc.uri.clone(),
                version: doc.version,
            },
            content_changes: vec![lsp::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }],
        };
        self.conn.notify::<DidChangeTextDocument>(params).await.map_err(err)?;
        Ok(true)
    }

    async fn symbols_cached(&self, cache: &mut HashMap<RepoPath, Vec<SymbolInfo>>, file: &RepoPath) -> Vec<SymbolInfo> {
        if !cache.contains_key(file) {
            let symbols = self.document_symbols(file).await.unwrap_or_default();
            cache.insert(file.clone(), symbols);
        }
        cache[file].clone()
    }

    /// The symbol a server item refers to, with the container-qualified name
    /// from `documentSymbol`. `None` outside the root.
    async fn resolve(
        &self,
        cache: &mut HashMap<RepoPath, Vec<SymbolInfo>>,
        uri: &Uri,
        name: &str,
        range: lsp::Range,
        selection: lsp::Position,
    ) -> Option<SymbolInfo> {
        let file = self.file_of(uri)?;
        let symbols = self.symbols_cached(cache, &file).await;
        Some(symbols::resolve(&symbols, file, name, span(range), pos(selection)))
    }

    async fn prepare_calls(&self, at: &At) -> Result<Vec<lsp::CallHierarchyItem>, QueryError> {
        let timed = self.begin(&at.file, Needs::Project).await?;
        let params = lsp::CallHierarchyPrepareParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let items = self.conn.request::<CallHierarchyPrepare>(params).await;
        let found = items.as_ref().map_or(0, |i| i.as_ref().map_or(0, Vec::len));
        self.log_query("callHierarchy/prepare", Place(at), &timed, &format!(", {found} items"));
        Ok(items.map_err(err)?.unwrap_or_default())
    }

    /// The symbols the call hierarchy `items` stand for, each once, those
    /// outside the root left out.
    async fn symbols_of(&self, items: Vec<lsp::CallHierarchyItem>) -> Vec<SymbolInfo> {
        let mut cache = HashMap::new();
        let mut out: Vec<SymbolInfo> = Vec::new();
        for item in items {
            let resolved = self
                .resolve(
                    &mut cache,
                    &item.uri,
                    &item.name,
                    item.range,
                    item.selection_range.start,
                )
                .await;
            if let Some(s) = resolved.filter(|s| !out.iter().any(|o| o.sym == s.sym)) {
                out.push(s);
            }
        }
        out
    }
}

fn extension(file: &RepoPath) -> &str {
    std::str::from_utf8(&file.0)
        .ok()
        .and_then(|p| p.rsplit_once('.'))
        .map_or("", |(_, ext)| ext)
}

fn outline(symbols: &[lsp::DocumentSymbol]) -> Vec<Outline> {
    symbols
        .iter()
        .map(|s| Outline {
            name: s.name.clone(),
            span: span(s.range),
            at: pos(s.selection_range.start),
            detail: s.detail.clone(),
            children: outline(s.children.as_deref().unwrap_or_default()),
        })
        .collect()
}

fn symbols_in(file: &RepoPath, response: Option<lsp::DocumentSymbolResponse>) -> Vec<SymbolInfo> {
    match response {
        None => Vec::new(),
        Some(lsp::DocumentSymbolResponse::Nested(nested)) => symbols::flatten(file, &outline(&nested)),
        Some(lsp::DocumentSymbolResponse::Flat(flat)) => flat
            .into_iter()
            .map(|s| SymbolInfo {
                sym: Sym {
                    file: file.clone(),
                    name: symbols::qualified(s.container_name, s.name),
                },
                span: span(s.location.range),
                at: pos(s.location.range.start),
                detail: None,
            })
            .collect(),
    }
}

fn flatten_hover(contents: lsp::HoverContents) -> String {
    let marked = |m: lsp::MarkedString| match m {
        lsp::MarkedString::String(s) => s,
        lsp::MarkedString::LanguageString(l) => format!("```{}\n{}\n```", l.language, l.value),
    };
    match contents {
        lsp::HoverContents::Scalar(m) => marked(m),
        lsp::HoverContents::Array(ms) => ms.into_iter().map(marked).collect::<Vec<_>>().join("\n\n"),
        lsp::HoverContents::Markup(m) => m.value,
    }
}

fn convert_diagnostic(d: lsp::Diagnostic) -> Diagnostic {
    let severity = match d.severity {
        Some(lsp::DiagnosticSeverity::WARNING) => Severity::Warning,
        Some(lsp::DiagnosticSeverity::INFORMATION) => Severity::Info,
        Some(lsp::DiagnosticSeverity::HINT) => Severity::Hint,
        _ => Severity::Error,
    };
    Diagnostic {
        span: span(d.range),
        severity,
        message: d.message,
        source: d.source,
        code: d.code.map(|c| match c {
            lsp::NumberOrString::Number(n) => n.to_string(),
            lsp::NumberOrString::String(s) => s,
        }),
    }
}

impl LanguageServer for LspServer {
    async fn document_symbols(&self, file: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
        let timed = self.begin(file, Needs::Text).await?;
        let params = lsp::DocumentSymbolParams {
            text_document: self.id(file)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = self.conn.request::<DocumentSymbolRequest>(params).await;
        self.log_query("documentSymbol", file.to_string_lossy(), &timed, "");
        Ok(symbols_in(file, response.map_err(err)?))
    }

    async fn text_symbols(&self, file: &RepoPath, text: &str) -> Result<Vec<SymbolInfo>, QueryError> {
        let started = Instant::now();
        let uri = self.virtual_uri(file)?;
        self.conn
            .notify::<DidOpenTextDocument>(lsp::DidOpenTextDocumentParams {
                text_document: lsp::TextDocumentItem {
                    uri: uri.clone(),
                    language_id: language_id(extension(file)).into(),
                    version: 1,
                    text: text.to_owned(),
                },
            })
            .await
            .map_err(err)?;
        let params = lsp::DocumentSymbolParams {
            text_document: lsp::TextDocumentIdentifier { uri: uri.clone() },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = self.conn.request::<DocumentSymbolRequest>(params).await;
        let closed = self
            .conn
            .notify::<DidCloseTextDocument>(lsp::DidCloseTextDocumentParams {
                text_document: lsp::TextDocumentIdentifier { uri },
            })
            .await;
        let response = response.map_err(err)?;
        closed.map_err(err)?;
        let timed = Timed {
            started,
            sync: Duration::ZERO,
            synced: Synced {
                changed: true,
                lock: Duration::ZERO,
                quiet: None,
            },
        };
        self.log_query("documentSymbol (text)", file.to_string_lossy(), &timed, "");
        Ok(symbols_in(file, response))
    }

    async fn definition(&self, at: &At) -> Result<Vec<Location>, QueryError> {
        let timed = self.begin(&at.file, Needs::Project).await?;
        let params = lsp::GotoDefinitionParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = self.conn.request::<GotoDefinition>(params).await;
        self.log_query("definition", Place(at), &timed, "");
        Ok(match response.map_err(err)? {
            None => Vec::new(),
            Some(lsp::GotoDefinitionResponse::Scalar(l)) => vec![self.location(&l.uri, l.range)],
            Some(lsp::GotoDefinitionResponse::Array(ls)) => ls.iter().map(|l| self.location(&l.uri, l.range)).collect(),
            Some(lsp::GotoDefinitionResponse::Link(ls)) => ls
                .iter()
                .map(|l| self.location(&l.target_uri, l.target_selection_range))
                .collect(),
        })
    }

    async fn references(&self, at: &At) -> Result<Vec<Location>, QueryError> {
        let timed = self.begin(&at.file, Needs::Project).await?;
        let params = lsp::ReferenceParams {
            text_document_position: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp::ReferenceContext {
                include_declaration: false,
            },
        };
        let response = self.conn.request::<References>(params).await;
        let found = response.as_ref().map_or(0, |r| r.as_ref().map_or(0, Vec::len));
        self.log_query("references", Place(at), &timed, &format!(", {found} found"));
        Ok(response
            .map_err(err)?
            .unwrap_or_default()
            .iter()
            .map(|l| self.location(&l.uri, l.range))
            .collect())
    }

    async fn hover(&self, at: &At) -> Result<Option<String>, QueryError> {
        let timed = self.begin(&at.file, Needs::Project).await?;
        let params = lsp::HoverParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let response = self.conn.request::<HoverRequest>(params).await;
        self.log_query("hover", Place(at), &timed, "");
        Ok(response.map_err(err)?.map(|h| flatten_hover(h.contents)))
    }

    async fn incoming_calls(&self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
        let mut callers = Vec::new();
        for item in self.prepare_calls(at).await? {
            let params = lsp::CallHierarchyIncomingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let started = Instant::now();
            let calls = self.conn.request::<CallHierarchyIncomingCalls>(params).await;
            let found = calls.as_ref().map_or(0, |c| c.as_ref().map_or(0, Vec::len));
            self.log_request(
                "callHierarchy/incomingCalls",
                Place(at),
                &format!("{found} calls"),
                started,
            );
            callers.extend(
                calls
                    .map_err(err)?
                    .unwrap_or_default()
                    .into_iter()
                    .map(|call| call.from),
            );
        }
        Ok(self.symbols_of(callers).await)
    }

    async fn outgoing_calls(&self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
        let mut callees = Vec::new();
        for item in self.prepare_calls(at).await? {
            let params = lsp::CallHierarchyOutgoingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let started = Instant::now();
            let calls = self.conn.request::<CallHierarchyOutgoingCalls>(params).await;
            let found = calls.as_ref().map_or(0, |c| c.as_ref().map_or(0, Vec::len));
            self.log_request(
                "callHierarchy/outgoingCalls",
                Place(at),
                &format!("{found} calls"),
                started,
            );
            callees.extend(calls.map_err(err)?.unwrap_or_default().into_iter().map(|call| call.to));
        }
        Ok(self.symbols_of(callees).await)
    }

    async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>, QueryError> {
        let started = Instant::now();
        let params = lsp::WorkspaceSymbolParams {
            query: query.to_owned(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = self.conn.request::<WorkspaceSymbolRequest>(params).await;
        self.log_request("workspace/symbol", format!("{query:?}"), "no sync", started);
        let response = response.map_err(err)?;
        // (uri, name, range if the server gave one)
        let found: Vec<(Uri, String, Option<lsp::Range>)> = match response {
            None => Vec::new(),
            Some(lsp::WorkspaceSymbolResponse::Flat(symbols)) => symbols
                .into_iter()
                .map(|s| (s.location.uri, s.name, Some(s.location.range)))
                .collect(),
            Some(lsp::WorkspaceSymbolResponse::Nested(symbols)) => symbols
                .into_iter()
                .map(|s| match s.location {
                    lsp::OneOf::Left(l) => (l.uri, s.name, Some(l.range)),
                    lsp::OneOf::Right(l) => (l.uri, s.name, None),
                })
                .collect(),
        };
        let mut cache = HashMap::new();
        let mut out = Vec::new();
        for (uri, name, range) in found {
            let Some(file) = self.file_of(&uri) else { continue };
            let resolved = match range {
                Some(range) => self.resolve(&mut cache, &uri, &name, range, range.start).await,
                None => {
                    let symbols = self.symbols_cached(&mut cache, &file).await;
                    symbols.into_iter().find(|s| s.sym.name.last() == Some(&name))
                }
            };
            out.extend(resolved);
        }
        Ok(out)
    }

    async fn diagnostics(&self, file: &RepoPath) -> Result<Vec<Diagnostic>, QueryError> {
        let key = key(&self.uri_of(file)?);
        let generation = |conn: &Connection| conn.with_state(|s| s.diagnostics.get(&key).map_or(0, |d| d.generation));
        let before = generation(&self.conn);
        let timed = self.begin(file, Needs::Project).await?;
        let changed = timed.synced.changed;
        let items = match self.diagnostics {
            Diagnostics::Pull => {
                let params = lsp::DocumentDiagnosticParams {
                    text_document: self.id(file)?,
                    identifier: None,
                    previous_result_id: None,
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                };
                let report = self.conn.request::<DocumentDiagnosticRequest>(params).await;
                self.log_query("diagnostics", file.to_string_lossy(), &timed, ", pull");
                match report.map_err(err)? {
                    lsp::DocumentDiagnosticReportResult::Report(lsp::DocumentDiagnosticReport::Full(full)) => {
                        full.full_document_diagnostic_report.items
                    }
                    _ => return Err(err("server answered a diagnostic pull without a full report")),
                }
            }
            Diagnostics::Push => {
                let conn = &self.conn;
                let waiting = Instant::now();
                let fresh = conn
                    .wait_for(DIAGNOSTICS_TIMEOUT, |s| {
                        s.diagnostics
                            .get(&key)
                            .is_some_and(|d| d.generation > before || (!changed && d.generation > 0))
                    })
                    .await;
                let detail = format!(
                    ", push wait {} ms{}",
                    waiting.elapsed().as_millis(),
                    if fresh { "" } else { ", timed out" }
                );
                self.log_query("diagnostics", file.to_string_lossy(), &timed, &detail);
                if !fresh {
                    return Err(err("the server published no diagnostics for the file in time"));
                }
                conn.with_state(|s| s.diagnostics.get(&key).map(|d| d.items.clone()).unwrap_or_default())
            }
        };
        Ok(items.into_iter().map(convert_diagnostic).collect())
    }

    async fn shutdown(&self) {
        self.conn.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(opened: Instant) -> Doc {
        Doc {
            file: RepoPath(b"a.ts".to_vec()),
            uri: "file:///a.ts".parse().unwrap(),
            version: 1,
            text: String::new(),
            opened,
        }
    }

    #[test]
    fn the_timing_line_names_a_lock_wait_an_open_and_a_quiet_wait_only_when_they_happened() {
        let plain = Synced {
            changed: false,
            lock: Duration::ZERO,
            quiet: None,
        };
        assert_eq!(plain.describe(Duration::from_millis(3)), "sync 3 ms");
        let busy = Synced {
            changed: true,
            lock: Duration::from_millis(40),
            quiet: Some(Duration::from_millis(800)),
        };
        assert_eq!(
            busy.describe(Duration::from_millis(900)),
            "sync 900 ms, lock 40 ms, opened or changed, quiet wait 800 ms"
        );
    }

    #[test]
    fn one_quiet_wait_settles_the_documents_opened_before_it_began() {
        let before = Instant::now();
        let early = doc(before);
        let late = doc(Instant::now() + Duration::from_secs(1));
        let mut docs = Docs::default();
        assert!(!docs.is_settled(&early));
        docs.quiet_from = Some(before + Duration::from_millis(1));
        assert!(docs.is_settled(&early));
        assert!(!docs.is_settled(&late));
    }
}
