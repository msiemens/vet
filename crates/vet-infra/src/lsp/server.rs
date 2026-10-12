//! A started language server: the `LanguageServer` port over a [`Connection`].
//!
//! Open documents are mirrored from disk before every query (full-text sync),
//! so on the live worktree the server sees an agent's latest write. Nothing is
//! cached here; whether an answer may be reused is decided where answers are
//! kept (I2), which asks the server only whether it was at work on its own.
//!
//! A query may be dropped at any await. Sync is safe against that because
//! messages are queued rather than awaited: a document's state in [`Docs`]
//! changes in the same step as the message that tells the server. A query that
//! opens a document of its own closes it when dropped.

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
    DocumentSymbolRequest, GotoDefinition, GotoImplementation, HoverRequest, References, Request,
    WorkspaceSymbolRequest,
};
use vet_core::content::RepoPath;
use vet_semantic::ports::{
    At, Diagnostic, LanguageServer, Location, Pos, QueryError, Readiness, Severity, Span, Sym, SymbolInfo, Tree,
};
use vet_semantic::symbols::{self, Outline};

use super::QUIET;
use super::capabilities::Diagnostics;
use super::rpc::{Connection, State};
use super::uri::{file_uri, key, path_of};
use crate::budget::Budget;
use crate::language::language_id;
use crate::paths;

/// Documents kept open at once; the oldest are closed beyond this.
const MAX_OPEN_DOCS: usize = 64;
const DIAGNOSTICS_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a newly opened document may keep the server busy before a query goes ahead.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How often a query is repeated after the server dropped it, and how long to wait before.
const TRANSIENT_RETRIES: u32 = 20;
const TRANSIENT_BACKOFF: Duration = Duration::from_millis(100);

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

/// What making the server see a file took and did, for the timing lines of the log.
#[derive(Default)]
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
    /// How often the server dropped the request and it was sent again.
    retries: u32,
}

impl Timed {
    fn describe(&self) -> String {
        let sync = self.synced.describe(self.sync);
        match self.retries {
            0 => sync,
            n => format!("{sync}, {n} retries"),
        }
    }
}

/// The server's answer to a query, with its timing.
struct Asked<T> {
    response: Result<T, QueryError>,
    timed: Timed,
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
    /// Whether answers across files wait for the server to go quiet after a document was opened.
    loads_project_on_open: bool,
    /// Numbers the in-memory documents of `text_symbols`, so concurrent calls
    /// for one file do not share a URI.
    virtual_docs: AtomicU64,
    /// Where the files the server is told about are read.
    io: Budget,
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
    pub(super) fn new(
        conn: Connection,
        root: PathBuf,
        tree: Tree,
        diagnostics: Diagnostics,
        loads_project_on_open: bool,
        io: Budget,
    ) -> Self {
        Self {
            conn,
            root,
            tree,
            diagnostics,
            docs: tokio::sync::Mutex::new(Docs::default()),
            loads_project_on_open,
            virtual_docs: AtomicU64::new(0),
            io,
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

    /// The places of an answer to `definition` or `implementation`, in the server's order.
    fn goto_locations(&self, response: Option<lsp::GotoDefinitionResponse>) -> Vec<Location> {
        match response {
            None => Vec::new(),
            Some(lsp::GotoDefinitionResponse::Scalar(l)) => vec![self.location(&l.uri, l.range)],
            Some(lsp::GotoDefinitionResponse::Array(ls)) => ls.iter().map(|l| self.location(&l.uri, l.range)).collect(),
            Some(lsp::GotoDefinitionResponse::Link(ls)) => ls
                .iter()
                .map(|l| self.location(&l.target_uri, l.target_selection_range))
                .collect(),
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
            retries: 0,
        })
    }

    /// Asks the server `R`, after making it see `file` if there is one. A server
    /// that drops the request as transient (a document changed under it, or it
    /// was busy) is asked again, after the document is synced again: on the live
    /// tree the file may have changed once more. Dropping the query ends the
    /// retries and cancels the request in flight.
    async fn request<R: Request>(
        &self,
        file: Option<(&RepoPath, Needs)>,
        params: R::Params,
    ) -> Result<Asked<R::Result>, QueryError>
    where
        R::Params: Clone,
    {
        let started = Instant::now();
        let mut retries = 0;
        loop {
            let mut timed = match file {
                Some((file, needs)) => self.begin(file, needs).await?,
                None => Timed {
                    started,
                    sync: Duration::ZERO,
                    synced: Synced::default(),
                    retries: 0,
                },
            };
            match self.conn.request::<R>(params.clone()).await {
                Err(e) if e.is_transient() && retries < TRANSIENT_RETRIES => {
                    retries += 1;
                    tokio::time::sleep(TRANSIENT_BACKOFF).await;
                }
                response => {
                    timed.started = started;
                    timed.retries = retries;
                    return Ok(Asked {
                        response: response.map_err(err),
                        timed,
                    });
                }
            }
        }
    }

    /// The timing line of a query: `what` names the thing asked about.
    fn log_query(&self, request: &str, what: impl Display, timed: &Timed, detail: &str) {
        vet_semantic::trace::log(
            || format!("{request} {:?} {what} ({}{detail})", self.tree, timed.describe()),
            timed.started,
        );
    }

    /// The timing line of a request that needs no sync of its own.
    fn log_request(&self, request: &str, what: impl Display, detail: &str, started: Instant) {
        vet_semantic::trace::log(|| format!("{request} {:?} {what} ({detail})", self.tree), started);
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
        let unsettled = needs == Needs::Project
            && self.loads_project_on_open
            && docs.open.iter().any(|d| &d.file == file && !docs.is_settled(d));
        drop(docs);
        let quiet = if unsettled { Some(self.settle().await) } else { None };
        Ok(Synced { changed, lock, quiet })
    }

    /// Opening a file can start loading its project (tsserver), and answers
    /// before that finishes resolve nothing outside the file. So a query that
    /// resolves across files waits for the server to go quiet once after the
    /// document was opened; one wait covers every document opened before it.
    /// A server that stays busy past the timeout is asked anyway. Waits of
    /// concurrent queries overlap, so documents opened together are settled
    /// together. How long the wait took.
    async fn settle(&self) -> Duration {
        let from = Instant::now();
        self.conn.with_state(State::touch);
        self.conn.wait_for(OPEN_TIMEOUT, |s| s.is_idle(QUIET)).await;
        let mut docs = self.docs.lock().await;
        docs.quiet_from = docs.quiet_from.max(Some(from));
        from.elapsed()
    }

    /// Brings the server's copy of `file` in line with the disk. Whether the file
    /// changed. Every change to `docs` follows its message without an await in
    /// between, so a dropped query never leaves `docs` out of step with the server.
    async fn refresh(&self, docs: &mut Vec<Doc>, file: &RepoPath) -> Result<bool, QueryError> {
        let path = self.absolute(file)?;
        let read = path.clone();
        let text = self
            .io
            .blocking(move || std::fs::read(read))
            .await
            .map_err(std::io::Error::other)
            .and_then(|read| read)
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
                    .map_err(err)?;
            }
            return Ok(true);
        };
        if docs[index].text == text {
            return Ok(false);
        }
        let doc = &mut docs[index];
        let version = doc.version + 1;
        let params = lsp::DidChangeTextDocumentParams {
            text_document: lsp::VersionedTextDocumentIdentifier {
                uri: doc.uri.clone(),
                version,
            },
            content_changes: vec![lsp::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.clone(),
            }],
        };
        self.conn.notify::<DidChangeTextDocument>(params).map_err(err)?;
        doc.version = version;
        doc.text = text;
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
        let params = lsp::CallHierarchyPrepareParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let Asked { response: items, timed } = self
            .request::<CallHierarchyPrepare>(Some((&at.file, Needs::Project)), params)
            .await?;
        let found = items.as_ref().map_or(0, |i| i.as_ref().map_or(0, Vec::len));
        self.log_query("callHierarchy/prepare", Place(at), &timed, &format!(", {found} items"));
        Ok(items?.unwrap_or_default())
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

/// A document that exists only in the server's memory, for the length of one
/// query. It is closed when dropped, however the query ends.
struct VirtualDoc<'a> {
    conn: &'a Connection,
    uri: Uri,
}

impl<'a> VirtualDoc<'a> {
    fn open(conn: &'a Connection, uri: Uri, file: &RepoPath, text: &str) -> Result<Self, QueryError> {
        conn.notify::<DidOpenTextDocument>(lsp::DidOpenTextDocumentParams {
            text_document: lsp::TextDocumentItem {
                uri: uri.clone(),
                language_id: language_id(extension(file)).into(),
                version: 1,
                text: text.to_owned(),
            },
        })
        .map_err(err)?;
        Ok(Self { conn, uri })
    }
}

impl Drop for VirtualDoc<'_> {
    fn drop(&mut self) {
        // A server that is gone has nothing left open.
        let _ = self
            .conn
            .notify::<DidCloseTextDocument>(lsp::DidCloseTextDocumentParams {
                text_document: lsp::TextDocumentIdentifier { uri: self.uri.clone() },
            });
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
            container: is_container(s.kind),
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
                container: is_container(s.kind),
            })
            .collect(),
    }
}

/// Whether symbols of `kind` hold code. `Object` is what rust-analyzer lists an `impl` as.
fn is_container(kind: lsp::SymbolKind) -> bool {
    [
        lsp::SymbolKind::MODULE,
        lsp::SymbolKind::NAMESPACE,
        lsp::SymbolKind::PACKAGE,
        lsp::SymbolKind::CLASS,
        lsp::SymbolKind::METHOD,
        lsp::SymbolKind::CONSTRUCTOR,
        lsp::SymbolKind::ENUM,
        lsp::SymbolKind::INTERFACE,
        lsp::SymbolKind::FUNCTION,
        lsp::SymbolKind::OBJECT,
        lsp::SymbolKind::STRUCT,
        lsp::SymbolKind::OPERATOR,
    ]
    .contains(&kind)
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
    fn readiness(&self) -> Readiness {
        self.conn.with_state(|state| state.readiness())
    }

    async fn document_symbols(&self, file: &RepoPath) -> Result<Vec<SymbolInfo>, QueryError> {
        let params = lsp::DocumentSymbolParams {
            text_document: self.id(file)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<DocumentSymbolRequest>(Some((file, Needs::Text)), params)
            .await?;
        self.log_query("documentSymbol", file.to_string_lossy(), &timed, "");
        Ok(symbols_in(file, response?))
    }

    async fn text_symbols(&self, file: &RepoPath, text: &str) -> Result<Vec<SymbolInfo>, QueryError> {
        let uri = self.virtual_uri(file)?;
        let doc = VirtualDoc::open(&self.conn, uri, file, text)?;
        let params = lsp::DocumentSymbolParams {
            text_document: lsp::TextDocumentIdentifier { uri: doc.uri.clone() },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, mut timed } = self.request::<DocumentSymbolRequest>(None, params).await?;
        drop(doc);
        timed.synced.changed = true;
        self.log_query("documentSymbol (text)", file.to_string_lossy(), &timed, "");
        Ok(symbols_in(file, response?))
    }

    async fn definition(&self, at: &At) -> Result<Vec<Location>, QueryError> {
        let params = lsp::GotoDefinitionParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<GotoDefinition>(Some((&at.file, Needs::Project)), params)
            .await?;
        self.log_query("definition", Place(at), &timed, "");
        Ok(self.goto_locations(response?))
    }

    async fn implementations(&self, at: &At) -> Result<Vec<Location>, QueryError> {
        let params = lsp::request::GotoImplementationParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<GotoImplementation>(Some((&at.file, Needs::Project)), params)
            .await?;
        let response = response?;
        let found = self.goto_locations(response);
        self.log_query("implementation", Place(at), &timed, &format!(", {} found", found.len()));
        Ok(found)
    }

    async fn references(&self, at: &At) -> Result<Vec<Location>, QueryError> {
        let params = lsp::ReferenceParams {
            text_document_position: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp::ReferenceContext {
                include_declaration: false,
            },
        };
        let Asked { response, timed } = self
            .request::<References>(Some((&at.file, Needs::Project)), params)
            .await?;
        let found = response.as_ref().map_or(0, |r| r.as_ref().map_or(0, Vec::len));
        self.log_query("references", Place(at), &timed, &format!(", {found} found"));
        Ok(response?
            .unwrap_or_default()
            .iter()
            .map(|l| self.location(&l.uri, l.range))
            .collect())
    }

    async fn hover(&self, at: &At) -> Result<Option<String>, QueryError> {
        let params = lsp::HoverParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<HoverRequest>(Some((&at.file, Needs::Project)), params)
            .await?;
        self.log_query("hover", Place(at), &timed, "");
        Ok(response?.map(|h| flatten_hover(h.contents)))
    }

    async fn incoming_calls(&self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
        let mut callers = Vec::new();
        for item in self.prepare_calls(at).await? {
            let params = lsp::CallHierarchyIncomingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let Asked { response: calls, timed } = self
                .request::<CallHierarchyIncomingCalls>(Some((&at.file, Needs::Project)), params)
                .await?;
            let found = calls.as_ref().map_or(0, |c| c.as_ref().map_or(0, Vec::len));
            self.log_request(
                "callHierarchy/incomingCalls",
                Place(at),
                &format!("{found} calls"),
                timed.started,
            );
            callers.extend(calls?.unwrap_or_default().into_iter().map(|call| call.from));
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
            let Asked { response: calls, timed } = self
                .request::<CallHierarchyOutgoingCalls>(Some((&at.file, Needs::Project)), params)
                .await?;
            let found = calls.as_ref().map_or(0, |c| c.as_ref().map_or(0, Vec::len));
            self.log_request(
                "callHierarchy/outgoingCalls",
                Place(at),
                &format!("{found} calls"),
                timed.started,
            );
            callees.extend(calls?.unwrap_or_default().into_iter().map(|call| call.to));
        }
        Ok(self.symbols_of(callees).await)
    }

    async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>, QueryError> {
        let params = lsp::WorkspaceSymbolParams {
            query: query.to_owned(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self.request::<WorkspaceSymbolRequest>(None, params).await?;
        self.log_request("workspace/symbol", format!("{query:?}"), "no sync", timed.started);
        let response = response?;
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
        let items = match self.diagnostics {
            Diagnostics::Pull => {
                let params = lsp::DocumentDiagnosticParams {
                    text_document: self.id(file)?,
                    identifier: None,
                    previous_result_id: None,
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                };
                let Asked { response, timed } = self
                    .request::<DocumentDiagnosticRequest>(Some((file, Needs::Project)), params)
                    .await?;
                self.log_query("diagnostics", file.to_string_lossy(), &timed, ", pull");
                match response? {
                    lsp::DocumentDiagnosticReportResult::Report(lsp::DocumentDiagnosticReport::Full(full)) => {
                        full.full_document_diagnostic_report.items
                    }
                    _ => return Err(err("server answered a diagnostic pull without a full report")),
                }
            }
            Diagnostics::Push => {
                let timed = self.begin(file, Needs::Project).await?;
                let changed = timed.synced.changed;
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use serde_json::Value;

    use super::*;
    use crate::lsp::fake::{self, Fake, Reply};

    const CONTENT_MODIFIED: i64 = -32801;
    const SERVER_CANCELLED: i64 = -32802;

    fn path(p: &str) -> RepoPath {
        RepoPath(p.as_bytes().to_vec())
    }

    /// A server on the live tree of a directory with `files`, talking to a fake.
    fn live_server(
        files: &[(&str, &str)],
        behave: impl Fn(&Value) -> Reply + Send + 'static,
    ) -> (LspServer, Fake, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        let (conn, fake) = fake::start(behave);
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (
            LspServer::new(conn, root, Tree::Live, Diagnostics::Pull, false, Budget::new(1)),
            fake,
            dir,
        )
    }

    fn requests(fake: &Fake) -> usize {
        fake.messages("textDocument/documentSymbol").len()
    }

    #[tokio::test]
    async fn a_query_dropped_anywhere_leaves_the_server_with_the_documents_on_disk() {
        for delay in [0, 1, 2, 3, 5, 8, 30] {
            let hold = Arc::new(AtomicBool::new(true));
            let (server, fake, dir) = live_server(&[("a.rs", "one"), ("b.rs", "b")], {
                let hold = hold.clone();
                move |_| {
                    if hold.load(Ordering::SeqCst) {
                        Reply::Hold
                    } else {
                        Reply::Result(Value::Null)
                    }
                }
            });
            // `a.rs` is open and then changes on disk. The query for `b.rs` opens it and
            // brings `a.rs` up to date, and is dropped after `delay`, in the sync or in
            // the request the server never answers.
            hold.store(false, Ordering::SeqCst);
            server.document_symbols(&path("a.rs")).await.unwrap();
            hold.store(true, Ordering::SeqCst);
            std::fs::write(dir.path().join("a.rs"), "two").unwrap();
            let dropped =
                tokio::time::timeout(Duration::from_millis(delay), server.document_symbols(&path("b.rs"))).await;
            assert!(dropped.is_err(), "{delay} ms");

            std::fs::write(dir.path().join("a.rs"), "three").unwrap();
            hold.store(false, Ordering::SeqCst);
            server.document_symbols(&path("b.rs")).await.unwrap();

            let uri = |f: &str| server.uri_of(&path(f)).unwrap().as_str().to_owned();
            assert!(fake.violations().is_empty(), "{delay} ms: {:?}", fake.violations());
            assert_eq!(
                fake.documents(),
                [(uri("a.rs"), "three".to_owned()), (uri("b.rs"), "b".to_owned())].into(),
                "{delay} ms"
            );
        }
    }

    #[tokio::test]
    async fn a_dropped_query_for_the_symbols_of_a_text_closes_its_document_and_cancels_the_request() {
        let (server, fake, _dir) = live_server(&[], |_| Reply::Hold);
        let dropped = tokio::time::timeout(
            Duration::from_millis(50),
            server.text_symbols(&path("a.rs"), "fn main() {}"),
        )
        .await;
        assert!(dropped.is_err());
        fake.until("the document closed", |log| {
            log.iter().any(|m| m["method"] == "textDocument/didClose")
        })
        .await;
        assert!(fake.documents().is_empty());
        assert!(fake.violations().is_empty());
        assert_eq!(fake.messages("$/cancelRequest").len(), 1);
    }

    #[tokio::test]
    async fn a_query_superseded_by_a_newer_revision_is_dropped_and_cancelled_at_the_server() {
        let (server, fake, _dir) = live_server(&[("a.rs", "x")], |_| Reply::Hold);
        let revisions = vet_app::revisions::Revisions::default();
        let at = At {
            file: path("a.rs"),
            pos: Pos { line: 0, col: 0 },
        };
        let superseding = async {
            fake.until("the hover", |log| {
                log.iter().any(|m| m["method"] == "textDocument/hover")
            })
            .await;
            revisions.advance(2);
        };
        let (answer, ()) =
            vet_semantic::join::join(revisions.until_superseded(1, server.hover(&at)), superseding).await;
        assert_eq!(answer.map(|_| ()), Err(2));

        fake.until("the cancellation", |log| {
            log.iter().any(|m| m["method"] == "$/cancelRequest")
        })
        .await;
        let hover = &fake.messages("textDocument/hover")[0];
        assert_eq!(fake.messages("$/cancelRequest")[0]["params"]["id"], hover["id"]);
    }

    #[tokio::test]
    async fn the_retry_of_a_request_the_server_dropped_syncs_the_document_again() {
        for code in [CONTENT_MODIFIED, SERVER_CANCELLED] {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("a.rs");
            std::fs::write(&file, "v1").unwrap();
            let asked = AtomicUsize::new(0);
            let (conn, fake) = fake::start(move |_| {
                if asked.fetch_add(1, Ordering::SeqCst) == 0 {
                    // The file changes while the server works, and it gives up on the request.
                    std::fs::write(&file, "v2").unwrap();
                    Reply::Error(code)
                } else {
                    Reply::Result(Value::Null)
                }
            });
            let root = std::fs::canonicalize(dir.path()).unwrap();
            let server = LspServer::new(conn, root, Tree::Live, Diagnostics::Pull, false, Budget::new(1));
            server.document_symbols(&path("a.rs")).await.unwrap();

            let steps: Vec<_> = fake
                .log()
                .iter()
                .map(|m| match m["method"].as_str().unwrap() {
                    "textDocument/didOpen" => format!("open {}", m["params"]["textDocument"]["text"]),
                    "textDocument/didChange" => format!("change {}", m["params"]["contentChanges"][0]["text"]),
                    other => other.to_owned(),
                })
                .collect();
            assert_eq!(
                steps,
                [
                    "open \"v1\"",
                    "textDocument/documentSymbol",
                    "change \"v2\"",
                    "textDocument/documentSymbol"
                ],
                "{code}"
            );
        }
    }

    #[tokio::test]
    async fn a_query_dropped_while_the_server_keeps_dropping_it_stops_asking() {
        let (server, fake, _dir) = live_server(&[("a.rs", "x")], |_| Reply::Error(CONTENT_MODIFIED));
        let dropped = tokio::time::timeout(Duration::from_millis(250), server.document_symbols(&path("a.rs"))).await;
        assert!(dropped.is_err());
        let asked = requests(&fake);
        assert!(asked > 1, "{asked}");
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(requests(&fake), asked);
    }

    #[tokio::test]
    async fn an_error_that_is_not_transient_is_not_asked_again() {
        let (server, fake, _dir) = live_server(&[("a.rs", "x")], |_| Reply::Error(-32602));
        let error = server.document_symbols(&path("a.rs")).await.unwrap_err();
        assert_eq!(error.0, "textDocument/documentSymbol: failed");
        assert_eq!(requests(&fake), 1);
    }

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
