//! Semantic results and language server status to protocol DTOs.

use p::UnavailableKind as Kind;
use vet_app::ports::{FoundServer, ServerOrigin, ServerState, Tsserver};
use vet_app::semantics::{Row, Rows, ServerRuntimes, Standing, WorkspaceSymbols};
use vet_app::servers::ServerStatus;
use vet_core::content::RepoPath;
use vet_core::num::sat_u32;
use vet_protocol as p;
use vet_semantic::derived::{
    Bounds, CallSite, Calls, Change, ChangedSymbol, Derived, Fate, FileChanges, Gap, Inputs, Judged, Located, Origin,
    OutlineSymbol, Reach, Reference, RemovalImpact, Usages,
};
use vet_semantic::pool::{Outcome, Phase, PoolError};
use vet_semantic::ports::{
    At, BackendId, Diagnostic, Feature, Features, LaunchError, Location, Pos, RestoreFailure, RestoreFailureKind,
    Severity, Span, Sym, SymbolInfo, SymbolKind,
};
use vet_semantic::query::SemanticError;

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

fn symbol_kind(kind: SymbolKind) -> p::SymbolKind {
    match kind {
        SymbolKind::Type => p::SymbolKind::Type,
        SymbolKind::Function => p::SymbolKind::Function,
        SymbolKind::Module => p::SymbolKind::Module,
        SymbolKind::Other => p::SymbolKind::Other,
    }
}

fn symbol_name(info: &SymbolInfo) -> p::SymbolName {
    p::SymbolName {
        id: symbol_id(&info.sym),
        modules: sat_u32(info.modules),
        kind: symbol_kind(info.kind),
    }
}

fn symbol(info: &SymbolInfo) -> p::SymbolInfo {
    p::SymbolInfo {
        id: symbol_id(&info.sym),
        modules: sat_u32(info.modules),
        kind: symbol_kind(info.kind),
        span: span(info.span),
        at: position(info.at),
        test: test(&info.sym),
    }
}

/// Vet's conclusion by file name (SPEC I3): the file's path and the containers the symbol is in.
/// The symbol's own name is not a container of itself.
fn test(sym: &Sym) -> bool {
    vet_core::testfile::is_test(&sym.file.to_string_lossy(), containers(sym))
}

/// The containers a symbol is in: its name without its own last part.
fn containers(sym: &Sym) -> &[String] {
    &sym.name[..sym.name.len().saturating_sub(1)]
}

fn location(l: &Location) -> p::SemanticLocation {
    p::SemanticLocation {
        path: l.file.to_string_lossy(),
        span: span(l.span),
        external: l.external,
    }
}

fn reference(r: &Reference) -> p::Reference {
    p::Reference {
        location: location(&r.location),
        within: r.within.as_ref().map(symbol_name),
        test: vet_core::testfile::is_test(
            &r.location.file.to_string_lossy(),
            r.within.as_ref().map_or(&[][..], |w| containers(&w.sym)),
        ),
    }
}

pub fn locations(l: &[Location]) -> Vec<p::SemanticLocation> {
    l.iter().map(location).collect()
}

/// The rows an answer about places came in as, as the protocol delivers them: with the side they
/// answer for, and `unavailable`, why no server answered, where there was one.
pub fn location_rows(rows: &Rows, side: p::Side, unavailable: Option<p::Unavailable>) -> p::LocationRows {
    p::LocationRows {
        revision: rows.revision,
        side,
        rows: rows.rows.iter().map(row).collect(),
        total: rows.total,
        unavailable,
    }
}

/// The row as the use case built it, value for value.
fn row(r: &Row) -> p::LocationRow {
    let change = match r.change {
        Standing::Hunk(_) => p::LocationChange::Hunk,
        Standing::File => p::LocationChange::File,
        Standing::Unchanged => p::LocationChange::Unchanged,
    };
    p::LocationRow {
        path: r.path.to_string_lossy(),
        line: r.line,
        col: r.col,
        within: r.within.as_ref().map(symbol_name),
        highlights: r.highlights.clone(),
        text: r.text.clone(),
        match_start: r.match_start,
        match_end: r.match_end,
        change,
        hunk: match r.change {
            Standing::Hunk(hunk) => Some(hunk),
            _ => None,
        },
        external: r.external,
    }
}

fn diagnostic(d: &Diagnostic, origin: p::DiagnosticOrigin) -> p::Diagnostic {
    p::Diagnostic {
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
        origin,
    }
}

/// Diagnostics not compared with the base yet.
pub fn pending_diagnostics(d: &[Diagnostic]) -> Vec<p::Diagnostic> {
    d.iter().map(|d| diagnostic(d, p::DiagnosticOrigin::Pending)).collect()
}

/// Diagnostics with the origin the comparison found.
pub fn judged_diagnostics(d: &[Judged]) -> Vec<p::Diagnostic> {
    d.iter()
        .map(|j| {
            diagnostic(
                &j.diagnostic,
                match j.origin {
                    Origin::New => p::DiagnosticOrigin::New,
                    Origin::Existing => p::DiagnosticOrigin::Existing,
                    Origin::Unknown => p::DiagnosticOrigin::Unknown,
                },
            )
        })
        .collect()
}

/// The few words that name a kind of missing answer.
fn label(kind: p::UnavailableKind) -> &'static str {
    match kind {
        Kind::Unsupported => "No language support",
        Kind::ServerMissing => "No language server",
        Kind::ServerFailed => "Language server failed",
        Kind::NotOffered => "Not available for this language",
        Kind::AnalysisLimited => "Analysis limited",
        Kind::SnapshotUnavailable => "Base not available",
        Kind::QueryFailed => "Language server query failed",
        Kind::Incomplete => "Language server still working",
    }
}

/// A failed query as the reason to show; `language` is the file's.
pub fn unavailable(error: &SemanticError, language: Option<String>) -> p::Unavailable {
    let kind = match error {
        SemanticError::NoBackend(_) => Kind::Unsupported,
        SemanticError::Pool(PoolError::Launch(LaunchError::NotInstalled(_))) => Kind::ServerMissing,
        SemanticError::Pool(PoolError::Launch(_) | PoolError::Stopped(_)) => Kind::ServerFailed,
        SemanticError::Pool(PoolError::AnalysisLimited(_)) => Kind::AnalysisLimited,
        SemanticError::Pool(PoolError::NotMaterializable(_)) => Kind::SnapshotUnavailable,
        SemanticError::Query(_) => Kind::QueryFailed,
        SemanticError::NotOffered { .. } => Kind::NotOffered,
        SemanticError::Incomplete { .. } => Kind::Incomplete,
    };
    p::Unavailable {
        language: language.filter(|_| kind != Kind::Unsupported),
        kind,
        label: label(kind).to_owned(),
        message: error.to_string(),
    }
}

/// Why a file no language is configured for is not analysed.
pub fn unsupported() -> p::Unavailable {
    p::Unavailable {
        kind: Kind::Unsupported,
        label: label(Kind::Unsupported).to_owned(),
        message: "No language server handles this type of file".to_owned(),
        language: None,
    }
}

/// What the shell is told when the server `name` died while running.
pub fn server_died(name: &str, outcome: Outcome) -> p::Notification {
    p::Notification::ServerDied {
        name: name.to_owned(),
        outcome: match outcome {
            Outcome::Restarted => p::ServerOutcome::Restarted,
            Outcome::Stopped => p::ServerOutcome::Stopped,
        },
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

/// The most symbols of one language an answer lists.
const MAX_WORKSPACE_SYMBOLS: usize = 200;

pub fn workspace_symbols(found: &WorkspaceSymbols) -> p::WorkspaceSymbols {
    p::WorkspaceSymbols {
        revision: found.revision,
        languages: found
            .languages
            .iter()
            .map(|l| p::LanguageSymbols {
                language: l.language.0.clone(),
                symbols: l
                    .result
                    .as_ref()
                    .map(|symbols| symbols.iter().take(MAX_WORKSPACE_SYMBOLS).map(symbol).collect())
                    .unwrap_or_default(),
                failure: l
                    .result
                    .as_ref()
                    .err()
                    .map(|e| unavailable(e, Some(l.language.0.clone()))),
            })
            .collect(),
    }
}

/// `gap` is why the symbols of the file are unknown, if they are.
pub fn file_symbols(
    revision: u32,
    path: &str,
    d: &Derived<FileChanges>,
    gap: Option<&SemanticError>,
) -> p::FileSymbols {
    p::FileSymbols {
        revision,
        path: path.to_owned(),
        symbols: d.value.symbols.iter().map(changed).collect(),
        context: d.value.context.iter().map(symbol).collect(),
        gap: gap.map(|e| unavailable(e, None)),
        inputs: inputs(&d.inputs),
    }
}

pub fn file_outline(
    revision: u32,
    path: &str,
    symbols: &[OutlineSymbol],
    unavailable: Option<p::Unavailable>,
) -> p::FileOutline {
    p::FileOutline {
        revision,
        path: path.to_owned(),
        symbols: (symbols.iter())
            .map(|o| p::OutlineSymbol {
                symbol: symbol(&o.symbol),
                change: o.change.map(change),
            })
            .collect(),
        unavailable,
    }
}

pub fn symbol_at(revision: u32, found: Option<&Located>, unavailable: Option<p::Unavailable>) -> p::SymbolAt {
    p::SymbolAt {
        revision,
        symbol: found.map(|l| symbol(&l.symbol)),
        change: found.and_then(|l| l.change).map(change),
        unavailable,
    }
}

pub fn calls(revision: u32, info: &SymbolInfo, d: &Derived<Calls>) -> p::Calls {
    p::Calls {
        revision,
        symbol: symbol(info),
        callers: (d.value.callers.iter())
            .map(|c| p::ChangedCaller {
                symbol: symbol(&c.symbol),
                change: change(c.change),
                depth: c.depth,
            })
            .collect(),
        callees: d.value.callees.iter().map(changed).collect(),
        affected: d.value.affected.iter().map(symbol).collect(),
        gaps: d.value.gaps.iter().map(gap).collect(),
        inputs: inputs(&d.inputs),
    }
}

pub fn usages(revision: u32, info: &SymbolInfo, d: &Derived<Usages>) -> p::Usages {
    p::Usages {
        revision,
        symbol: symbol(info),
        references: d.value.references.iter().map(reference).collect(),
        gap: d.value.gap.as_ref().map(|e| unavailable(e, None)),
        inputs: inputs(&d.inputs),
    }
}

fn fate(f: &Fate) -> p::CallFate {
    match f {
        Fate::Gone => p::CallFate::Gone,
        Fate::Modified => p::CallFate::Modified,
        Fate::LikelyBroken => p::CallFate::LikelyBroken,
        Fate::BindingChanged { now } => p::CallFate::BindingChanged { now: locations(now) },
        Fate::Unknown(error) => p::CallFate::Unknown {
            why: unavailable(error, None),
        },
    }
}

fn gap(g: &Gap) -> p::Gap {
    match g {
        Gap::Failed { what, error } => p::Gap::Failed {
            what: what.clone(),
            why: unavailable(error, None),
        },
        Gap::TooManyCallers { max } => p::Gap::TooManyCallers { max: sat_u32(*max) },
        Gap::TooDeep { max_depth } => p::Gap::TooDeep { max_depth: *max_depth },
    }
}

fn call_site(c: &CallSite) -> p::CallSite {
    p::CallSite {
        base: reference(&c.base),
        fate: fate(&c.fate),
    }
}

pub fn removal_impact(revision: u32, d: &Derived<RemovalImpact>) -> p::RemovalImpact {
    p::RemovalImpact {
        revision,
        symbol: symbol(&d.value.symbol),
        call_sites: d.value.call_sites.iter().map(call_site).collect(),
        gap: d.value.gap.as_ref().map(|e| unavailable(e, None)),
        inputs: inputs(&d.inputs),
    }
}

pub fn reachability(revision: u32, info: &SymbolInfo, d: &Derived<Reach>) -> p::Reachability {
    p::Reachability {
        revision,
        symbol: symbol(info),
        reach: match &d.value {
            Reach::Reached { path } => p::Reach::Reached {
                path: path.iter().map(symbol_name).collect(),
            },
            Reach::UnreachedWithinBound => p::Reach::UnreachedWithinBound,
            Reach::Unknown { gaps } => p::Reach::Unknown {
                gaps: gaps.iter().map(gap).collect(),
            },
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
            tsserver,
        }) => p::ServerState::Found {
            path: command.program.clone(),
            origin: origin(*o),
            version: version.clone(),
            tsserver: tsserver.as_ref().map(|t| match t {
                Tsserver::Using { whose, path } => p::Tsserver::Using {
                    whose: whose.clone(),
                    path: path.to_string_lossy().into_owned(),
                },
                Tsserver::Missing { reason } => p::Tsserver::Missing { reason: reason.clone() },
            }),
        },
        ServerState::Missing { reason } => p::ServerState::Missing { reason: reason.clone() },
    }
}

pub fn restore_failures(all: &[RestoreFailure]) -> Vec<p::RestoreFailure> {
    all.iter()
        .map(|f| p::RestoreFailure {
            kind: match f.kind {
                RestoreFailureKind::Failed => p::RestoreFailureKind::Failed,
                RestoreFailureKind::StoodIn => p::RestoreFailureKind::StoodIn,
            },
            project: f.project.clone(),
            tool: f.tool.clone(),
            message: f.message.clone(),
        })
        .collect()
}

fn server_features(f: Features) -> p::ServerFeatures {
    p::ServerFeatures {
        document_symbols: f.offers(Feature::DocumentSymbols),
        definition: f.offers(Feature::Definition),
        references: f.offers(Feature::References),
        implementations: f.offers(Feature::Implementations),
        hover: f.offers(Feature::Hover),
        call_hierarchy: f.offers(Feature::CallHierarchy),
        workspace_symbols: f.offers(Feature::WorkspaceSymbols),
    }
}

/// What the servers of the changeset on view are doing.
pub fn runtimes(now: &ServerRuntimes) -> p::ServerRuntimes {
    p::ServerRuntimes {
        serial: now.serial,
        runtimes: now
            .runtimes
            .iter()
            .map(|r| p::ServerRuntime {
                language: r.language.0.clone(),
                side: super::side(r.side),
                phase: match &r.phase {
                    Phase::Starting => p::ServerPhase::Starting,
                    Phase::Indexing { done_of_total, detail } => p::ServerPhase::Indexing {
                        done: done_of_total.map(|(done, _)| done),
                        total: done_of_total.map(|(_, total)| total),
                        detail: detail.clone(),
                    },
                    Phase::Ready => p::ServerPhase::Ready,
                    Phase::Failed(e) => p::ServerPhase::Failed {
                        why: unavailable(&SemanticError::Pool(e.clone()), Some(r.language.0.clone())),
                    },
                },
            })
            .collect(),
    }
}

/// `features` tells for a language what its server advertised.
pub fn servers(
    all: &[ServerStatus],
    features: impl Fn(&BackendId) -> Option<Features>,
    now: &ServerRuntimes,
) -> p::Response {
    p::Response::Servers {
        runtimes: runtimes(now),
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
                features: features(&s.language).map(server_features),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use vet_app::semantics::{LanguageSymbols, Row, Standing};
    use vet_core::content::RepoPath;

    use super::*;

    fn reference(file: &str, from: (u32, u32), to: (u32, u32), external: bool) -> Reference {
        Reference {
            location: Location {
                file: RepoPath::from(file),
                span: Span {
                    start: Pos {
                        line: from.0,
                        col: from.1,
                    },
                    end: Pos { line: to.0, col: to.1 },
                },
                external,
            },
            within: None,
        }
    }

    fn within(kind: SymbolKind) -> SymbolInfo {
        SymbolInfo {
            sym: Sym {
                file: RepoPath::from("db.py"),
                name: vec!["TinyDB".into(), "table".into()],
            },
            span: Span {
                start: Pos { line: 1, col: 0 },
                end: Pos { line: 9, col: 0 },
            },
            at: Pos { line: 1, col: 8 },
            detail: None,
            kind,
            modules: 0,
        }
    }

    #[test]
    fn a_language_that_could_not_answer_says_why_in_the_kind_the_viewer_acts_on() {
        let found = WorkspaceSymbols {
            revision: 3,
            languages: vec![
                LanguageSymbols {
                    language: BackendId("rust".into()),
                    result: Ok(Vec::new()),
                },
                LanguageSymbols {
                    language: BackendId("go".into()),
                    result: Err(SemanticError::Pool(PoolError::AnalysisLimited(BackendId("go".into())))),
                },
            ],
        };
        let mapped = workspace_symbols(&found);
        let failures: Vec<_> = mapped
            .languages
            .iter()
            .map(|l| l.failure.as_ref().map(|f| (f.kind, f.language.as_deref())))
            .collect();
        assert_eq!(
            failures,
            [None, Some((p::UnavailableKind::AnalysisLimited, Some("go")))]
        );
    }

    #[test]
    fn a_place_lists_the_kind_of_the_symbol_it_is_in() {
        let kinds = |kind| {
            let r = Reference {
                within: Some(within(kind)),
                ..reference("db.py", (3, 8), (3, 13), false)
            };
            super::reference(&r).within.map(|w| w.kind)
        };
        assert_eq!(kinds(SymbolKind::Function), Some(p::SymbolKind::Function));
        assert_eq!(kinds(SymbolKind::Type), Some(p::SymbolKind::Type));
    }

    #[test]
    fn a_symbol_and_a_place_carry_the_test_conclusion_by_file_name() {
        // A test by its path.
        let by_path = SymbolInfo {
            sym: Sym {
                file: RepoPath::from("tests/a.rs"),
                name: vec!["t".into()],
            },
            ..within(SymbolKind::Function)
        };
        assert!(super::symbol(&by_path).test);
        // A test by a `tests` container.
        let by_container = SymbolInfo {
            sym: Sym {
                file: RepoPath::from("src/a.rs"),
                name: vec!["tests".into(), "t".into()],
            },
            ..within(SymbolKind::Function)
        };
        assert!(super::symbol(&by_container).test);
        // Production code is not.
        assert!(!super::symbol(&within(SymbolKind::Function)).test);
        // A place too, judged by the containers of the symbol it is in.
        let in_test = Reference {
            within: Some(by_container),
            ..reference("src/a.rs", (3, 8), (3, 13), false)
        };
        assert!(super::reference(&in_test).test);
        let in_production = Reference {
            within: Some(within(SymbolKind::Function)),
            ..reference("src/a.rs", (3, 8), (3, 13), false)
        };
        assert!(!super::reference(&in_production).test);
    }

    fn a_row(change: Standing, external: bool) -> Row {
        Row {
            path: RepoPath::from(if external { "/dep/lib.rs" } else { "src/db.rs" }),
            line: 4,
            col: 13,
            within: None,
            highlights: Vec::new(),
            text: String::new(),
            match_start: 0,
            match_end: 0,
            change,
            external,
        }
    }

    #[test]
    fn a_row_is_delivered_value_for_value() {
        let r = Row {
            within: Some(within(SymbolKind::Function)),
            highlights: vec![1, 2, 3],
            text: "x = Foo()".into(),
            match_start: 4,
            match_end: 7,
            ..a_row(Standing::Hunk(2), false)
        };
        let mapped = super::row(&r);
        assert_eq!(mapped.path, "src/db.rs");
        assert_eq!((mapped.line, mapped.col), (4, 13));
        assert_eq!(
            mapped.within,
            Some(p::SymbolName {
                id: p::SymbolId {
                    path: "db.py".into(),
                    name: vec!["TinyDB".into(), "table".into()],
                },
                modules: 0,
                kind: p::SymbolKind::Function,
            })
        );
        assert_eq!(mapped.highlights, [1, 2, 3]);
        assert_eq!(mapped.text, "x = Foo()");
        assert_eq!((mapped.match_start, mapped.match_end), (4, 7));
        assert!(!mapped.external);
    }

    #[test]
    fn a_row_delivers_where_the_place_stands_and_that_it_is_not_of_the_repository() {
        let in_hunk = super::row(&a_row(Standing::Hunk(3), false));
        assert_eq!((in_hunk.change, in_hunk.hunk), (p::LocationChange::Hunk, Some(3)));
        let in_file = super::row(&a_row(Standing::File, false));
        assert_eq!((in_file.change, in_file.hunk), (p::LocationChange::File, None));
        let unchanged = super::row(&a_row(Standing::Unchanged, true));
        assert_eq!(unchanged.change, p::LocationChange::Unchanged);
        assert_eq!((unchanged.hunk, unchanged.external), (None, true));
    }

    #[test]
    fn a_server_that_died_reaches_the_shell_as_its_name_and_what_became_of_it() {
        assert_eq!(
            server_died("rust-analyzer", Outcome::Restarted),
            p::Notification::ServerDied {
                name: "rust-analyzer".to_owned(),
                outcome: p::ServerOutcome::Restarted,
            }
        );
        assert_eq!(
            server_died("rust-analyzer", Outcome::Stopped),
            p::Notification::ServerDied {
                name: "rust-analyzer".to_owned(),
                outcome: p::ServerOutcome::Stopped,
            }
        );
    }
}
