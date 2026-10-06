//! A started language server: the `LanguageServer` port over a [`Connection`].
//!
//! Open documents are mirrored from disk before every query (full-text sync),
//! so on the live worktree the server sees an agent's latest write. Nothing is
//! cached here; whether an answer may be reused is the pool's decision (I2).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

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

use super::capabilities::Diagnostics;
use super::config::language_id;
use super::rpc::Connection;

/// Documents kept open at once; the oldest are closed beyond this.
const MAX_OPEN_DOCS: usize = 64;
const DIAGNOSTICS_TIMEOUT: Duration = Duration::from_secs(20);

struct Doc {
    file: RepoPath,
    uri: Uri,
    version: i32,
    text: String,
}

/// Shared by concurrent callers: requests go straight to the connection;
/// only document sync is serialized.
pub struct LspServer {
    conn: Connection,
    /// Canonical, so server URIs can be mapped back to repository paths.
    root: PathBuf,
    tree: Tree,
    diagnostics: Diagnostics,
    docs: tokio::sync::Mutex<Vec<Doc>>,
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

fn contains(s: Span, p: Pos) -> bool {
    let key = |p: Pos| (p.line, p.col);
    key(s.start) <= key(p) && key(p) <= key(s.end)
}

impl LspServer {
    pub(super) fn new(conn: Connection, root: PathBuf, tree: Tree, diagnostics: Diagnostics) -> Self {
        Self {
            conn,
            root,
            tree,
            diagnostics,
            docs: tokio::sync::Mutex::new(Vec::new()),
        }
    }

    fn conn(&self) -> Result<&Connection, QueryError> {
        Ok(&self.conn)
    }

    fn path_of(&self, file: &RepoPath) -> Result<PathBuf, QueryError> {
        let relative = gix::path::try_from_byte_slice(&file.0).map_err(err)?;
        Ok(self.root.join(relative))
    }

    fn uri_of(&self, file: &RepoPath) -> Result<Uri, QueryError> {
        let url = url::Url::from_file_path(self.path_of(file)?).map_err(|()| err("path is not absolute"))?;
        Uri::from_str(url.as_str()).map_err(err)
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

    /// The repository path of a server URI, or `None` outside the root.
    fn file_of(&self, uri: &Uri) -> Option<RepoPath> {
        let path = url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()?;
        let relative = path.strip_prefix(&self.root).ok()?;
        Some(repo_path(relative))
    }

    fn location(&self, uri: &Uri, range: lsp::Range) -> Location {
        match self.file_of(uri) {
            Some(file) => Location {
                file,
                span: span(range),
                external: false,
            },
            None => {
                let absolute = url::Url::parse(uri.as_str())
                    .ok()
                    .and_then(|u| u.to_file_path().ok())
                    .map_or_else(|| uri.as_str().as_bytes().to_vec(), |p| repo_path(&p).0);
                Location {
                    file: RepoPath(absolute),
                    span: span(range),
                    external: true,
                }
            }
        }
    }

    /// Makes the server see `file`; on the live tree also mirrors every open
    /// document from disk. `true` if `file` was opened or changed. Snapshot
    /// files never change, so they are opened once and left alone.
    async fn sync(&self, file: &RepoPath) -> Result<bool, QueryError> {
        let mut docs = self.docs.lock().await;
        if self.tree == Tree::Immutable {
            if docs.iter().any(|d| &d.file == file) {
                return Ok(false);
            }
            return self.refresh(&mut docs, file).await;
        }
        let open: Vec<_> = docs.iter().map(|d| d.file.clone()).collect();
        for other in open.into_iter().filter(|f| f != file) {
            self.refresh(&mut docs, &other).await?;
        }
        self.refresh(&mut docs, file).await
    }

    async fn refresh(&self, docs: &mut Vec<Doc>, file: &RepoPath) -> Result<bool, QueryError> {
        let path = self.path_of(file)?;
        let text = tokio::fs::read(&path)
            .await
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let index = docs.iter().position(|d| &d.file == file);
        let text = match (text, index) {
            (Ok(text), _) => text,
            (Err(_), Some(index)) => {
                let doc = docs.remove(index);
                self.conn()?
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
            self.conn()?
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
            });
            if docs.len() > MAX_OPEN_DOCS {
                let doc = docs.remove(0);
                self.conn()?
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
        self.conn()?
            .notify::<DidChangeTextDocument>(params)
            .await
            .map_err(err)?;
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
        let at = pos(selection);
        let symbols = self.symbols_cached(cache, &file).await;
        let exact = symbols.iter().find(|s| s.at == at);
        let innermost = || {
            symbols
                .iter()
                .filter(|s| contains(s.span, at))
                .max_by_key(|s| (s.span.start.line, s.span.start.col))
        };
        Some(exact.or_else(innermost).cloned().unwrap_or_else(|| SymbolInfo {
            sym: Sym {
                file,
                name: vec![name.to_owned()],
            },
            span: span(range),
            at,
        }))
    }

    async fn prepare_calls(&self, at: &At) -> Result<Vec<lsp::CallHierarchyItem>, QueryError> {
        self.sync(&at.file).await?;
        let params = lsp::CallHierarchyPrepareParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let items = self
            .conn()?
            .request::<CallHierarchyPrepare>(params)
            .await
            .map_err(err)?;
        Ok(items.unwrap_or_default())
    }
}

fn extension(file: &RepoPath) -> &str {
    std::str::from_utf8(&file.0)
        .ok()
        .and_then(|p| p.rsplit_once('.'))
        .map_or("", |(_, ext)| ext)
}

fn repo_path(relative: &Path) -> RepoPath {
    let bytes = gix::path::into_bstr(relative);
    RepoPath(gix::path::to_unix_separators_on_windows(bytes).into_owned().into())
}

fn walk_symbols(file: &RepoPath, chain: &mut Vec<String>, symbols: &[lsp::DocumentSymbol], out: &mut Vec<SymbolInfo>) {
    for s in symbols {
        chain.push(s.name.clone());
        out.push(SymbolInfo {
            sym: Sym {
                file: file.clone(),
                name: chain.clone(),
            },
            span: span(s.range),
            at: pos(s.selection_range.start),
        });
        walk_symbols(file, chain, s.children.as_deref().unwrap_or_default(), out);
        chain.pop();
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
        self.sync(file).await?;
        let params = lsp::DocumentSymbolParams {
            text_document: self.id(file)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = self
            .conn()?
            .request::<DocumentSymbolRequest>(params)
            .await
            .map_err(err)?;
        let mut out = Vec::new();
        match response {
            None => {}
            Some(lsp::DocumentSymbolResponse::Nested(symbols)) => {
                walk_symbols(file, &mut Vec::new(), &symbols, &mut out);
            }
            Some(lsp::DocumentSymbolResponse::Flat(symbols)) => {
                for s in symbols {
                    let mut name: Vec<_> = s.container_name.into_iter().filter(|c| !c.is_empty()).collect();
                    name.push(s.name);
                    out.push(SymbolInfo {
                        sym: Sym {
                            file: file.clone(),
                            name,
                        },
                        span: span(s.location.range),
                        at: pos(s.location.range.start),
                    });
                }
            }
        }
        Ok(out)
    }

    async fn definition(&self, at: &At) -> Result<Vec<Location>, QueryError> {
        self.sync(&at.file).await?;
        let params = lsp::GotoDefinitionParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = self.conn()?.request::<GotoDefinition>(params).await.map_err(err)?;
        Ok(match response {
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
        self.sync(&at.file).await?;
        let params = lsp::ReferenceParams {
            text_document_position: self.position_params(at)?,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp::ReferenceContext {
                include_declaration: false,
            },
        };
        let response = self.conn()?.request::<References>(params).await.map_err(err)?;
        Ok(response
            .unwrap_or_default()
            .iter()
            .map(|l| self.location(&l.uri, l.range))
            .collect())
    }

    async fn hover(&self, at: &At) -> Result<Option<String>, QueryError> {
        self.sync(&at.file).await?;
        let params = lsp::HoverParams {
            text_document_position_params: self.position_params(at)?,
            work_done_progress_params: Default::default(),
        };
        let response = self.conn()?.request::<HoverRequest>(params).await.map_err(err)?;
        Ok(response.map(|h| flatten_hover(h.contents)))
    }

    async fn incoming_calls(&self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
        let mut cache = HashMap::new();
        let mut out: Vec<SymbolInfo> = Vec::new();
        for item in self.prepare_calls(at).await? {
            let params = lsp::CallHierarchyIncomingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let calls = self
                .conn()?
                .request::<CallHierarchyIncomingCalls>(params)
                .await
                .map_err(err)?;
            for call in calls.unwrap_or_default() {
                let from = call.from;
                let resolved = self
                    .resolve(
                        &mut cache,
                        &from.uri,
                        &from.name,
                        from.range,
                        from.selection_range.start,
                    )
                    .await;
                if let Some(s) = resolved.filter(|s| !out.iter().any(|o| o.sym == s.sym)) {
                    out.push(s);
                }
            }
        }
        Ok(out)
    }

    async fn outgoing_calls(&self, at: &At) -> Result<Vec<SymbolInfo>, QueryError> {
        let mut cache = HashMap::new();
        let mut out: Vec<SymbolInfo> = Vec::new();
        for item in self.prepare_calls(at).await? {
            let params = lsp::CallHierarchyOutgoingCallsParams {
                item,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };
            let calls = self
                .conn()?
                .request::<CallHierarchyOutgoingCalls>(params)
                .await
                .map_err(err)?;
            for call in calls.unwrap_or_default() {
                let to = call.to;
                let resolved = self
                    .resolve(&mut cache, &to.uri, &to.name, to.range, to.selection_range.start)
                    .await;
                if let Some(s) = resolved.filter(|s| !out.iter().any(|o| o.sym == s.sym)) {
                    out.push(s);
                }
            }
        }
        Ok(out)
    }

    async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInfo>, QueryError> {
        let params = lsp::WorkspaceSymbolParams {
            query: query.to_owned(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let response = self
            .conn()?
            .request::<WorkspaceSymbolRequest>(params)
            .await
            .map_err(err)?;
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
        let key = self.uri_of(file)?.as_str().to_owned();
        let generation = |conn: &Connection| conn.with_state(|s| s.diagnostics.get(&key).map_or(0, |d| d.generation));
        let before = generation(self.conn()?);
        let changed = self.sync(file).await?;
        let items = match self.diagnostics {
            Diagnostics::Pull => {
                let params = lsp::DocumentDiagnosticParams {
                    text_document: self.id(file)?,
                    identifier: None,
                    previous_result_id: None,
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                };
                let report = self
                    .conn()?
                    .request::<DocumentDiagnosticRequest>(params)
                    .await
                    .map_err(err)?;
                match report {
                    lsp::DocumentDiagnosticReportResult::Report(lsp::DocumentDiagnosticReport::Full(full)) => {
                        full.full_document_diagnostic_report.items
                    }
                    _ => return Err(err("server answered a diagnostic pull without a full report")),
                }
            }
            Diagnostics::Push => {
                let conn = self.conn()?;
                let fresh = conn
                    .wait_for(DIAGNOSTICS_TIMEOUT, |s| {
                        s.diagnostics
                            .get(&key)
                            .is_some_and(|d| d.generation > before || (!changed && d.generation > 0))
                    })
                    .await;
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
