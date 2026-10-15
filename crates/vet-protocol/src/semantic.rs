//! Answers of the language servers and the analyses derived from them (§7).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Highlights, Side};

/// 0-based line and UTF-16 column.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Position {
    pub line: u32,
    pub col: u32,
}

/// End exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextSpan {
    pub start: Position,
    pub end: Position,
}

/// A symbol is identified by its file and container chain (outermost first),
/// as in §7.3: moves and renames change the identity.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolId {
    pub path: String,
    pub name: Vec<String>,
}

/// A symbol as a list names it: its identity, and how many of the leading names of the chain are
/// modules or namespaces, which a list leaves out. The count is display data, not identity.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolName {
    pub id: SymbolId,
    pub modules: u32,
    /// What it is, as the file's outline says.
    pub kind: SymbolKind,
}

/// What a symbol is, as far as a list needs to tell them apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SymbolKind {
    /// A class, struct, enum, interface or the like.
    Type,
    /// A function, method, constructor or operator.
    Function,
    /// A module, namespace or package.
    Module,
    /// A variable, field, constant or anything else.
    Other,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolInfo {
    pub id: SymbolId,
    /// See [`SymbolName`].
    pub modules: u32,
    pub kind: SymbolKind,
    /// The whole definition, including its body.
    pub span: TextSpan,
    /// Where its name is: the position to ask `References` about.
    pub at: Position,
    /// Vet's conclusion that this is a test, by the file's path and a `tests`/`test` container it
    /// is in (derived by file name, not from a language server; SPEC I6).
    pub test: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SemanticLocation {
    /// Repository-relative; an absolute path if `external`.
    pub path: String,
    pub span: TextSpan,
    /// Outside the repository: a dependency or toolchain source, which Vet has
    /// no snapshot of.
    pub external: bool,
}

/// A place that refers to a symbol, with the symbol it is in.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Reference {
    pub location: SemanticLocation,
    /// The innermost namespace, type or function around it; absent at the top level of a file,
    /// outside the repository and where the language server cannot list the file's symbols.
    pub within: Option<SymbolName>,
    /// Vet's conclusion that this place is a test, by its file's path and the containers of the
    /// symbol it is in (derived by file name, not from a language server; SPEC I6).
    pub test: bool,
}

/// `revision` and `side` are those of the request.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Locations {
    pub revision: u32,
    pub side: Side,
    pub locations: Vec<SemanticLocation>,
    pub unavailable: Option<Unavailable>,
}

/// Where a place stands against the changeset, on the side that was asked about.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LocationChange {
    /// In a file the changeset does not touch, or outside the repository.
    Unchanged,
    /// In a changed file, outside its changed lines.
    File,
    /// In a changed hunk.
    Hunk,
}

/// A place a navigation request found, with what a list of such places shows.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocationRow {
    /// Repository-relative; an absolute path if `external`.
    pub path: String,
    /// 1-based, where the match starts.
    pub line: u32,
    /// 1-based UTF-16 column, where the match starts.
    pub col: u32,
    /// The innermost namespace, type or function around it; absent at the top level of a file,
    /// outside the repository and where the language server cannot list the file's symbols.
    pub within: Option<SymbolName>,
    /// The line of `line` without surrounding whitespace, and cut after a few hundred characters;
    /// empty where the file has no text to read.
    pub text: String,
    /// The highlighting of `text` as the line has it in its file, empty for a file without a
    /// known language or text (see [`Highlights`]).
    pub highlights: Highlights,
    /// Where the match is in `text`: start and end (exclusive), UTF-16 offsets. The end of the
    /// text if the match goes on beyond the line.
    pub match_start: u32,
    pub match_end: u32,
    pub change: LocationChange,
    /// The index among the file's hunks of the changed hunk the place is in; set exactly when
    /// `change` is `Hunk`. The hunk's review state is the manifest's `FileReview.hunks` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hunk: Option<u32>,
    /// Outside the repository: a dependency or toolchain source, which Vet has no snapshot of.
    pub external: bool,
}

/// `revision` and `side` are those of the request. `rows` are the first of `total` places
/// that are in the repository, by path and line, then those outside, likewise.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocationRows {
    pub revision: u32,
    pub side: Side,
    pub rows: Vec<LocationRow>,
    /// More than `rows` has when the answer was cut.
    pub total: u32,
    pub unavailable: Option<Unavailable>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Diagnostic {
    pub span: TextSpan,
    pub severity: Severity,
    pub message: String,
    /// The tool that reported it, e.g. `rustc`.
    pub source: Option<String>,
    pub code: Option<String>,
    pub origin: DiagnosticOrigin,
}

/// Whether the change introduced a diagnostic (derived, SPEC §7.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiagnosticOrigin {
    New,
    /// The base has it too, on a line that maps to it.
    Existing,
    /// Not compared with the base yet.
    Pending,
    /// The base side's diagnostics are not available: never claimed to be new.
    Unknown,
}

/// Why a semantic answer is missing, so the shell can say so instead of
/// showing nothing (I6: unavailable is never shown as nothing).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Unavailable {
    pub kind: UnavailableKind,
    /// A few words for the kind, to show where there is room for little, e.g. "No language server".
    pub label: String,
    /// Fit to show.
    pub message: String,
    /// The language whose server is the problem, as `ListServers` names it;
    /// absent for `Unsupported`.
    pub language: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UnavailableKind {
    /// No language is configured for this kind of file.
    Unsupported,
    /// No server was found for the language: see `ListServers`.
    ServerMissing,
    /// The server does not start, cannot take documents or broke the protocol.
    ServerFailed,
    /// The language's server does not offer the operation asked for.
    NotOffered,
    /// The server would execute project code, which the review does not allow.
    AnalysisLimited,
    /// The base snapshot cannot be rebuilt (§6.1).
    SnapshotUnavailable,
    /// The server failed this query.
    QueryFailed,
    /// The server was at work on its own while it gave the answer, so what it lacks is unknown,
    /// not absent: the origins of diagnostics whose base side was given so. Asking again once
    /// the server is done gives the answer.
    Incomplete,
}

/// What a derived result was computed from (I6: derived results show their inputs).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DerivedInputs {
    pub analysis: String,
    pub bounds: Option<DerivedBounds>,
    /// The primitive queries consulted, e.g. `incoming_calls(target:src/a.rs:3:4)`.
    pub primitives: Vec<String>,
    /// The diff hunks consulted, e.g. `src/a.rs: old 3..5 new 3..6`.
    pub hunks: Vec<String>,
    /// Configuration that shaped the result, e.g. the entry points.
    pub config: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DerivedBounds {
    pub max_depth: u32,
    pub max_nodes: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SymbolChange {
    Added,
    Modified,
    /// Present on the base side only.
    Removed,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangedSymbol {
    /// The target-side symbol, or the base-side one if `Removed`.
    pub symbol: SymbolInfo,
    pub change: SymbolChange,
}

/// What the changeset did to the symbols of one file (derived, §7.3).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileSymbols {
    pub revision: u32,
    pub path: String,
    /// The target side's changed symbols in source order, then the removed ones in the order of
    /// the base side.
    pub symbols: Vec<ChangedSymbol>,
    /// Unchanged symbols that hold changed ones, in source order: the containers to show them in.
    pub context: Vec<SymbolInfo>,
    /// Set when the file's symbols could not be determined: they are unknown, not unchanged.
    pub gap: Option<Unavailable>,
    pub inputs: DerivedInputs,
}

/// A symbol of a file's outline, and what the changeset did to it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OutlineSymbol {
    /// The target side's symbol, or the base side's if it exists there only.
    pub symbol: SymbolInfo,
    /// Absent: the changeset did not change it, or the view has no marks.
    pub change: Option<SymbolChange>,
}

/// Every symbol of a file, changed or not (derived, §7.3).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileOutline {
    pub revision: u32,
    pub path: String,
    /// In reading order: the symbols of the target side in source order, each removed one at the
    /// place it had in the base. A symbol is nested in the symbol whose name is the longest
    /// proper prefix of its own.
    pub symbols: Vec<OutlineSymbol>,
    /// Set when the file's symbols are unknown (no language support, a server that does not
    /// answer): `symbols` is then empty.
    pub unavailable: Option<Unavailable>,
}

/// What one language's server found for a search of symbols by name.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LanguageSymbols {
    /// `rust`, `typescript`, `python` or `csharp`, as in `ServerInfo`.
    pub language: String,
    /// In the order the server gave them, which has its own idea of a match.
    pub symbols: Vec<SymbolInfo>,
    /// Set when the language's server could not answer: why, fit to show.
    pub failure: Option<Unavailable>,
}

/// The symbols of the target side that match a name, per language of the project.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceSymbols {
    pub revision: u32,
    pub languages: Vec<LanguageSymbols>,
}

/// The symbol a position is in, if any.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolAt {
    pub revision: u32,
    /// The innermost symbol (a function, type, module, field or constant) that holds the position:
    /// the target side's, or the base side's if it exists there only. Absent outside every symbol,
    /// and where the language server cannot list the file's symbols (`unavailable`).
    pub symbol: Option<SymbolInfo>,
    /// Absent: the changeset did not change it.
    pub change: Option<SymbolChange>,
    pub unavailable: Option<Unavailable>,
}

/// A symbol's declaration as text, with its syntax highlighting.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolSignature {
    pub code: String,
    /// The highlighting of `code`, empty for prose or an unknown language (see [`Highlights`]).
    pub highlights: Highlights,
}

/// A changed symbol that reaches the one asked about through calls of changed symbols alone.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangedCaller {
    pub symbol: SymbolInfo,
    /// Added or modified.
    pub change: SymbolChange,
    /// How many changed symbols lie between it and the one asked about.
    pub depth: u32,
}

/// What a derived result lacks.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Gap {
    /// A query failed, so what it was for is unknown. `what` names that; none when the server
    /// lacks the operation altogether.
    Failed { what: Option<String>, why: Unavailable },
    /// There are more callers than the walk follows.
    TooManyCallers { max: u32 },
    /// Callers further up than the walk goes were not followed.
    TooDeep { max_depth: u32 },
}

/// The calls around a symbol of the target side (derived, §7.3).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Calls {
    pub revision: u32,
    pub symbol: SymbolInfo,
    /// In the order of a walk down the callers: each is followed by those that call it.
    pub callers: Vec<ChangedCaller>,
    /// What it calls that was added or modified (target side) or removed (base side).
    pub callees: Vec<ChangedSymbol>,
    /// Unchanged symbols that call it directly; empty unless the symbol itself changed.
    pub affected: Vec<SymbolInfo>,
    /// Queries that failed or bounds that cut the walk; the matching list is incomplete.
    pub gaps: Vec<Gap>,
    pub inputs: DerivedInputs,
}

/// The references to a symbol of the target side, each with the symbol it is in.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Usages {
    pub revision: u32,
    pub symbol: SymbolInfo,
    pub references: Vec<Reference>,
    /// Set when the references could not be listed: they are unknown, not absent.
    pub gap: Option<Unavailable>,
    pub inputs: DerivedInputs,
}

/// What became of one reference to a removed symbol, judged in the target.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CallFate {
    /// Its lines were deleted.
    Gone,
    /// Its lines were edited.
    Modified,
    /// Unchanged text that no longer resolves.
    LikelyBroken,
    /// Unchanged text that now resolves elsewhere.
    BindingChanged {
        now: Vec<SemanticLocation>,
    },
    Unknown {
        why: Unavailable,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CallSite {
    /// Where the reference is on the base side.
    pub base: Reference,
    pub fate: CallFate,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemovalImpact {
    pub revision: u32,
    pub symbol: SymbolInfo,
    pub call_sites: Vec<CallSite>,
    /// Set when the references could not be listed: the call sites are unknown, not absent.
    pub gap: Option<Unavailable>,
    pub inputs: DerivedInputs,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Reach {
    /// The path from the changed symbol to the entry point.
    Reached { path: Vec<SymbolName> },
    /// No entry point within the bounds and entry points in `inputs`.
    UnreachedWithinBound,
    /// Backend gaps (dependency injection, reflection, serialization).
    Unknown { gaps: Vec<Gap> },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Reachability {
    pub revision: u32,
    pub symbol: SymbolInfo,
    pub reach: Reach,
    pub inputs: DerivedInputs,
}
