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

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SymbolInfo {
    pub id: SymbolId,
    /// The whole definition, including its body.
    pub span: TextSpan,
    /// Where its name is: the position to ask `References` about.
    pub at: Position,
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
}

/// Why a semantic answer is missing, so the shell can say so instead of
/// showing nothing (§7.1: a language without a usable server degrades).
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
    /// The server does not start, lacks a required operation or broke the protocol.
    ServerFailed,
    /// The server would execute project code, which the review does not allow.
    AnalysisLimited,
    /// The base snapshot cannot be rebuilt (§6.1).
    SnapshotUnavailable,
    /// The server failed this query.
    QueryFailed,
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

/// A file whose symbols could not be determined: unknown, not unchanged.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileGap {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangedSymbols {
    pub revision: u32,
    pub symbols: Vec<ChangedSymbol>,
    /// Changed files whose analysis failed, with why: their symbols are unknown. Files no
    /// language is configured for are not listed.
    pub gaps: Vec<FileGap>,
    pub inputs: DerivedInputs,
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

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Impact {
    pub revision: u32,
    pub symbol: SymbolInfo,
    pub references: Vec<SemanticLocation>,
    pub callers: Vec<SymbolInfo>,
    /// Queries that failed; the matching list is unknown, not empty.
    pub gaps: Vec<String>,
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
        reason: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CallSite {
    /// Where the reference is on the base side.
    pub base: SemanticLocation,
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
    pub gap: Option<String>,
    pub inputs: DerivedInputs,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Reach {
    /// The path from the changed symbol to the entry point.
    Reached { path: Vec<SymbolId> },
    /// No entry point within the bounds and entry points in `inputs`.
    UnreachedWithinBound,
    /// Backend gaps (dependency injection, reflection, serialization).
    Unknown { gaps: Vec<String> },
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
