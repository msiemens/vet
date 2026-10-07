//! Where the semantic layer reports how long its phases took, for a host that
//! asks (`VET_TIMING`). Nothing is measured for a host that does not install a sink.

use std::sync::OnceLock;
use std::time::Instant;

use vet_core::content::RepoPath;
use vet_core::review::Side;

use crate::ports::{At, Diagnostic, Location, SymbolInfo};
use crate::query::{Primitives, SemanticError};

/// Receives a finished phase: what it was and when it began.
pub type Sink = fn(String, Instant);

static SINK: OnceLock<Sink> = OnceLock::new();

/// The first sink installed stays.
pub fn install(sink: Sink) {
    let _ = SINK.set(sink);
}

/// Reports `what` as having taken the time since `started`. `what` is built
/// only if there is a sink.
pub fn log(what: impl FnOnce() -> String, started: Instant) {
    if let Some(sink) = SINK.get() {
        sink(what(), started);
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Base => "base",
        Side::Target => "target",
    }
}

fn place(at: &At) -> String {
    format!("{}:{}:{}", at.file.to_string_lossy(), at.pos.line, at.pos.col)
}

/// The primitives of a derived analysis, each reported with the analysis it
/// served, the side it asked and when it began, so the order and overlap of
/// an analysis's queries can be read off the log.
pub struct Traced<'a, P> {
    inner: &'a P,
    analysis: String,
}

impl<'a, P> Traced<'a, P> {
    pub fn new(inner: &'a P, analysis: impl Into<String>) -> Self {
        Self {
            inner,
            analysis: analysis.into(),
        }
    }

    fn report(&self, op: &str, side: Option<Side>, what: impl FnOnce() -> String, started: Instant) {
        log(
            || match side {
                Some(side) => format!("{}: {op}({}:{})", self.analysis, side_name(side), what()),
                None => format!("{}: {op}({})", self.analysis, what()),
            },
            started,
        );
    }
}

impl<P: Primitives + Sync> Primitives for Traced<'_, P> {
    async fn document_symbols(&self, side: Side, file: &RepoPath) -> Result<Vec<SymbolInfo>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.document_symbols(side, file).await;
        self.report("document_symbols", Some(side), || file.to_string_lossy(), started);
        answer
    }

    async fn text_symbols(&self, file: &RepoPath, text: &str) -> Result<Vec<SymbolInfo>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.text_symbols(file, text).await;
        self.report("text_symbols", None, || file.to_string_lossy(), started);
        answer
    }

    async fn definition(&self, side: Side, at: &At) -> Result<Vec<Location>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.definition(side, at).await;
        self.report("definition", Some(side), || place(at), started);
        answer
    }

    async fn references(&self, side: Side, at: &At) -> Result<Vec<Location>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.references(side, at).await;
        self.report("references", Some(side), || place(at), started);
        answer
    }

    async fn hover(&self, side: Side, at: &At) -> Result<Option<String>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.hover(side, at).await;
        self.report("hover", Some(side), || place(at), started);
        answer
    }

    async fn incoming_calls(&self, side: Side, at: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.incoming_calls(side, at).await;
        self.report("incoming_calls", Some(side), || place(at), started);
        answer
    }

    async fn outgoing_calls(&self, side: Side, at: &At) -> Result<Vec<SymbolInfo>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.outgoing_calls(side, at).await;
        self.report("outgoing_calls", Some(side), || place(at), started);
        answer
    }

    async fn diagnostics(&self, side: Side, file: &RepoPath) -> Result<Vec<Diagnostic>, SemanticError> {
        let started = Instant::now();
        let answer = self.inner.diagnostics(side, file).await;
        self.report("diagnostics", Some(side), || file.to_string_lossy(), started);
        answer
    }
}
