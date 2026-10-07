//! Word-level changes inside a line hunk, for intra-line highlighting.

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::content::Hunk;
use crate::text::{Lines, unit_len};

/// Hunks with more tokens than this on a side are not diffed further.
const MAX_TOKENS: usize = 20_000;
/// Changes with fewer than this many units between them are one change.
const MIN_GAP: u32 = 3;

/// A changed range of both texts, in units (see [`crate::text`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextChange {
    pub from_base: u32,
    pub to_base: u32,
    pub from_target: u32,
    pub to_target: u32,
}

/// The changes of `hunk`: a pure insertion or deletion as one change, anything else down to the
/// words that differ. A hunk without a differing word (only its line breaks differ) is one change
/// of its lines.
pub fn hunk_changes(base: &Lines, target: &Lines, hunk: &Hunk) -> Vec<TextChange> {
    let (base0, target0) = (base.offset(hunk.old.start), target.offset(hunk.new.start));
    let whole = TextChange {
        from_base: base0,
        to_base: base.offset(hunk.old.end),
        from_target: target0,
        to_target: target.offset(hunk.new.end),
    };
    let (old, new) = (tokens(base.slice(hunk.old)), tokens(target.slice(hunk.new)));
    if hunk.old.is_empty() || hunk.new.is_empty() || old.len().max(new.len()) > MAX_TOKENS {
        return vec![whole];
    }
    let (old_at, new_at) = (offsets(&old), offsets(&new));
    let mut out: Vec<TextChange> = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &old, &new) {
        let (old_range, new_range) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index..old_index + old_len, new_index..new_index),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index..old_index, new_index..new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (old_index..old_index + old_len, new_index..new_index + new_len),
        };
        let change = TextChange {
            from_base: base0 + old_at[old_range.start],
            to_base: base0 + old_at[old_range.end],
            from_target: target0 + new_at[new_range.start],
            to_target: target0 + new_at[new_range.end],
        };
        match out.last_mut() {
            Some(last)
                if last.to_base + MIN_GAP > change.from_base && last.to_target + MIN_GAP > change.from_target =>
            {
                last.to_base = change.to_base;
                last.to_target = change.to_target;
            }
            _ => out.push(change),
        }
    }
    if out.is_empty() {
        out.push(whole);
    }
    out
}

/// Offset of each token's start, and of the end of the last one.
fn offsets(tokens: &[&str]) -> Vec<u32> {
    let mut at = vec![0];
    for token in tokens {
        at.push(at[at.len() - 1] + unit_len(token));
    }
    at
}

/// Words, runs of blanks, line breaks and single other characters.
fn tokens(text: &str) -> Vec<&str> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Blank,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c == ' ' || c == '\t' {
            Class::Blank
        } else {
            Class::Other
        }
    };
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let kind = class(c);
        let mut end = start + c.len_utf8();
        if kind == Class::Other {
            if c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n') {
                end += 1;
                chars.next();
            }
        } else {
            while let Some(&(i, n)) = chars.peek() {
                if class(n) != kind {
                    break;
                }
                end = i + n.len_utf8();
                chars.next();
            }
        }
        out.push(&text[start..end]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::LineRange;

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

    fn changes(base: &str, target: &str, hunks: &[Hunk]) -> Vec<TextChange> {
        let (base, target) = (Lines::new(base), Lines::new(target));
        hunks.iter().flat_map(|h| hunk_changes(&base, &target, h)).collect()
    }

    #[test]
    fn pure_addition_and_deletion_are_whole_lines() {
        let (a, b) = ("x\ny\n", "x\nnew\nlines\ny\n");
        assert_eq!(changes(a, b, &[hunk((1, 1), (1, 3))]), [change((2, 2), (2, 12))]);
        assert_eq!(changes(b, a, &[hunk((1, 3), (1, 1))]), [change((2, 12), (2, 2))]);
    }

    #[test]
    fn replaced_line_is_down_to_the_word() {
        let (a, b) = ("keep\nlet total = 1;\nkeep\n", "keep\nlet sum = 1;\nkeep\n");
        assert_eq!(changes(a, b, &[hunk((1, 2), (1, 2))]), [change((9, 14), (9, 12))]);
    }

    #[test]
    fn no_trailing_newline() {
        let (a, b) = ("a\nold", "a\nnew");
        assert_eq!(changes(a, b, &[hunk((1, 2), (1, 2))]), [change((2, 5), (2, 5))]);
        // The last line gains its terminator and a line.
        assert_eq!(changes("a", "a\nb", &[hunk((0, 1), (0, 2))]), [change((1, 1), (1, 3))]);
    }

    #[test]
    fn crlf_counts_as_one_unit() {
        let (a, b) = ("a\r\nold\r\nz\r\n", "a\r\nnew\r\nz\r\n");
        assert_eq!(changes(a, b, &[hunk((1, 2), (1, 2))]), [change((2, 5), (2, 5))]);
        // A hunk without a differing word is still one change of its lines.
        assert_eq!(changes(a, b, &[hunk((2, 3), (2, 3))]), [change((6, 8), (6, 8))]);
    }

    #[test]
    fn utf16_offsets() {
        let (a, b) = ("😀 old\n", "😀 new\n");
        assert_eq!(changes(a, b, &[hunk((0, 1), (0, 1))]), [change((3, 6), (3, 6))]);
    }

    #[test]
    fn close_words_are_one_change_and_hunks_stay_apart() {
        let (a, b) = ("a\nb\nc\nd\n", "a2\nb2\nc\nd2\n");
        let hunks = [hunk((0, 2), (0, 2)), hunk((3, 4), (3, 4))];
        // The two words of the first hunk are one change, one line break apart.
        assert_eq!(changes(a, b, &hunks), [change((0, 3), (0, 5)), change((6, 7), (8, 10))]);
    }

    #[test]
    fn unrelated_lines_are_one_change() {
        let changes = changes("aa\nbb\n", "xx\nyy\n", &[hunk((0, 2), (0, 2))]);
        assert_eq!(changes, [change((0, 5), (0, 5))]);
    }

    #[test]
    fn words_far_apart_in_a_hunk_are_separate_changes() {
        let (a, b) = ("foo bar\nmid\nbaz qux\n", "foo BAR\nmid\nbaz QUX\n");
        assert_eq!(
            changes(a, b, &[hunk((0, 3), (0, 3))]),
            [change((4, 7), (4, 7)), change((16, 19), (16, 19))]
        );
    }
}
