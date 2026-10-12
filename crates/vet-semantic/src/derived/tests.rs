//! Derived analyses over a fake `Primitives`.

use std::collections::HashMap;

use vet_core::content::{FileDiff, Hunk, LineRange, RepoPath};
use vet_core::review::Side;

use super::*;
use crate::ports::{Diagnostic, Location, Pos, Span, Sym, SymbolInfo};
use crate::query::{Asked, Primitives, SemanticError};
use crate::testutil::{answered, block_on};

fn file(p: &str) -> RepoPath {
    RepoPath(p.as_bytes().to_vec())
}

fn lines(start: u32, end: u32) -> LineRange {
    LineRange { start, end }
}

fn span(start: u32, end: u32) -> Span {
    Span {
        start: Pos { line: start, col: 0 },
        end: Pos { line: end, col: 0 },
    }
}

/// A symbol `name` spanning lines `start..end`, named on its first line.
fn sym(path: &str, name: &str, start: u32, end: u32) -> SymbolInfo {
    SymbolInfo {
        sym: Sym {
            file: file(path),
            name: name.split("::").map(String::from).collect(),
        },
        span: span(start, end),
        at: Pos { line: start, col: 3 },
        detail: None,
        container: true,
    }
}

/// Like [`sym`], but a parameter, variable or field: nothing is written inside it.
fn variable(path: &str, name: &str, start: u32, end: u32) -> SymbolInfo {
    SymbolInfo {
        container: false,
        ..sym(path, name, start, end)
    }
}

fn at(path: &str, line: u32, col: u32) -> At {
    At {
        file: file(path),
        pos: Pos { line, col },
    }
}

fn site(path: &str, line: u32) -> Location {
    Location {
        file: file(path),
        span: Span {
            start: Pos { line, col: 4 },
            end: Pos { line, col: 9 },
        },
        external: false,
    }
}

fn hunk(old: (u32, u32), new: (u32, u32)) -> Hunk {
    Hunk {
        old: lines(old.0, old.1),
        new: lines(new.0, new.1),
    }
}

fn diff(path: &str, hunks: Vec<Hunk>) -> FileDiff {
    FileDiff {
        old_path: Some(file(path)),
        new_path: Some(file(path)),
        old_blob: None,
        new_blob: None,
        hunks,
    }
}

#[derive(Default)]
struct Fake {
    symbols: HashMap<(Side, RepoPath), Vec<SymbolInfo>>,
    references: HashMap<(Side, At), Vec<Location>>,
    callers: HashMap<(Side, At), Vec<SymbolInfo>>,
    definitions: HashMap<(Side, At), Vec<Location>>,
    failing_callers: Vec<At>,
    /// Files whose document symbols the server fails to answer.
    failing_symbols: Vec<RepoPath>,
    /// What the target side's server answers for a text under a file's name.
    texts: HashMap<(RepoPath, String), Vec<SymbolInfo>>,
    /// Every answer is one reused from an earlier question.
    cached: bool,
}

/// No content is available.
struct NoTexts;

impl Texts for NoTexts {
    async fn base_text(&self, _: &FileDiff) -> Option<String> {
        None
    }
    async fn target_text(&self, _: &FileDiff) -> Option<String> {
        None
    }
}

/// The content of every file is `base of <path>` and `target of <path>`.
struct FakeTexts;

impl Texts for FakeTexts {
    async fn base_text(&self, file: &FileDiff) -> Option<String> {
        Some(format!("base of {}", file.path().to_string_lossy()))
    }
    async fn target_text(&self, file: &FileDiff) -> Option<String> {
        Some(format!("target of {}", file.path().to_string_lossy()))
    }
}

impl Primitives for Fake {
    async fn text_symbols(&self, f: &RepoPath, text: &str) -> Asked<Vec<SymbolInfo>> {
        let found = (self.texts.get(&(f.clone(), text.to_owned())).cloned())
            .ok_or_else(|| SemanticError::NoBackend(f.clone()))?;
        self.answer(found)
    }
    async fn document_symbols(&self, side: Side, f: &RepoPath) -> Asked<Vec<SymbolInfo>> {
        if self.failing_symbols.contains(f) {
            return Err(SemanticError::Query(crate::ports::QueryError("server crashed".into())));
        }
        let found =
            (self.symbols.get(&(side, f.clone())).cloned()).ok_or_else(|| SemanticError::NoBackend(f.clone()))?;
        self.answer(found)
    }
    async fn definition(&self, side: Side, a: &At) -> Asked<Vec<Location>> {
        self.answer(self.definitions.get(&(side, a.clone())).cloned().unwrap_or_default())
    }
    async fn references(&self, side: Side, a: &At) -> Asked<Vec<Location>> {
        self.answer(self.references.get(&(side, a.clone())).cloned().unwrap_or_default())
    }
    async fn hover(&self, _: Side, _: &At) -> Asked<Option<String>> {
        self.answer(None)
    }
    async fn incoming_calls(&self, side: Side, a: &At) -> Asked<Vec<SymbolInfo>> {
        if self.failing_callers.contains(a) {
            return Err(SemanticError::Query(crate::ports::QueryError(
                "no call hierarchy".into(),
            )));
        }
        self.answer(self.callers.get(&(side, a.clone())).cloned().unwrap_or_default())
    }
    async fn outgoing_calls(&self, _: Side, _: &At) -> Asked<Vec<SymbolInfo>> {
        self.answer(vec![])
    }
    async fn diagnostics(&self, _: Side, _: &RepoPath) -> Asked<Vec<Diagnostic>> {
        self.answer(vec![])
    }
}

impl Fake {
    fn answer<T>(&self, value: T) -> Asked<T> {
        let mut answered = answered(value)?;
        answered.provenance.from_cache = self.cached;
        Ok(answered)
    }
}

// base:   a.rs  impl S { fn keep 1..4, fn drop 5..8 } fn other 10..14
// target: a.rs  impl S { fn keep 1..5 (edited) } fn other 7..11, fn fresh 12..15
fn changed_fixture() -> (Fake, Vec<FileDiff>) {
    let mut fake = Fake::default();
    fake.symbols.insert(
        (Side::Base, file("a.rs")),
        vec![
            sym("a.rs", "S", 0, 9),
            sym("a.rs", "S::keep", 1, 4),
            sym("a.rs", "S::drop", 5, 8),
            sym("a.rs", "other", 10, 14),
        ],
    );
    fake.symbols.insert(
        (Side::Target, file("a.rs")),
        vec![
            sym("a.rs", "S", 0, 6),
            sym("a.rs", "S::keep", 1, 5),
            sym("a.rs", "other", 7, 11),
            sym("a.rs", "fresh", 12, 15),
        ],
    );
    // keep gains a line, drop is deleted, fresh is added.
    let hunks = vec![hunk((3, 4), (3, 5)), hunk((5, 9), (5, 5)), hunk((14, 14), (12, 15))];
    (fake, vec![diff("a.rs", hunks)])
}

#[test]
fn changed_symbols_are_the_innermost_symbols_touched_by_hunks() {
    let (fake, files) = changed_fixture();
    let result = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}));
    let summary: Vec<_> = result
        .value
        .symbols
        .iter()
        .map(|c| (c.symbol.sym.name.join("::"), c.change))
        .collect();
    assert_eq!(
        summary,
        [
            ("S::keep".to_string(), Change::Modified),
            // The deletion point lies inside `S` but outside its remaining children.
            ("S".to_string(), Change::Modified),
            ("fresh".to_string(), Change::Added),
            ("S::drop".to_string(), Change::Removed),
        ]
    );
    assert_eq!(
        result.inputs.primitives,
        [
            "document_symbols(base:a.rs) [fake]",
            "document_symbols(target:a.rs) [fake]"
        ]
    );
    assert_eq!(result.inputs.hunks.len(), 1);
}

#[test]
fn base_symbols_come_from_the_base_text_not_from_a_base_server() {
    let (mut fake, files) = changed_fixture();
    let base = fake.symbols.remove(&(Side::Base, file("a.rs"))).unwrap();
    fake.texts.insert((file("a.rs"), "base of a.rs".into()), base);
    let with_texts = block_on(changed_symbols(&fake, &FakeTexts, &files, &|_, _| {})).value;
    let (expected, _) = changed_fixture();
    let with_server = block_on(changed_symbols(&expected, &NoTexts, &files, &|_, _| {})).value;
    assert_eq!(with_texts, with_server);
    assert!(with_texts.gaps.is_empty());
}

#[test]
fn target_symbols_come_from_the_target_text_not_from_the_file_as_the_server_has_it() {
    let (mut fake, files) = changed_fixture();
    // The server's file differs from the revision's text: only the text's symbols count.
    let target = fake
        .symbols
        .insert((Side::Target, file("a.rs")), vec![sym("a.rs", "moved_on", 0, 20)]);
    fake.texts
        .insert((file("a.rs"), "target of a.rs".into()), target.unwrap());
    let with_texts = block_on(changed_symbols(&fake, &FakeTexts, &files, &|_, _| {})).value;
    let (expected, _) = changed_fixture();
    let with_server = block_on(changed_symbols(&expected, &NoTexts, &files, &|_, _| {})).value;
    assert_eq!(with_texts, with_server);
}

#[test]
fn base_symbols_fall_back_to_the_base_server_when_the_text_cannot_be_placed() {
    let (fake, files) = changed_fixture();
    // The fake knows no symbols for the text, as a server that refuses it.
    let changes = block_on(changed_symbols(&fake, &FakeTexts, &files, &|_, _| {})).value;
    assert_eq!(changes.symbols.len(), 4);
    assert!(changes.gaps.is_empty());
}

#[test]
fn added_and_deleted_files_report_every_symbol() {
    let mut fake = Fake::default();
    fake.symbols.insert(
        (Side::Target, file("new.rs")),
        vec![sym("new.rs", "T", 0, 5), sym("new.rs", "T::f", 1, 3)],
    );
    fake.symbols
        .insert((Side::Base, file("old.rs")), vec![sym("old.rs", "U", 0, 3)]);
    let files = vec![
        FileDiff {
            old_path: None,
            new_path: Some(file("new.rs")),
            old_blob: None,
            new_blob: None,
            hunks: vec![hunk((0, 0), (0, 5))],
        },
        FileDiff {
            old_path: Some(file("old.rs")),
            new_path: None,
            old_blob: None,
            new_blob: None,
            hunks: vec![hunk((0, 3), (0, 0))],
        },
    ];
    let changes = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}))
        .value
        .symbols;
    let kinds: Vec<_> = changes.iter().map(|c| c.change).collect();
    assert_eq!(kinds, [Change::Added, Change::Added, Change::Removed]);
}

#[test]
fn changed_symbols_keep_the_order_of_the_files_however_many_are_asked_at_once() {
    let mut fake = Fake::default();
    let mut files = Vec::new();
    for n in 0..10 {
        let path = format!("f{n}.rs");
        fake.symbols
            .insert((Side::Base, file(&path)), vec![sym(&path, "s", 0, 3)]);
        fake.symbols
            .insert((Side::Target, file(&path)), vec![sym(&path, "s", 0, 3)]);
        files.push(diff(&path, vec![hunk((1, 2), (1, 2))]));
    }
    let result = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}));
    let order: Vec<_> = result.value.symbols.iter().map(|c| c.symbol.sym.file.clone()).collect();
    let want: Vec<_> = (0..10).map(|n| file(&format!("f{n}.rs"))).collect();
    assert_eq!(order, want);
    assert_eq!(result.inputs.primitives.len(), 20);
    assert_eq!(result.inputs.primitives[2], "document_symbols(base:f1.rs) [fake]");
}

#[test]
fn each_analysed_file_is_reported_as_done_of_the_files_with_hunks() {
    let mut fake = Fake::default();
    let mut files = vec![diff("untouched.rs", vec![])];
    for n in 0..6 {
        let path = format!("f{n}.rs");
        fake.symbols
            .insert((Side::Base, file(&path)), vec![sym(&path, "s", 0, 3)]);
        fake.symbols
            .insert((Side::Target, file(&path)), vec![sym(&path, "s", 0, 3)]);
        files.push(diff(&path, vec![hunk((1, 2), (1, 2))]));
    }
    let reported = std::sync::Mutex::new(Vec::new());
    block_on(changed_symbols(&fake, &NoTexts, &files, &|done, total| {
        reported.lock().unwrap().push((done, total));
    }));
    let mut reported = reported.into_inner().unwrap();
    reported.sort();
    assert_eq!(reported, (1..=6).map(|done| (done, 6)).collect::<Vec<_>>());
}

#[test]
fn files_without_a_backend_are_no_gap_but_a_failed_analysis_is() {
    let mut fake = Fake::default();
    fake.failing_symbols.push(file("broken.rs"));
    let files = vec![
        diff("README.md", vec![hunk((0, 1), (0, 2))]),
        diff("broken.rs", vec![hunk((0, 1), (0, 2))]),
    ];
    let changes = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {})).value;
    assert!(changes.symbols.is_empty());
    let gaps: Vec<_> = changes.gaps.iter().map(|g| (&g.file, g.reason.as_str())).collect();
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!(gaps[0].0, &file("broken.rs"));
    assert!(gaps[0].1.contains("server crashed"), "{gaps:?}");
}

#[test]
fn impact_lists_references_and_callers_of_added_and_modified_symbols_only() {
    let (mut fake, files) = changed_fixture();
    let changes = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}))
        .value
        .symbols;
    let keep = changes[0].symbol.location();
    fake.references
        .insert((Side::Target, keep.clone()), vec![site("b.rs", 7)]);
    fake.callers
        .insert((Side::Target, keep), vec![sym("b.rs", "caller", 5, 9)]);
    fake.failing_callers.push(changes[1].symbol.location());
    fake.symbols
        .insert((Side::Target, file("b.rs")), vec![sym("b.rs", "caller", 5, 9)]);

    let result = block_on(impact(&fake, &changes));
    assert_eq!(result.value.len(), 3);
    assert_eq!(result.value[0].references.len(), 1);
    assert_eq!(result.value[0].references[0].location, site("b.rs", 7));
    assert_eq!(result.value[0].callers.len(), 1);
    assert!(result.value[1].gaps.iter().any(|g| g.contains("no call hierarchy")));
    assert_eq!(result.inputs.primitives.len(), 7);
}

#[test]
fn a_reference_is_in_the_innermost_container_not_in_a_parameter_or_variable() {
    let (mut fake, files) = changed_fixture();
    let changes = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}))
        .value
        .symbols;
    fake.symbols.insert(
        (Side::Target, file("b.rs")),
        vec![
            sym("b.rs", "Billing", 0, 20),
            sym("b.rs", "Billing::run", 4, 12),
            variable("b.rs", "Billing::run::arg", 4, 5),
            variable("b.rs", "Billing::run::total", 6, 7),
            sym("b.rs", "Billing::run::inner", 8, 10),
            variable("b.rs", "LIMIT", 22, 23),
        ],
    );
    fake.references.insert(
        (Side::Target, changes[0].symbol.location()),
        vec![
            // In the parameter's type, in a local's initializer, in the nested function, in
            // the class body, at the top level and in a variable at the top level.
            site("b.rs", 4),
            site("b.rs", 6),
            site("b.rs", 9),
            site("b.rs", 15),
            site("b.rs", 21),
            site("b.rs", 22),
        ],
    );
    let result = block_on(impact(&fake, &changes[..1]));
    let within: Vec<_> = result.value[0]
        .references
        .iter()
        .map(|r| r.within.as_ref().map(|s| s.name.join("::")))
        .collect();
    assert_eq!(
        within,
        [
            Some("Billing::run".to_owned()),
            Some("Billing::run".to_owned()),
            Some("Billing::run::inner".to_owned()),
            Some("Billing".to_owned()),
            None,
            None,
        ]
    );
    // The outline is asked once for the file, however many references are in it.
    let outlines = result
        .inputs
        .primitives
        .iter()
        .filter(|p| p.starts_with("document_symbols("))
        .count();
    assert_eq!(outlines, 1);
}

#[test]
fn a_reference_whose_file_has_no_outline_or_is_outside_the_repository_is_in_nothing() {
    let (mut fake, files) = changed_fixture();
    let changes = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}))
        .value
        .symbols;
    fake.failing_symbols.push(file("c.rs"));
    let outside = Location {
        external: true,
        ..site("/dep/lib.rs", 3)
    };
    fake.references.insert(
        (Side::Target, changes[0].symbol.location()),
        vec![site("c.rs", 1), outside.clone(), site("unknown.rs", 1)],
    );
    let result = block_on(impact(&fake, &changes[..1]));
    let refs = &result.value[0].references;
    assert_eq!(refs.len(), 3);
    assert!(refs.iter().all(|r| r.within.is_none()));
    // Not an error: the references are listed and nothing is missing from them.
    assert!(result.value[0].gaps.is_empty(), "{:?}", result.value[0].gaps);
    assert_eq!(refs[1].location, outside);
}

#[test]
fn a_remaining_reference_of_a_removed_symbol_is_in_the_container_it_was_in_on_the_base() {
    let (mut fake, files) = changed_fixture();
    let changes = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}))
        .value
        .symbols;
    let dropped = &changes[3];
    fake.symbols.insert(
        (Side::Base, file("c.rs")),
        vec![sym("c.rs", "Report", 0, 10), sym("c.rs", "Report::print", 3, 6)],
    );
    fake.references
        .insert((Side::Base, dropped.symbol.location()), vec![site("c.rs", 4)]);
    let result = block_on(removal_impact(&fake, &changes, &files));
    let site = &result.value[0].call_sites[0];
    assert_eq!(
        site.base.within.as_ref().map(|s| s.name.join("::")),
        Some("Report::print".to_owned())
    );
}

#[test]
fn removed_symbol_call_sites_are_classified() {
    let (mut fake, mut files) = changed_fixture();
    let changes = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}))
        .value
        .symbols;
    let dropped = &changes[3];
    assert_eq!(dropped.change, Change::Removed);

    // c.rs: line 2 deleted, line 5 edited, line 8 unchanged and unresolved,
    // line 20 unchanged and resolves elsewhere. d.rs is deleted.
    files.push(diff(
        "c.rs",
        vec![hunk((2, 3), (2, 2)), hunk((5, 6), (5, 7)), hunk((0, 0), (0, 1))],
    ));
    files.push(FileDiff {
        old_path: Some(file("d.rs")),
        new_path: None,
        old_blob: None,
        new_blob: None,
        hunks: vec![hunk((0, 4), (0, 0))],
    });
    fake.references.insert(
        (Side::Base, dropped.symbol.location()),
        vec![
            site("c.rs", 2),
            site("c.rs", 5),
            site("c.rs", 8),
            site("c.rs", 20),
            site("d.rs", 1),
            site("e.rs", 3),
        ],
    );
    // c.rs gains one line at the top, so old line 8 is new line 9.
    let elsewhere = site("other.rs", 1);
    fake.definitions
        .insert((Side::Target, at("c.rs", 21, 4)), vec![elsewhere.clone()]);

    let result = block_on(removal_impact(&fake, &changes, &files));
    let fates: Vec<_> = result.value[0].call_sites.iter().map(|c| c.fate.clone()).collect();
    assert_eq!(
        fates,
        [
            Fate::Gone,
            Fate::Modified,
            Fate::LikelyBroken,
            Fate::BindingChanged { now: vec![elsewhere] },
            Fate::Gone,
            Fate::LikelyBroken,
        ]
    );
    assert!(
        result
            .inputs
            .primitives
            .contains(&"definition(target:c.rs:9:4) [fake]".to_string())
    );
    assert!(
        result
            .inputs
            .primitives
            .contains(&"definition(target:e.rs:3:4) [fake]".to_string())
    );
}

/// A call-graph node; its line is its first letter, so nodes have distinct positions.
fn node(name: &str) -> SymbolInfo {
    let line = u32::from(name.as_bytes()[0]);
    sym("g.rs", name, line, line + 1)
}

fn graph(edges: &[(&str, &[&str])]) -> Fake {
    let mut fake = Fake::default();
    for (callee, callers) in edges {
        fake.callers.insert(
            (Side::Target, node(callee).location()),
            callers.iter().map(|c| node(c)).collect(),
        );
    }
    fake
}

const BOUNDS: Bounds = Bounds {
    max_depth: 4,
    max_nodes: 100,
};

#[test]
fn reached_reports_the_path_and_the_queries_it_used() {
    let fake = graph(&[("leaf", &["mid"]), ("mid", &["main"])]);
    let result = block_on(reachability(&fake, &node("leaf"), &EntryPoints::default(), BOUNDS));
    let names = |path: &[Sym]| path.iter().map(|s| s.name.join("::")).collect::<Vec<_>>();
    match result.value {
        Reach::Reached { path } => assert_eq!(names(&path), ["leaf", "mid", "main"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        result.inputs.primitives,
        [
            "incoming_calls(target:g.rs:108:3) [fake]",
            "incoming_calls(target:g.rs:109:3) [fake]"
        ]
    );
    assert_eq!(result.inputs.config, ["entry point ::main"]);
}

#[test]
fn the_inputs_say_which_server_answered_and_which_answers_were_reused() {
    let mut fake = graph(&[("leaf", &["main"])]);
    fake.cached = true;
    let result = block_on(reachability(&fake, &node("leaf"), &EntryPoints::default(), BOUNDS));
    assert_eq!(
        result.inputs.primitives,
        ["incoming_calls(target:g.rs:108:3) [fake, cached]"]
    );

    // A query that failed names no server.
    let mut broken = Fake::default();
    let leaf = node("leaf");
    broken.failing_callers.push(leaf.location());
    let result = block_on(reachability(&broken, &leaf, &EntryPoints::default(), BOUNDS));
    assert_eq!(result.inputs.primitives, ["incoming_calls(target:g.rs:108:3)"]);

    let (mut fake, files) = changed_fixture();
    fake.failing_symbols.push(file("a.rs"));
    let result = block_on(changed_symbols(&fake, &NoTexts, &files, &|_, _| {}));
    assert_eq!(
        result.inputs.primitives,
        ["document_symbols(base:a.rs)", "document_symbols(target:a.rs)"]
    );
}

#[test]
fn depth_bound_gives_unreached_not_reached() {
    let fake = graph(&[("leaf", &["mid"]), ("mid", &["main"])]);
    let bounds = Bounds { max_depth: 1, ..BOUNDS };
    let result = block_on(reachability(&fake, &node("leaf"), &EntryPoints::default(), bounds));
    assert_eq!(result.value, Reach::UnreachedWithinBound);
    assert_eq!(result.inputs.bounds, Some(bounds));
}

#[test]
fn cycles_terminate() {
    let fake = graph(&[("a", &["b"]), ("b", &["a"])]);
    let result = block_on(reachability(&fake, &node("a"), &EntryPoints::default(), BOUNDS));
    assert_eq!(result.value, Reach::UnreachedWithinBound);
}

#[test]
fn backend_errors_surface_as_unknown() {
    let mut fake = Fake::default();
    let leaf = node("leaf");
    fake.failing_callers.push(leaf.location());
    let result = block_on(reachability(&fake, &leaf, &EntryPoints::default(), BOUNDS));
    assert!(matches!(result.value, Reach::Unknown { gaps } if gaps[0].contains("no call hierarchy")));
}

#[test]
fn entry_points_match_by_path_prefix_and_name() {
    let entries = EntryPoints(vec![EntryPoint {
        path_prefix: "src/bin/".into(),
        name: "run".into(),
    }]);
    assert!(entries.matches(&sym("src/bin/x.rs", "run", 0, 1).sym));
    assert!(!entries.matches(&sym("src/lib.rs", "run", 0, 1).sym));
    assert!(!entries.matches(&sym("src/bin/x.rs", "main", 0, 1).sym));
}
