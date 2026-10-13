//! Text search in the files of a view, or of the whole repository.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Completion, FileStatus, Hunk, HunkState, View};

/// What is looked for, and how. A line is matched on its own; a pattern never spans lines.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchPattern {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
    /// `text` is a regular expression in the syntax of Rust's `regex` crate.
    pub regex: bool,
}

/// Where to look.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum SearchScope {
    /// The changed files of `view`, each as the diff shows it. Only the files the review
    /// filter shows are looked in.
    View { view: View },
    /// Every file of the target snapshot (ignored files are not in it), the changed ones with
    /// their review state. The review filter does not apply: a file it hides is searched like
    /// an unchanged one. `side` does not apply either; only the target is searched.
    Repository,
}

/// The sides of a diff to look in. A context line is on both.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SearchSide {
    Both,
    Target,
    Base,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SearchLineKind {
    Context,
    Added,
    Removed,
}

/// A match in a line, in UTF-16 units from the start of the line (not of `text`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    pub from: u32,
    pub to: u32,
}

/// A line with matches.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchLine {
    pub kind: SearchLineKind,
    /// 0-based: in the target text for a context or added line, in the base text for a removed
    /// one.
    pub line: u32,
    /// The hunk of an added or removed line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hunk: Option<u32>,
    /// The line, or for a long one the part round its first match.
    pub text: String,
    /// Where `text` starts in the line, in UTF-16 units.
    pub text_start: u32,
    pub hits: Vec<SearchHit>,
}

/// A file with matches.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchFile {
    pub path: String,
    /// Absent for a file the view does not change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<FileStatus>,
    /// The file's review progress, for a changed file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub completion: Option<Completion>,
    /// The hunks of a changed file, with the review state of each in `states`; a preview of a
    /// match draws the diff from them. Empty for an unchanged file.
    pub hunks: Vec<Hunk>,
    pub states: Vec<HunkState>,
    pub lines: Vec<SearchLine>,
    /// The matches in `lines`.
    pub hits: u32,
}
