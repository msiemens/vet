//! How a text is shown: its sides, syntax highlighting and changed ranges.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A side of the changeset.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Side {
    Base,
    Target,
}

/// Syntax highlighting of a text, as the plain numbers of a `Vec<u32>`: three per coloured
/// range, in order and not overlapping: the gap since the end of the previous range (since the
/// start of the text for the first), the range's length, and its kind (0 keyword, 1 string,
/// 2 number, 3 comment, 4 type, 5 function, 6 attribute, 7 heading, 8 link). Offsets are
/// UTF-16 units of the text with `\r\n` counted as one unit, as for [`TextChange`].
pub type Highlights = Vec<u32>;

/// A changed range of the base and target text, as a view that draws changes by whole lines needs
/// it. Offsets are UTF-16 units of the text with `\r\n` counted as one unit.
///
/// The changes of a file are line-snapped: a line-based view that snaps a change to the lines it
/// touches and groups the changes that touch shows exactly one block for each hunk, covering the
/// hunk's lines. The first change of a block starts in the hunk's first line and the last one ends
/// in its last line. Lines are counted as in `FileChange::base_lines`: a final line break starts
/// one more, empty line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextChange {
    pub from_base: u32,
    pub to_base: u32,
    pub from_target: u32,
    pub to_target: u32,
}
