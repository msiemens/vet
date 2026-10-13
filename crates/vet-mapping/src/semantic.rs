//! Semantic results and language server status to protocol DTOs.

use p::UnavailableKind as Kind;
use vet_app::ports::{FoundServer, ServerOrigin, ServerState, Tsserver};
use vet_app::servers::ServerStatus;
use vet_core::content::{FileDiff, LineRange, RepoPath};
use vet_core::review::Side;
use vet_protocol as p;
use vet_semantic::derived::{
    Bounds, CallSite, Calls, Change, ChangedSymbol, Derived, Fate, FileChanges, Inputs, Located, OutlineSymbol, Reach,
    Reference, RemovalImpact, Usages,
};
use vet_semantic::pool::{Outcome, PoolError};
use vet_semantic::ports::{
    At, BackendId, Diagnostic, LaunchError, Location, Pos, RestoreFailure, Severity, Span, Sym, SymbolInfo, SymbolKind,
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
    }
}

fn symbol(info: &SymbolInfo) -> p::SymbolInfo {
    p::SymbolInfo {
        id: symbol_id(&info.sym),
        modules: sat_u32(info.modules),
        kind: symbol_kind(info.kind),
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

fn reference(r: &Reference) -> p::Reference {
    p::Reference {
        location: location(&r.location),
        within: r.within.as_ref().map(symbol_name),
    }
}

pub fn locations(l: &[Location]) -> Vec<p::SemanticLocation> {
    l.iter().map(location).collect()
}

/// The most rows an answer lists.
pub const MAX_ROWS: usize = 200;
/// The most UTF-16 units of a line a row shows.
const MAX_ROW_TEXT: u32 = 300;

fn utf16_len(s: &str) -> u32 {
    sat_u32(s.encode_utf16().count())
}

/// The line `line` (0-based) of `text`; empty past its end.
pub fn line_at(text: &str, line: u32) -> &str {
    text.lines().nth(line as usize).unwrap_or("")
}

/// Where a place on `side` stands against the diff: `files` are the changed files.
pub fn change_at(files: &[FileDiff], side: Side, file: &RepoPath, line: u32) -> p::LocationChange {
    let on_side = |f: &&FileDiff| match side {
        Side::Base => f.old_path.as_ref() == Some(file),
        Side::Target => f.new_path.as_ref() == Some(file),
    };
    let Some(diff) = files.iter().find(on_side) else {
        return p::LocationChange::Unchanged;
    };
    let at = LineRange {
        start: line,
        end: line + 1,
    };
    let touched = diff.hunks.iter().any(|h| {
        at.intersects(match side {
            Side::Base => h.old,
            Side::Target => h.new,
        })
    });
    if touched {
        p::LocationChange::Hunk
    } else {
        p::LocationChange::File
    }
}

/// A row for the reference `r`, whose line is `line_text` (the whole line, if it could be read).
/// `highlights` gives the highlighting of a range of the line, in UTF-16 units from its start.
pub fn row(
    r: &Reference,
    line_text: &str,
    change: p::LocationChange,
    highlights: impl FnOnce(u32, u32) -> p::Highlights,
) -> p::LocationRow {
    let span = r.location.span;
    let trimmed = line_text.trim_start();
    let lead = utf16_len(&line_text[..line_text.len() - trimmed.len()]);
    let mut shown = 0;
    let text: String = trimmed
        .trim_end()
        .chars()
        .take_while(|c| {
            shown += c.len_utf16() as u32;
            shown <= MAX_ROW_TEXT
        })
        .collect();
    let len = utf16_len(&text);
    let match_start = span.start.col.saturating_sub(lead).min(len);
    let mut match_end = if span.end.line == span.start.line {
        span.end.col.saturating_sub(lead).clamp(match_start, len)
    } else {
        len
    };
    if match_end == match_start {
        // A server may point at a name without giving its extent.
        let word = text
            .encode_utf16()
            .skip(match_start as usize)
            .take_while(|&u| char::from_u32(u.into()).is_some_and(|c| c.is_alphanumeric() || c == '_'))
            .count();
        match_end += sat_u32(word);
    }
    p::LocationRow {
        path: r.location.file.to_string_lossy(),
        line: span.start.line + 1,
        col: span.start.col + 1,
        within: r.within.as_ref().map(symbol_name),
        highlights: highlights(lead, lead + len),
        text,
        match_start,
        match_end,
        change,
        external: r.location.external,
    }
}

/// The rows by path and line, those in the repository before the others, cut to [`MAX_ROWS`];
/// with how many there were. A cut keeps the places in changed hunks, then those in changed
/// files, before the others.
pub fn finish(mut rows: Vec<p::LocationRow>) -> (Vec<p::LocationRow>, u32) {
    let total = sat_u32(rows.len());
    if rows.len() > MAX_ROWS {
        let rank = |r: &p::LocationRow| match r.change {
            p::LocationChange::Hunk => 0,
            p::LocationChange::File => 1,
            p::LocationChange::Unchanged => 2,
        };
        rows.sort_by_key(|r| (rank(r), r.external));
        rows.truncate(MAX_ROWS);
    }
    rows.sort_by(|a, b| (a.external, &a.path, a.line, a.col).cmp(&(b.external, &b.path, b.line, b.col)));
    (rows, total)
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
    let kind = match error {
        SemanticError::NoBackend(_) => Kind::Unsupported,
        SemanticError::Pool(PoolError::Launch(LaunchError::NotInstalled(_))) => Kind::ServerMissing,
        SemanticError::Pool(PoolError::Launch(_) | PoolError::Stopped(_)) => Kind::ServerFailed,
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

/// What the viewer hears when the server `name` died while running.
pub fn server_died(name: &str, outcome: Outcome) -> p::Notification {
    let text = match outcome {
        Outcome::Restarted => format!("{name} stopped unexpectedly and was restarted."),
        Outcome::Stopped => {
            format!("{name} stopped unexpectedly again and stays stopped. Restart it from the Help menu.")
        }
    };
    p::Notification {
        kind: p::NotificationKind::Info,
        text,
        copy_text: None,
        host: None,
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

/// `gap` is why the symbols of the file are unknown, if they are.
pub fn file_symbols(revision: u32, path: &str, d: &Derived<FileChanges>, gap: Option<String>) -> p::FileSymbols {
    p::FileSymbols {
        revision,
        path: path.to_owned(),
        symbols: d.value.symbols.iter().map(changed).collect(),
        context: d.value.context.iter().map(symbol).collect(),
        gap,
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
        gaps: d.value.gaps.clone(),
        inputs: inputs(&d.inputs),
    }
}

pub fn usages(revision: u32, info: &SymbolInfo, d: &Derived<Usages>) -> p::Usages {
    p::Usages {
        revision,
        symbol: symbol(info),
        references: d.value.references.iter().map(reference).collect(),
        gap: d.value.gap.clone(),
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
        base: reference(&c.base),
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
                path: path.iter().map(symbol_name).collect(),
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
            project: f.project.clone(),
            tool: f.tool.clone(),
            message: f.message.clone(),
        })
        .collect()
}

/// `implementations` tells for a language whether its server advertised them.
pub fn servers(all: &[ServerStatus], implementations: impl Fn(&BackendId) -> Option<bool>) -> p::Response {
    p::Response::Servers {
        servers: all
            .iter()
            .map(|s| p::ServerInfo {
                language: s.language.0.clone(),
                title: s.title.clone(),
                server: s.server.clone(),
                extensions: s.extensions.clone(),
                state: server_state(&s.state),
                custom_path: s.custom_path.clone(),
                pinned_version: s.pinned.clone(),
                install_blocker: s.install_blocker.clone(),
                can_install: s.can_install(),
                implementations: implementations(&s.language),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::Hunk;

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

    /// A row of an unhighlighted line.
    fn plain_row(r: &Reference, line: &str) -> p::LocationRow {
        row(r, line, p::LocationChange::Unchanged, |_, _| Vec::new())
    }

    fn diff(path: &str, hunks: &[(u32, u32, u32, u32)]) -> FileDiff {
        FileDiff {
            old_path: Some(RepoPath::from(path)),
            new_path: Some(RepoPath::from(path)),
            old_blob: None,
            new_blob: None,
            hunks: hunks
                .iter()
                .map(|&(os, oe, ns, ne)| Hunk {
                    old: LineRange { start: os, end: oe },
                    new: LineRange { start: ns, end: ne },
                })
                .collect(),
        }
    }

    #[test]
    fn a_row_shows_the_trimmed_line_with_the_match_inside_it() {
        let r = reference("a.cs", (3, 12), (3, 15), false);
        let row = plain_row(&r, "        x = Foo();  ");
        assert_eq!((row.line, row.col), (4, 13));
        assert_eq!(row.text, "x = Foo();");
        assert_eq!(&row.text[row.match_start as usize..row.match_end as usize], "Foo");
    }

    #[test]
    fn a_rows_highlights_are_asked_for_the_trimmed_text_in_units_of_its_line() {
        let r = reference("a.rs", (0, 4), (0, 5), false);
        let asked = std::cell::Cell::new((0, 0));
        // Three units of indentation, then "é😀 f" (five units) and trailing blanks.
        row(&r, "   é😀 f  ", p::LocationChange::Unchanged, |from, to| {
            asked.set((from, to));
            vec![1, 2, 3]
        });
        assert_eq!(asked.get(), (3, 3 + 5));
    }

    #[test]
    fn a_place_without_extent_marks_the_name_it_points_at() {
        let r = reference("Program.cs", (9, 15), (9, 15), false);
        let row = plain_row(&r, "    public int Area() => 4;");
        assert_eq!(&row.text[row.match_start as usize..row.match_end as usize], "Area");
    }

    #[test]
    fn match_columns_count_utf16_units_not_bytes() {
        // "é" is two bytes, the emoji one scalar of two UTF-16 units.
        let r = reference("a.rs", (0, 4), (0, 7), false);
        let row = plain_row(&r, "é😀 f(x)");
        let units: Vec<u16> = row.text.encode_utf16().collect();
        let matched = String::from_utf16(&units[row.match_start as usize..row.match_end as usize]).unwrap();
        assert_eq!(matched, "f(x");
    }

    #[test]
    fn a_match_over_several_lines_ends_with_the_first_line() {
        let r = reference("a.rs", (1, 4), (3, 1), false);
        let row = plain_row(&r, "    fn run() {");
        assert_eq!(
            &row.text[row.match_start as usize..row.match_end as usize],
            "fn run() {"
        );
    }

    #[test]
    fn a_long_line_is_cut_and_a_match_beyond_the_cut_is_clamped_to_its_end() {
        let line = "x".repeat(1000);
        let matched = |from, to| {
            let row = plain_row(&reference("a.rs", (0, from), (0, to), false), &line);
            assert_eq!(row.text.len(), 300);
            (row.match_start, row.match_end)
        };
        assert_eq!(matched(100, 110), (100, 110));
        assert_eq!(matched(295, 310), (295, 300));
        assert_eq!(matched(900, 910), (300, 300));
    }

    #[test]
    fn a_line_that_could_not_be_read_leaves_an_empty_text() {
        let r = reference("/dep/lib.rs", (7, 2), (7, 5), true);
        let row = plain_row(&r, "");
        assert_eq!((row.text.as_str(), row.match_start, row.match_end), ("", 0, 0));
        assert!(row.external);
    }

    #[test]
    fn the_line_of_a_text_is_found_whatever_its_line_ending() {
        assert_eq!(line_at("a\r\nb\nc", 1), "b");
        assert_eq!(line_at("a\r\nb\nc", 0), "a");
        assert_eq!(line_at("a", 5), "");
    }

    #[test]
    fn a_place_is_in_a_hunk_in_a_changed_file_or_unchanged() {
        let files = [diff("a.rs", &[(10, 12, 10, 15)])];
        let a = RepoPath::from("a.rs");
        assert_eq!(change_at(&files, Side::Target, &a, 14), p::LocationChange::Hunk);
        assert_eq!(change_at(&files, Side::Target, &a, 15), p::LocationChange::File);
        assert_eq!(change_at(&files, Side::Base, &a, 12), p::LocationChange::File);
        assert_eq!(change_at(&files, Side::Base, &a, 11), p::LocationChange::Hunk);
        assert_eq!(
            change_at(&files, Side::Target, &RepoPath::from("b.rs"), 14),
            p::LocationChange::Unchanged
        );
    }

    #[test]
    fn a_renamed_file_is_changed_under_the_name_of_the_side() {
        let mut renamed = diff("new.rs", &[(0, 1, 0, 1)]);
        renamed.old_path = Some(RepoPath::from("old.rs"));
        let files = [renamed];
        assert_eq!(
            change_at(&files, Side::Base, &RepoPath::from("old.rs"), 0),
            p::LocationChange::Hunk
        );
        assert_eq!(
            change_at(&files, Side::Target, &RepoPath::from("old.rs"), 0),
            p::LocationChange::Unchanged
        );
    }

    #[test]
    fn rows_are_ordered_by_path_and_line_with_outside_places_last_and_cut_with_the_total_kept() {
        let at =
            |path: &str, line: u32, external: bool| plain_row(&reference(path, (line, 0), (line, 1), external), "");
        let rows = vec![
            at("/dep/a.rs", 1, true),
            at("src/b.rs", 9, false),
            at("src/a.rs", 20, false),
            at("src/a.rs", 3, false),
        ];
        let (sorted, total) = finish(rows);
        let order: Vec<_> = sorted.iter().map(|r| (r.path.as_str(), r.line)).collect();
        assert_eq!(
            order,
            [("src/a.rs", 4), ("src/a.rs", 21), ("src/b.rs", 10), ("/dep/a.rs", 2)]
        );
        assert_eq!(total, 4);

        let many = (0..MAX_ROWS as u32 + 7)
            .map(|n| {
                let mut r = at("src/a.rs", n, false);
                if n >= MAX_ROWS as u32 {
                    r.change = p::LocationChange::Hunk;
                }
                r
            })
            .collect();
        let (cut, total) = finish(many);
        assert_eq!((cut.len(), total as usize), (MAX_ROWS, MAX_ROWS + 7));
        let hunks = cut.iter().filter(|r| r.change == p::LocationChange::Hunk).count();
        assert_eq!(hunks, 7, "a cut keeps the places in changed hunks");
        assert!(cut.windows(2).all(|w| w[0].line < w[1].line));
    }
}
