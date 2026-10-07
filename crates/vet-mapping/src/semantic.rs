//! Semantic results and language server status to protocol DTOs.

use vet_app::ports::{FoundServer, ServerOrigin, ServerState};
use vet_app::servers::ServerStatus;
use vet_core::content::RepoPath;
use vet_protocol as p;
use vet_semantic::derived::{
    Bounds, CallSite, Change, ChangedSymbol, Changes, Derived, Fate, Inputs, Reach, RemovalImpact, SymbolImpact,
};
use vet_semantic::pool::PoolError;
use vet_semantic::ports::{
    At, Diagnostic, LaunchError, Location, Pos, RestoreFailure, Severity, Span, Sym, SymbolInfo,
};
use vet_semantic::query::SemanticError;

use super::sat_u32;

pub fn at(path: &str, line: u32, col: u32) -> At {
    At {
        file: RepoPath::from(path),
        pos: Pos { line, col },
    }
}

pub fn core_sym(id: &p::SymbolId) -> Sym {
    Sym {
        file: RepoPath::from(id.path.as_str()),
        name: id.name.clone(),
    }
}

fn symbol_id(sym: &Sym) -> p::SymbolId {
    p::SymbolId {
        path: sym.file.to_string_lossy(),
        name: sym.name.clone(),
    }
}

fn position(pos: Pos) -> p::Position {
    p::Position {
        line: pos.line,
        col: pos.col,
    }
}

fn span(span: Span) -> p::TextSpan {
    p::TextSpan {
        start: position(span.start),
        end: position(span.end),
    }
}

fn symbol(info: &SymbolInfo) -> p::SymbolInfo {
    p::SymbolInfo {
        id: symbol_id(&info.sym),
        span: span(info.span),
        at: position(info.at),
    }
}

fn location(l: &Location) -> p::SemanticLocation {
    p::SemanticLocation {
        path: l.file.to_string_lossy(),
        span: span(l.span),
        external: l.external,
    }
}

pub fn locations(l: &[Location]) -> Vec<p::SemanticLocation> {
    l.iter().map(location).collect()
}

pub fn diagnostics(d: &[Diagnostic]) -> Vec<p::Diagnostic> {
    d.iter()
        .map(|d| p::Diagnostic {
            span: span(d.span),
            severity: match d.severity {
                Severity::Error => p::Severity::Error,
                Severity::Warning => p::Severity::Warning,
                Severity::Info => p::Severity::Info,
                Severity::Hint => p::Severity::Hint,
            },
            message: d.message.clone(),
            source: d.source.clone(),
            code: d.code.clone(),
        })
        .collect()
}

/// The few words that name a kind of missing answer.
fn label(kind: p::UnavailableKind) -> &'static str {
    use p::UnavailableKind as Kind;
    match kind {
        Kind::Unsupported => "No language support",
        Kind::ServerMissing => "No language server",
        Kind::ServerFailed => "Language server failed",
        Kind::AnalysisLimited => "Analysis limited",
        Kind::SnapshotUnavailable => "Base not available",
        Kind::QueryFailed => "Language server query failed",
    }
}

/// A failed query as the reason to show; `language` is the file's.
pub fn unavailable(error: &SemanticError, language: Option<String>) -> p::Unavailable {
    use p::UnavailableKind as Kind;
    let kind = match error {
        SemanticError::NoBackend(_) => Kind::Unsupported,
        SemanticError::Pool(PoolError::Launch(LaunchError::NotInstalled(_))) => Kind::ServerMissing,
        SemanticError::Pool(PoolError::Launch(_) | PoolError::StartFailed(_)) => Kind::ServerFailed,
        SemanticError::Pool(PoolError::AnalysisLimited(_)) => Kind::AnalysisLimited,
        SemanticError::Pool(PoolError::NotMaterializable(_)) => Kind::SnapshotUnavailable,
        SemanticError::Query(_) => Kind::QueryFailed,
    };
    p::Unavailable {
        language: language.filter(|_| kind != Kind::Unsupported),
        kind,
        label: label(kind).to_owned(),
        message: error.to_string(),
    }
}

/// The answer of a query, or an empty one with the reason.
pub fn answer<T: Default>(result: Result<T, SemanticError>, language: Option<String>) -> (T, Option<p::Unavailable>) {
    match result {
        Ok(value) => (value, None),
        Err(e) => (T::default(), Some(unavailable(&e, language))),
    }
}

fn bounds(b: Bounds) -> p::DerivedBounds {
    p::DerivedBounds {
        max_depth: b.max_depth,
        max_nodes: sat_u32(b.max_nodes),
    }
}

fn inputs(i: &Inputs) -> p::DerivedInputs {
    p::DerivedInputs {
        analysis: i.analysis.to_owned(),
        bounds: i.bounds.map(bounds),
        primitives: i.primitives.clone(),
        hunks: i.hunks.clone(),
        config: i.config.clone(),
    }
}

fn change(c: Change) -> p::SymbolChange {
    match c {
        Change::Added => p::SymbolChange::Added,
        Change::Modified => p::SymbolChange::Modified,
        Change::Removed => p::SymbolChange::Removed,
    }
}

fn changed(c: &ChangedSymbol) -> p::ChangedSymbol {
    p::ChangedSymbol {
        symbol: symbol(&c.symbol),
        change: change(c.change),
    }
}

pub fn changed_symbols(revision: u32, d: &Derived<Changes>) -> p::ChangedSymbols {
    p::ChangedSymbols {
        revision,
        symbols: d.value.symbols.iter().map(changed).collect(),
        gaps: (d.value.gaps.iter())
            .map(|g| p::FileGap {
                path: g.file.to_string_lossy(),
                reason: g.reason.clone(),
            })
            .collect(),
        inputs: inputs(&d.inputs),
    }
}

pub fn impact(revision: u32, d: &Derived<SymbolImpact>) -> p::Impact {
    p::Impact {
        revision,
        symbol: symbol(&d.value.symbol),
        references: locations(&d.value.references),
        callers: d.value.callers.iter().map(symbol).collect(),
        gaps: d.value.gaps.clone(),
        inputs: inputs(&d.inputs),
    }
}

fn fate(f: &Fate) -> p::CallFate {
    match f {
        Fate::Gone => p::CallFate::Gone,
        Fate::Modified => p::CallFate::Modified,
        Fate::LikelyBroken => p::CallFate::LikelyBroken,
        Fate::BindingChanged { now } => p::CallFate::BindingChanged { now: locations(now) },
        Fate::Unknown(reason) => p::CallFate::Unknown { reason: reason.clone() },
    }
}

fn call_site(c: &CallSite) -> p::CallSite {
    p::CallSite {
        base: location(&c.base),
        fate: fate(&c.fate),
    }
}

pub fn removal_impact(revision: u32, d: &Derived<RemovalImpact>) -> p::RemovalImpact {
    p::RemovalImpact {
        revision,
        symbol: symbol(&d.value.symbol),
        call_sites: d.value.call_sites.iter().map(call_site).collect(),
        gap: d.value.gap.clone(),
        inputs: inputs(&d.inputs),
    }
}

pub fn reachability(revision: u32, info: &SymbolInfo, d: &Derived<Reach>) -> p::Reachability {
    p::Reachability {
        revision,
        symbol: symbol(info),
        reach: match &d.value {
            Reach::Reached { path } => p::Reach::Reached {
                path: path.iter().map(symbol_id).collect(),
            },
            Reach::UnreachedWithinBound => p::Reach::UnreachedWithinBound,
            Reach::Unknown { gaps } => p::Reach::Unknown { gaps: gaps.clone() },
        },
        inputs: inputs(&d.inputs),
    }
}

fn origin(o: ServerOrigin) -> p::ServerOrigin {
    match o {
        ServerOrigin::Custom => p::ServerOrigin::Custom,
        ServerOrigin::Project => p::ServerOrigin::Project,
        ServerOrigin::Installed => p::ServerOrigin::Installed,
        ServerOrigin::Path => p::ServerOrigin::Path,
    }
}

fn server_state(state: &ServerState) -> p::ServerState {
    match state {
        ServerState::Found(FoundServer {
            command,
            origin: o,
            version,
            notes,
        }) => p::ServerState::Found {
            path: command.program.clone(),
            origin: origin(*o),
            version: version.clone(),
            notes: notes.clone(),
        },
        ServerState::Missing { reason } => p::ServerState::Missing { reason: reason.clone() },
    }
}

pub fn restore_failures(all: &[RestoreFailure]) -> Vec<p::RestoreFailure> {
    all.iter()
        .map(|f| p::RestoreFailure {
            project: f.project.clone(),
            tool: f.tool.clone(),
            message: f.message.clone(),
        })
        .collect()
}

pub fn servers(all: &[ServerStatus]) -> p::Response {
    p::Response::Servers {
        servers: all
            .iter()
            .map(|s| p::ServerInfo {
                language: s.language.0.clone(),
                title: s.title.clone(),
                server: s.server.clone(),
                state: server_state(&s.state),
                custom_path: s.custom_path.clone(),
                pinned_version: s.pinned.clone(),
                install_blocker: s.install_blocker.clone(),
                can_install: s.can_install(),
            })
            .collect(),
    }
}
