//! The diff of a file as a line-based editor view shows it: its numbering of lines, and the text
//! changes shaped so that the view draws one block per hunk.
//!
//! A text's lines are numbered as an editor does: a final line break is followed by one more,
//! empty line, and an empty text has one line.

use vet_core::content::Hunk;
use vet_core::inline::{TextChange, hunk_changes};
use vet_core::text::Lines;

/// Number of lines of `bytes`.
pub fn line_count(bytes: &[u8]) -> u32 {
    bytes.iter().filter(|&&b| b == b'\n').count() as u32 + 1
}

/// The last line of `bytes` that holds text (0-based): the one before the empty line a final line
/// break starts, and 0 for an empty text.
pub fn last_text_line(bytes: &[u8]) -> u32 {
    (line_count(bytes) - u32::from(bytes.last() == Some(&b'\n'))).saturating_sub(1)
}

/// The changes of `hunks` between two texts: down to the words that differ within a replaced
/// region (see [`hunk_changes`]), shaped so that a view which snaps changes to whole lines and
/// groups the ones that touch shows one block of exactly the hunk's lines for each hunk. That needs
/// the hunks to be apart by `vet_core::hunks::JOIN_GAP`.
pub fn text_changes(base: &str, target: &str, hunks: &[Hunk]) -> Vec<TextChange> {
    let (base, target) = (Lines::new(base), Lines::new(target));
    let (base, target) = (Doc(&base), Doc(&target));
    hunks
        .iter()
        .flat_map(|h| {
            let whole = TextChange {
                from_base: base.0.offset(h.old.start),
                to_base: base.0.offset(h.old.end),
                from_target: target.0.offset(h.new.start),
                to_target: target.0.offset(h.new.end),
            };
            one_block(hunk_changes(base.0, target.0, h), whole, &base, &target)
        })
        .collect()
}

/// A text with the lines an editor makes of it.
struct Doc<'a>(&'a Lines<'a>);

impl Doc<'_> {
    fn total(&self) -> u32 {
        self.0.total()
    }

    /// Index of the line holding `offset`, numbered as an editor does.
    fn line_at(&self, offset: u32) -> usize {
        let starts = self.0.starts();
        let line = starts.partition_point(|&u| u <= offset) - 1;
        if offset == self.total() && !self.0.text().ends_with('\n') && line > 0 {
            line - 1
        } else {
            line
        }
    }

    /// Offset of the start of the line holding `offset`.
    fn line_from(&self, offset: u32) -> u32 {
        self.0.starts()[self.line_at(offset)]
    }

    /// Offset of the end of the line holding `offset`, before its line break.
    fn line_to(&self, offset: u32) -> u32 {
        let starts = self.0.starts();
        let line = self.line_at(offset);
        match starts.get(line + 1) {
            None => self.total(),
            Some(&next) if line + 2 == starts.len() && !self.0.text().ends_with('\n') => next,
            Some(&next) => next - 1,
        }
    }
}

/// Where a line-based editor puts the start of a change: at the start of the line holding
/// it, except that a change starting at the end of a line on both sides is taken to begin
/// with the line break, so it starts on the next line.
fn snap_start(base: &Doc, target: &Doc, from_base: u32, from_target: u32) -> (u32, u32) {
    if base.line_to(from_base) == from_base
        && target.line_to(from_target) == from_target
        && from_base < base.total()
        && from_target < target.total()
    {
        (from_base + 1, from_target + 1)
    } else {
        (base.line_from(from_base), target.line_from(from_target))
    }
}

/// Where a line-based editor puts the end of a change: the end itself if it is at the start
/// of a line on both sides, else the start of the line after the one holding it.
fn snap_end(base: &Doc, target: &Doc, to_base: u32, to_target: u32) -> (u32, u32) {
    if base.line_from(to_base) == to_base && target.line_from(to_target) == to_target {
        (to_base, to_target)
    } else {
        (base.line_to(to_base) + 1, target.line_to(to_target) + 1)
    }
}

/// Where the lines ending at `offset` end in an editor: the offset, if it starts a line, else
/// one unit past the end of the line holding it.
fn end_of_lines(doc: &Doc, offset: u32) -> u32 {
    if doc.line_from(offset) == offset {
        offset
    } else {
        doc.line_to(offset) + 1
    }
}

/// Shapes the changes of a hunk (`whole`: its full line range) so that the editor shows
/// them as one block of exactly the hunk's lines: the first change starts and the last one
/// ends in the hunk's lines, and no two are far enough apart to be split.
fn one_block(mut out: Vec<TextChange>, whole: TextChange, base: &Doc, target: &Doc) -> Vec<TextChange> {
    let start = (whole.from_base, whole.from_target);
    let first = &mut out[0];
    if snap_start(base, target, first.from_base, first.from_target) != start {
        (first.from_base, first.from_target) = start;
    }
    // A change at the start of the hunk includes the line break before it where the editor
    // would take its start for a line break: a first line that is empty on both sides.
    if (first.from_base, first.from_target) == start
        && start.0 > 0
        && start.1 > 0
        && snap_start(base, target, start.0, start.1) != start
    {
        first.from_base -= 1;
        first.from_target -= 1;
    }
    // The end of the hunk as the editor shows it. The editor snaps both ends of a change
    // together, so where only one side's text ends without a line break, the other side's
    // end moves before its line break.
    let end = (end_of_lines(base, whole.to_base), end_of_lines(target, whole.to_target));
    let last = out.last_mut().expect("not empty");
    if snap_end(base, target, last.to_base, last.to_target) != end {
        let (to_base, to_target) = (whole.to_base, whole.to_target);
        let candidates = [
            (to_base, to_target),
            (to_base, to_target.saturating_sub(1)),
            (to_base.saturating_sub(1), to_target),
        ];
        (last.to_base, last.to_target) = candidates
            .into_iter()
            .find(|&(b, t)| snap_end(base, target, b, t) == end)
            .unwrap_or((to_base, to_target));
    }
    // The editor slides a pure insertion or deletion of lines to an equal line next to it,
    // unless it also replaces a character: the one before it, which both texts share.
    for change in &mut out {
        let multi_line = |doc: &Doc, from, to| to - from > 3 && doc.line_from(to) > from;
        let pure = (change.from_base == change.to_base && multi_line(target, change.from_target, change.to_target))
            || (change.from_target == change.to_target && multi_line(base, change.from_base, change.to_base));
        let starts_at_line_end = base.line_to(change.from_base) == change.from_base
            && target.line_to(change.from_target) == change.from_target;
        if pure && !starts_at_line_end && change.from_base > 0 && change.from_target > 0 {
            change.from_base -= 1;
            change.from_target -= 1;
        }
    }
    let mut block: Vec<TextChange> = Vec::with_capacity(out.len());
    for change in out {
        if let Some(prev) = block.last_mut() {
            let (end_base, end_target) = snap_end(base, target, prev.to_base, prev.to_target);
            let (next_base, next_target) = snap_start(base, target, change.from_base, change.from_target);
            if next_base > end_base + 1 && next_target > end_target + 1 {
                prev.to_base = change.to_base;
                prev.to_target = change.to_target;
                continue;
            }
        }
        block.push(change);
    }
    block
}

#[cfg(test)]
mod tests {
    use vet_core::content::LineRange;

    use super::*;

    fn hunk(old: (u32, u32), new: (u32, u32)) -> Hunk {
        Hunk {
            old: LineRange {
                start: old.0,
                end: old.1,
            },
            new: LineRange {
                start: new.0,
                end: new.1,
            },
        }
    }

    fn change(base: (u32, u32), target: (u32, u32)) -> TextChange {
        TextChange {
            from_base: base.0,
            to_base: base.1,
            from_target: target.0,
            to_target: target.1,
        }
    }

    #[test]
    fn lines_are_numbered_as_an_editor_does() {
        assert_eq!(line_count(b""), 1);
        assert_eq!(line_count(b"a"), 1);
        assert_eq!(line_count(b"a\n"), 2);
        assert_eq!(line_count(b"a\nb"), 2);
        assert_eq!(last_text_line(b""), 0);
        assert_eq!(last_text_line(b"a"), 0);
        assert_eq!(last_text_line(b"a\n"), 0);
        assert_eq!(last_text_line(b"a\nb"), 1);
        assert_eq!(last_text_line(b"a\nb\n"), 1);
    }

    #[test]
    fn pure_addition_and_deletion_are_whole_lines() {
        // The change includes the line break before the lines.
        let (a, b) = ("x\ny\n", "x\nnew\nlines\ny\n");
        assert_eq!(text_changes(a, b, &[hunk((1, 1), (1, 3))]), [change((1, 2), (1, 12))]);
        assert_eq!(text_changes(b, a, &[hunk((1, 3), (1, 1))]), [change((1, 12), (1, 2))]);
    }

    #[test]
    fn replaced_line_is_down_to_the_word() {
        let (a, b) = ("keep\nlet total = 1;\nkeep\n", "keep\nlet sum = 1;\nkeep\n");
        assert_eq!(text_changes(a, b, &[hunk((1, 2), (1, 2))]), [change((9, 14), (9, 12))]);
    }

    #[test]
    fn no_trailing_newline() {
        let (a, b) = ("a\nold", "a\nnew");
        assert_eq!(text_changes(a, b, &[hunk((1, 2), (1, 2))]), [change((2, 5), (2, 5))]);
        // The last line gains its terminator and a line.
        assert_eq!(
            text_changes("a", "a\nb", &[hunk((0, 1), (0, 2))]),
            [change((1, 1), (1, 3))]
        );
    }

    #[test]
    fn crlf_counts_as_one_unit() {
        let (a, b) = ("a\r\nold\r\nz\r\n", "a\r\nnew\r\nz\r\n");
        assert_eq!(text_changes(a, b, &[hunk((1, 2), (1, 2))]), [change((2, 5), (2, 5))]);
        // A hunk without a differing word is still one change of its lines.
        assert_eq!(text_changes(a, b, &[hunk((2, 3), (2, 3))]), [change((6, 8), (6, 8))]);
    }

    #[test]
    fn utf16_offsets() {
        let (a, b) = ("😀 old\n", "😀 new\n");
        assert_eq!(text_changes(a, b, &[hunk((0, 1), (0, 1))]), [change((3, 6), (3, 6))]);
    }

    #[test]
    fn adjacent_hunks_and_close_changes() {
        let (a, b) = ("a\nb\nc\nd\n", "a2\nb2\nc\nd2\n");
        let hunks = [hunk((0, 2), (0, 2)), hunk((3, 4), (3, 4))];
        // The two words of the first hunk are one change, one line break apart.
        assert_eq!(
            text_changes(a, b, &hunks),
            [change((0, 3), (0, 5)), change((6, 7), (8, 10))]
        );
    }

    #[test]
    fn unrelated_lines_stay_split_per_line() {
        let changes = text_changes("aa\nbb\n", "xx\nyy\n", &[hunk((0, 2), (0, 2))]);
        assert_eq!(changes, [change((0, 5), (0, 5))]);
    }

    /// The lines of base and target an editor shows as the block of `changes`, as start and end offsets.
    fn block(a: &str, b: &str, changes: &[TextChange]) -> ((u32, u32), (u32, u32)) {
        let (a, b) = (Lines::new(a), Lines::new(b));
        let (a, b) = (Doc(&a), Doc(&b));
        let (first, last) = (changes[0], changes[changes.len() - 1]);
        (
            snap_start(&a, &b, first.from_base, first.from_target),
            snap_end(&a, &b, last.to_base, last.to_target),
        )
    }

    #[test]
    fn a_first_line_empty_on_both_sides_is_not_taken_for_a_line_break() {
        // Deleting the empty line 1 of "x", "", "", "y": both texts have an empty line 1.
        let (a, b) = ("x\n\n\ny\n", "x\n\ny\n");
        let changes = text_changes(a, b, &[hunk((1, 2), (1, 1))]);
        assert_eq!(changes, [change((1, 3), (1, 2))]);
        assert_eq!(block(a, b, &changes), ((2, 2), (3, 2)));
    }

    #[test]
    fn inserted_lines_are_not_slid_to_an_equal_line_after_them() {
        let (a, b) = ("x\nfoo\nbaz\n", "x\nfoo\nbar\nfoo\nbaz\n");
        let changes = text_changes(a, b, &[hunk((1, 1), (1, 3))]);
        // The change replaces the line break before the lines, so that it cannot be slid.
        assert_eq!(changes, [change((1, 2), (1, 10))]);
        assert_eq!(block(a, b, &changes), ((2, 2), (2, 10)));
    }

    #[test]
    fn the_changes_of_a_hunk_reach_its_first_and_last_line() {
        // The words match across lines, so the first and last lines (empty) have no change.
        let (a, b) = ("k\nw\n\nz\n", "k\n\nn\n\nz\n");
        let hunks = [hunk((1, 3), (1, 4))];
        let changes = text_changes(a, b, &hunks);
        assert_eq!(block(a, b, &changes), ((2, 2), (5, 6)));
    }

    #[test]
    fn changes_inside_a_hunk_are_not_split_by_lines_between_them() {
        // Lines 1 to 3 differ, but only the words on lines 1 and 3 are found.
        let (a, b) = ("x\nfoo bar\nmid\nbaz qux\ny\n", "x\nfoo BAR\nmid\nbaz QUX\ny\n");
        let changes = text_changes(a, b, &[hunk((1, 4), (1, 4))]);
        assert_eq!(changes.len(), 1);
        assert_eq!(block(a, b, &changes), ((2, 2), (22, 22)));
    }

    #[test]
    fn a_text_that_ends_without_a_line_break_ends_the_block_with_its_line() {
        // The target gains its final line break; the base text has none.
        let (a, b) = (")", ")\n");
        let changes = text_changes(a, b, &[hunk((0, 1), (0, 1))]);
        let (start, end) = block(a, b, &changes);
        assert_eq!((start, end), ((0, 0), (2, 2)));
    }
}
