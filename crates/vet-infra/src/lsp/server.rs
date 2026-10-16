//! A started language server: the `LanguageServer` port over a [`Connection`].
//!
//! Open documents are mirrored before every query (full-text sync): on an
//! immutable tree from disk, whose files are the snapshot's; on the live
//! worktree from the revision's texts where the query carries them, so answers
//! keep the positions the revision was counted with, and from disk where it
//! has no text of a file. Nothing is cached here; whether an answer may be
//! reused is decided where answers are kept (I1), which asks the server only
//! whether it was at work on its own.
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
use vet_semantic::join::bounded;
use vet_semantic::ports::{
    At, Diagnostic, Exited, Features, LanguageServer, Location, OverlayText, Pos, QueryError, Readiness,
    ReadinessChanged, Severity, Span, Sym, SymbolInfo, SymbolKind, Tree,
};
use vet_semantic::symbols::{self, Outline};

use super::capabilities::{Diagnostics, Offered};
use super::rpc::{Connection, Ending, RequestError, State};
use super::uri::{file_uri, key, path_of};
use crate::budget::Budget;
use crate::language::language_id;
use crate::paths;

/// Documents kept open at once; the oldest are closed beyond this.
const MAX_OPEN_DOCS: usize = 64;
/// How many files a search of symbols by name asks the outline of at once.
const WORKSPACE_SYMBOL_FILES_IN_FLIGHT: usize = 8;
const DIAGNOSTICS_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a newly opened document may keep the server busy before a query goes ahead.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How often a query is repeated after the server dropped it, and how long to wait before.
const TRANSIENT_RETRIES: u32 = 20;
const TRANSIENT_BACKOFF: Duration = Duration::from_millis(100);
/// The shortest lock wait worth reporting in a timing description.
const MIN_REPORTED_LOCK: Duration = Duration::from_millis(1);

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
        if self.lock >= MIN_REPORTED_LOCK {
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
    offered: Offered,
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
    QueryError::Failed(e.to_string())
}

fn request_failed(e: RequestError) -> QueryError {
    match e.ending {
        Ending::Exited => QueryError::Exited(e.message),
        Ending::TimedOut => QueryError::TimedOut(e.message),
        Ending::Rejected => QueryError::Failed(e.message),
    }
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
        offered: Offered,
        loads_project_on_open: bool,
        io: Budget,
    ) -> Self {
        Self {
            conn,
            root,
            tree,
            offered,
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
    async fn begin(&self, file: &RepoPath, needs: Needs, overlay: Option<&OverlayText>) -> Result<Timed, QueryError> {
        let started = Instant::now();
        let synced = self.sync(file, needs, overlay).await?;
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
        overlay: Option<OverlayText>,
    ) -> Result<Asked<R::Result>, QueryError>
    where
        R::Params: Clone,
    {
        let started = Instant::now();
        let mut retries = 0;
        loop {
            let mut timed = match file {
                Some((file, needs)) => self.begin(file, needs, overlay.as_ref()).await?,
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
                        response: response.map_err(request_failed),
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
    /// document from the overlay's texts, where it has them, and the disk
    /// otherwise. Snapshot files never change, so they are opened once and left
    /// alone, and no overlay is asked: their disk is the revision.
    async fn sync(&self, file: &RepoPath, needs: Needs, overlay: Option<&OverlayText>) -> Result<Synced, QueryError> {
        let asked = Instant::now();
        let mut docs = self.docs.lock().await;
        let lock = asked.elapsed();
        let changed = if self.tree == Tree::Immutable && docs.open.iter().any(|d| &d.file == file) {
            false
        } else if self.tree == Tree::Immutable {
            self.refresh(&mut docs.open, file, None).await?
        } else {
            let overlay = overlay.cloned();
            let open: Vec<_> = docs.open.iter().map(|d| d.file.clone()).collect();
            for other in open.into_iter().filter(|f| f != file) {
                self.refresh(&mut docs.open, &other, overlay.as_ref()).await?;
            }
            self.refresh(&mut docs.open, file, overlay.as_ref()).await?
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
        self.conn.until_quiet(OPEN_TIMEOUT).await;
        let mut docs = self.docs.lock().await;
        docs.quiet_from = docs.quiet_from.max(Some(from));
        from.elapsed()
    }

    /// Brings the server's copy of `file` in line with the overlay's text, or the disk's where
    /// the overlay has none. Whether the file changed. Every change to `docs` follows its
    /// message without an await in between, so a dropped query never leaves `docs` out of step
    /// with the server.
    async fn refresh(
        &self,
        docs: &mut Vec<Doc>,
        file: &RepoPath,
        overlay: Option<&OverlayText>,
    ) -> Result<bool, QueryError> {
        let path = self.absolute(file)?;
        let read = path.clone();
        let disk = self
            .io
            .blocking(move || std::fs::read(read))
            .await
            .map_err(std::io::Error::other)
            .and_then(|read| read)
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let index = docs.iter().position(|d| &d.file == file);
        let text = match (disk, index) {
            (Ok(disk), _) => match overlay.map(|o| o.text(file)) {
                // The revision's text wins where it is known; `None` keeps the disk's.
                Some(read) => read.await.unwrap_or(disk),
                None => disk,
            },
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
                        language_id: language_id(file).into(),
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

    async fn symbols_cached(
        &self,
        cache: &mut HashMap<RepoPath, Vec<SymbolInfo>>,
        file: &RepoPath,
        overlay: Option<&OverlayText>,
    ) -> Vec<SymbolInfo> {
        if !cache.contains_key(file) {
            let symbols = self.document_symbols(file, overlay.cloned()).await.unwrap_or_default();
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
        overlay: Option<&OverlayText>,
    ) -> Option<SymbolInfo> {
        let file = self.file_of(uri)?;
        let symbols = self.symbols_cached(cache, &file, overlay).await;
        symbols::resolve(&symbols, file, name, span(range), pos(selection))
    }

    async fn prepare_calls(
        &self,
        at: &At,
        overlay: Option<OverlayText>,
    ) -> Result<Vec<lsp::CallHierarchyItem>, QueryError> {
        let params = lsp::CallHierarchyPrepareParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let Asked { response: items, timed } = self
            .request::<CallHierarchyPrepare>(Some((&at.file, Needs::Project)), params, overlay)
            .await?;
        let found = items.as_ref().map_or(0, |i| i.as_ref().map_or(0, Vec::len));
        self.log_query("callHierarchy/prepare", Place(at), &timed, &format!(", {found} items"));
        Ok(items?.unwrap_or_default())
    }

    /// The symbols on the other end of the calls of the declaration at `at`, asked with request `R`:
    /// `params` makes its parameters for one item, `other_end` takes the item from a call.
    async fn calls<R, C>(
        &self,
        at: &At,
        overlay: Option<OverlayText>,
        params: impl Fn(lsp::CallHierarchyItem) -> R::Params,
        other_end: impl Fn(C) -> lsp::CallHierarchyItem,
    ) -> Result<Vec<SymbolInfo>, QueryError>
    where
        R: Request<Result = Option<Vec<C>>>,
        R::Params: Clone,
    {
        let mut items = Vec::new();
        for item in self.prepare_calls(at, overlay.clone()).await? {
            let Asked { response: calls, timed } = self
                .request::<R>(Some((&at.file, Needs::Project)), params(item), overlay.clone())
                .await?;
            let found = calls.as_ref().map_or(0, |c| c.as_ref().map_or(0, Vec::len));
            self.log_request(R::METHOD, Place(at), &format!("{found} calls"), timed.started);
            items.extend(calls?.unwrap_or_default().into_iter().map(&other_end));
        }
        Ok(self.symbols_of(items, overlay.as_ref()).await)
    }

    /// The symbols the call hierarchy `items` stand for, each once, those outside the root
    /// and the code outside every declaration left out. A server reports that code as a
    /// module item named after the file.
    async fn symbols_of(&self, items: Vec<lsp::CallHierarchyItem>, overlay: Option<&OverlayText>) -> Vec<SymbolInfo> {
        let mut cache = HashMap::new();
        let mut out: Vec<SymbolInfo> = Vec::new();
        for item in items {
            if matches!(item.kind, lsp::SymbolKind::MODULE | lsp::SymbolKind::FILE) {
                let Some(file) = self.file_of(&item.uri) else { continue };
                let symbols = self.symbols_cached(&mut cache, &file, overlay).await;
                if !symbols::is_declared_at(&symbols, &item.name, pos(item.selection_range.start)) {
                    continue;
                }
            }
            let resolved = self
                .resolve(
                    &mut cache,
                    &item.uri,
                    &item.name,
                    item.range,
                    item.selection_range.start,
                    overlay,
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
                language_id: language_id(file).into(),
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

fn outline(symbols: &[lsp::DocumentSymbol]) -> Vec<Outline> {
    symbols
        .iter()
        .map(|s| Outline {
            name: s.name.clone(),
            span: span(s.range),
            at: pos(s.selection_range.start),
            detail: s.detail.clone(),
            container: is_container(s.kind),
            callable: is_callable(s.kind),
            kind: symbol_kind(s.kind),
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
                kind: symbol_kind(s.kind),
                modules: 0,
            })
            .collect(),
    }
}

fn symbol_kind(kind: lsp::SymbolKind) -> SymbolKind {
    if is_callable(kind) {
        SymbolKind::Function
    } else if [
        lsp::SymbolKind::MODULE,
        lsp::SymbolKind::NAMESPACE,
        lsp::SymbolKind::PACKAGE,
    ]
    .contains(&kind)
    {
        SymbolKind::Module
    } else if is_container(kind) {
        SymbolKind::Type
    } else {
        SymbolKind::Other
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

/// Whether symbols of `kind` are executable code with locals of its own.
fn is_callable(kind: lsp::SymbolKind) -> bool {
    [
        lsp::SymbolKind::METHOD,
        lsp::SymbolKind::CONSTRUCTOR,
        lsp::SymbolKind::FUNCTION,
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

    fn readiness_changed(&self, seen: Readiness) -> ReadinessChanged {
        self.conn.readiness_changed(seen)
    }

    fn features(&self) -> Features {
        self.offered.features
    }

    async fn document_symbols(
        &self,
        file: &RepoPath,
        overlay: Option<OverlayText>,
    ) -> Result<Vec<SymbolInfo>, QueryError> {
        let params = lsp::DocumentSymbolParams {
            text_document: self.id(file)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<DocumentSymbolRequest>(Some((file, Needs::Text)), params, overlay)
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
        let Asked { response, mut timed } = self.request::<DocumentSymbolRequest>(None, params, None).await?;
        drop(doc);
        timed.synced.changed = true;
        self.log_query("documentSymbol (text)", file.to_string_lossy(), &timed, "");
        Ok(symbols_in(file, response?))
    }

    async fn definition(&self, at: &At, overlay: Option<OverlayText>) -> Result<Vec<Location>, QueryError> {
        let params = lsp::GotoDefinitionParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<GotoDefinition>(Some((&at.file, Needs::Project)), params, overlay)
            .await?;
        self.log_query("definition", Place(at), &timed, "");
        Ok(self.goto_locations(response?))
    }

    async fn implementations(&self, at: &At, overlay: Option<OverlayText>) -> Result<Vec<Location>, QueryError> {
        let params = lsp::request::GotoImplementationParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<GotoImplementation>(Some((&at.file, Needs::Project)), params, overlay)
            .await?;
        let response = response?;
        let found = self.goto_locations(response);
        self.log_query("implementation", Place(at), &timed, &format!(", {} found", found.len()));
        Ok(found)
    }

    async fn references(&self, at: &At, overlay: Option<OverlayText>) -> Result<Vec<Location>, QueryError> {
        let params = lsp::ReferenceParams {
            text_document_position: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp::ReferenceContext {
                include_declaration: false,
            },
        };
        let Asked { response, timed } = self
            .request::<References>(Some((&at.file, Needs::Project)), params, overlay)
            .await?;
        let found = response.as_ref().map_or(0, |r| r.as_ref().map_or(0, Vec::len));
        self.log_query("references", Place(at), &timed, &format!(", {found} found"));
        Ok(response?
            .unwrap_or_default()
            .iter()
            .map(|l| self.location(&l.uri, l.range))
            .collect())
    }

    async fn hover(&self, at: &At, overlay: Option<OverlayText>) -> Result<Option<String>, QueryError> {
        let params = lsp::HoverParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let Asked { response, timed } = self
            .request::<HoverRequest>(Some((&at.file, Needs::Project)), params, overlay)
            .await?;
        self.log_query("hover", Place(at), &timed, "");
        Ok(response?.map(|h| flatten_hover(h.contents)))
    }

    async fn incoming_calls(&self, at: &At, overlay: Option<OverlayText>) -> Result<Vec<SymbolInfo>, QueryError> {
        self.calls::<CallHierarchyIncomingCalls, _>(
            at,
            overlay,
            |item| lsp::CallHierarchyIncomingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
            |call| call.from,
        )
        .await
    }

    async fn outgoing_calls(&self, at: &At, overlay: Option<OverlayText>) -> Result<Vec<SymbolInfo>, QueryError> {
        self.calls::<CallHierarchyOutgoingCalls, _>(
            at,
            overlay,
            |item| lsp::CallHierarchyOutgoingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
            |call| call.to,
        )
        .await
    }

    async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>, QueryError> {
        let params = lsp::WorkspaceSymbolParams {
            query: query.to_owned(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let Asked { response, timed } = self.request::<WorkspaceSymbolRequest>(None, params, None).await?;
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
        // The qualified names come from each file's document symbols; the files are asked together.
        let mut files: Vec<RepoPath> = Vec::new();
        for (uri, _, _) in &found {
            if let Some(file) = self.file_of(uri)
                && !files.contains(&file)
            {
                files.push(file);
            }
        }
        let mut cache: HashMap<RepoPath, Vec<SymbolInfo>> =
            bounded(WORKSPACE_SYMBOL_FILES_IN_FLIGHT, files, |file| async move {
                let symbols = self.document_symbols(&file, None).await.unwrap_or_default();
                (file, symbols)
            })
            .await
            .into_iter()
            .collect();
        let mut out = Vec::new();
        for (uri, name, range) in found {
            let Some(file) = self.file_of(&uri) else { continue };
            let resolved = match range {
                Some(range) => self.resolve(&mut cache, &uri, &name, range, range.start, None).await,
                None => {
                    let symbols = self.symbols_cached(&mut cache, &file, None).await;
                    symbols.into_iter().find(|s| s.sym.name.last() == Some(&name))
                }
            };
            out.extend(resolved);
        }
        Ok(out)
    }

    async fn diagnostics(&self, file: &RepoPath, overlay: Option<OverlayText>) -> Result<Vec<Diagnostic>, QueryError> {
        let key = key(&self.uri_of(file)?);
        let generation = |conn: &Connection| conn.with_state(|s| s.diagnostics.get(&key).map_or(0, |d| d.generation));
        let before = generation(&self.conn);
        let items = match self.offered.diagnostics {
            Diagnostics::Pull => {
                let params = lsp::DocumentDiagnosticParams {
                    text_document: self.id(file)?,
                    identifier: None,
                    previous_result_id: None,
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                };
                let Asked { response, timed } = self
                    .request::<DocumentDiagnosticRequest>(Some((file, Needs::Project)), params, overlay)
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
                let timed = self.begin(file, Needs::Project, overlay.as_ref()).await?;
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

    fn exited(&self) -> Exited {
        self.conn.exited()
    }

    async fn shutdown(&self) {
        self.conn.close().await;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap as Map;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use serde_json::{Value, json};
    use vet_semantic::ports::Overlay;

    use super::*;
    use crate::lsp::fake::{self, Fake, Reply};

    const OFFERED: Offered = Offered {
        diagnostics: Diagnostics::Pull,
        features: Features::ALL,
    };
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
            LspServer::new(conn, root, Tree::Live, OFFERED, false, Budget::new(1)),
            fake,
            dir,
        )
    }

    fn requests(fake: &Fake) -> usize {
        fake.messages("textDocument/documentSymbol").len()
    }

    /// The texts of a revision, as the queries of it carry them: one named file's
    /// text, `None` and so the disk's for every other file.
    struct Given(Arc<Map<RepoPath, String>>);

    impl Overlay for Given {
        fn text(&self, file: &RepoPath) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
            let text = self.0.get(file).cloned();
            Box::pin(async move { text })
        }
    }

    /// The texts of the revision: `file` has `text`, every other file has none.
    fn revision_texts(file: &str, text: &str) -> OverlayText {
        Arc::new(Given(Arc::new(Map::from_iter([(path(file), text.to_owned())]))))
    }

    fn uri_of(server: &LspServer, file: &str) -> String {
        server.uri_of(&path(file)).unwrap().as_str().to_owned()
    }

    fn steps(log: &[Value]) -> Vec<String> {
        log.iter()
            .filter_map(|m| {
                let method = m["method"].as_str()?;
                match method {
                    "textDocument/didOpen" => Some(format!("open {}", m["params"]["textDocument"]["text"])),
                    "textDocument/didChange" => Some(format!("change {}", m["params"]["contentChanges"][0]["text"])),
                    _ => None,
                }
            })
            .collect()
    }

    #[tokio::test]
    async fn a_live_server_is_asked_about_the_revision_texts_even_where_the_disk_has_shifted() {
        let (server, fake, _dir) = live_server(&[("a.rs", "shifted\nlines on disk")], |_| Reply::Result(Value::Null));
        let at = |line, col| At {
            file: path("a.rs"),
            pos: Pos { line, col },
        };
        // The agent has shifted the lines of `a.rs` since the snapshot settled: the revision's
        // text is not the disk's, and the disk's line 1 does not hover at the revision's.
        server
            .hover(&at(1, 4), Some(revision_texts("a.rs", "one\ntwo")))
            .await
            .unwrap();
        assert_eq!(
            fake.documents(),
            [(uri_of(&server, "a.rs"), "one\ntwo".to_owned())].into(),
            "the server was fed the revision's text, not the disk's"
        );

        // A file the revision holds no retained text of keeps the disk's.
        server
            .hover(&at(0, 0), Some(revision_texts("b.rs", "other file")))
            .await
            .unwrap();
        assert_eq!(
            fake.documents(),
            [(uri_of(&server, "a.rs"), "shifted\nlines on disk".to_owned())].into(),
            "without the revision's text the server was fed what the disk holds"
        );
        assert!(fake.violations().is_empty());
    }

    #[tokio::test]
    async fn a_newer_revision_replaces_the_texts_a_live_server_saw_and_falls_back_where_it_has_none() {
        let (server, fake, _dir) = live_server(&[("a.rs", "disk")], |_| Reply::Result(Value::Null));
        let at = At {
            file: path("a.rs"),
            pos: Pos { line: 0, col: 0 },
        };
        server.hover(&at, Some(revision_texts("a.rs", "one"))).await.unwrap();
        // The revision of the next query holds no text of the file: the disk's is the server's.
        server.hover(&at, Some(revision_texts("b.rs", "only b"))).await.unwrap();
        server.hover(&at, Some(revision_texts("a.rs", "two"))).await.unwrap();
        assert_eq!(
            fake.documents(),
            [(uri_of(&server, "a.rs"), "two".to_owned())].into(),
            "the texts of the newer revision replaced the older ones"
        );
        let log = fake.log();
        let steps = steps(&log);
        assert_eq!(
            steps,
            ["open \"one\"", "change \"disk\"", "change \"two\""],
            "{steps:?}"
        );
        assert!(fake.violations().is_empty());
    }

    #[tokio::test]
    async fn an_immutable_tree_is_not_fed_the_texts_of_an_overlay() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "on disk").unwrap();
        let (conn, fake) = fake::start(|_| Reply::Result(Value::Null));
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let server = LspServer::new(conn, root, Tree::Immutable, OFFERED, false, Budget::new(1));
        server
            .hover(
                &At {
                    file: path("a.rs"),
                    pos: Pos { line: 0, col: 0 },
                },
                Some(revision_texts("a.rs", "not the tree's")),
            )
            .await
            .unwrap();
        assert_eq!(
            fake.documents(),
            [(uri_of(&server, "a.rs"), "on disk".to_owned())].into(),
            "a snapshot's disk is the revision; the overlay has no say"
        );
        assert!(fake.violations().is_empty());
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
            server.document_symbols(&path("a.rs"), None).await.unwrap();
            hold.store(true, Ordering::SeqCst);
            std::fs::write(dir.path().join("a.rs"), "two").unwrap();
            let dropped = tokio::time::timeout(
                Duration::from_millis(delay),
                server.document_symbols(&path("b.rs"), None),
            )
            .await;
            assert!(dropped.is_err(), "{delay} ms");

            std::fs::write(dir.path().join("a.rs"), "three").unwrap();
            hold.store(false, Ordering::SeqCst);
            server.document_symbols(&path("b.rs"), None).await.unwrap();

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
            vet_semantic::join::join(revisions.until_superseded(1, server.hover(&at, None)), superseding).await;
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
            let server = LspServer::new(conn, root, Tree::Live, OFFERED, false, Budget::new(1));
            server.document_symbols(&path("a.rs"), None).await.unwrap();

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
        let dropped =
            tokio::time::timeout(Duration::from_millis(250), server.document_symbols(&path("a.rs"), None)).await;
        assert!(dropped.is_err());
        let asked = requests(&fake);
        assert!(asked > 1, "{asked}");
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(requests(&fake), asked);
    }

    #[tokio::test]
    async fn an_error_that_is_not_transient_is_not_asked_again() {
        let (server, fake, _dir) = live_server(&[("a.rs", "x")], |_| Reply::Error(-32602));
        let error = server.document_symbols(&path("a.rs"), None).await.unwrap_err();
        assert_eq!(error, QueryError::Failed("textDocument/documentSymbol: failed".into()));
        assert_eq!(requests(&fake), 1);
    }

    /// Longest a query that is not meant to wait may take; it only tells a hang from an answer.
    const HANG: Duration = Duration::from_secs(10);

    fn project_server(loads_project_on_open: bool, tree: Tree, files: &[&str]) -> (LspServer, Fake, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        for name in files {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        let (conn, fake) = fake::start(|_| Reply::Result(Value::Null));
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (
            LspServer::new(conn, root, tree, OFFERED, loads_project_on_open, Budget::new(1)),
            fake,
            dir,
        )
    }

    fn at(file: &str) -> At {
        At {
            file: path(file),
            pos: Pos { line: 0, col: 0 },
        }
    }

    /// The server reports work in progress, and the client has taken note of it.
    async fn begin_work(server: &LspServer, fake: &Fake) {
        let progress = |kind| json!({"token": "indexing", "value": {"kind": kind, "title": "Indexing"}});
        fake.notify("$/progress", progress("begin")).await;
        while server.conn.with_state(|s| s.readiness().idle) {
            tokio::task::yield_now().await;
        }
    }

    async fn end_work(fake: &Fake) {
        fake.notify("$/progress", json!({"token": "indexing", "value": {"kind": "end"}}))
            .await;
    }

    fn opened(log: &[Value]) -> usize {
        log.iter().filter(|m| m["method"] == "textDocument/didOpen").count()
    }

    #[tokio::test]
    async fn a_server_that_has_its_projects_loaded_when_ready_is_asked_while_it_is_busy() {
        for tree in [Tree::Immutable, Tree::Live] {
            let files = ["c0.cs", "c1.cs", "c2.cs", "c3.cs"];
            let (server, fake, _dir) = project_server(false, tree, &files);
            begin_work(&server, &fake).await;
            let hovers = async {
                for file in files {
                    server.hover(&at(file), None).await.unwrap();
                }
            };
            tokio::time::timeout(HANG, hovers)
                .await
                .expect("the hovers waited for the busy server to go quiet");
        }
    }

    #[tokio::test]
    async fn documents_opened_together_wait_for_the_server_to_go_quiet_together_and_a_later_one_waits_again() {
        let files = ["a0.ts", "a1.ts", "a2.ts", "a3.ts", "a4.ts", "a5.ts"];
        let (server, fake, _dir) = project_server(true, Tree::Live, &[&files[..], &["late.ts"]].concat());
        begin_work(&server, &fake).await;

        let together: Vec<At> = files.map(at).into();
        let ask = vet_semantic::join::bounded(6, &together, |at| server.hover(at, None));
        let release = async {
            fake.until("every document opened", |log| opened(log) == 6).await;
            assert!(
                fake.messages("textDocument/hover").is_empty(),
                "asked while the server was busy"
            );
            end_work(&fake).await;
        };
        let (answers, ()) = tokio::time::timeout(HANG, vet_semantic::join::join(ask, release))
            .await
            .expect("the documents opened together did not share one wait");
        assert!(answers.iter().all(Result::is_ok));

        // The server is busy again: a document opened after the wait began waits for the next one,
        // one the wait covered is answered at once.
        begin_work(&server, &fake).await;
        let hovered = fake.messages("textDocument/hover").len();
        let late_at = at("late.ts");
        let late = server.hover(&late_at, None);
        let release = async {
            fake.until("the late document opened", |log| opened(log) == 7).await;
            assert_eq!(
                fake.messages("textDocument/hover").len(),
                hovered,
                "the late one was asked at once"
            );
            server.hover(&together[0], None).await.unwrap();
            end_work(&fake).await;
        };
        let (late, ()) = tokio::time::timeout(HANG, vet_semantic::join::join(late, release))
            .await
            .expect("a document opened after the wait began was not waited for");
        late.unwrap();
    }
}
