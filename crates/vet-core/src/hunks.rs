//! What counts as one hunk of a line diff. The hunks are the unit of review state, so this is
//! policy of the domain, not of how a diff is displayed.

use imara_diff::sources::byte_lines;
use imara_diff::{Algorithm, Diff, InternedInput};

use crate::content::{Hunk, LineRange};
use crate::text::unit_len;

/// 0-based line hunks, histogram algorithm. Hunks with almost nothing between them are one
/// hunk (see [`JOIN_GAP`]).
pub fn line_hunks(old: &[u8], new: &[u8]) -> Vec<Hunk> {
    let old_lines: Vec<&[u8]> = byte_lines(old).collect();
    join_close_hunks(changed_lines(old, new), |r| {
        String::from_utf8_lossy(&old_lines[r.start as usize..r.end as usize].concat()).into_owned()
    })
}

/// 0-based line hunks of exactly the changed lines, not joined: what a merge or a question of
/// which lines changed needs, where [`line_hunks`] answers what is reviewed together.
pub fn changed_lines(old: &[u8], new: &[u8]) -> Vec<Hunk> {
    let input = InternedInput::new(byte_lines(old), byte_lines(new));
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let range = |r: std::ops::Range<u32>| LineRange {
        start: r.start,
        end: r.end,
    };
    diff.hunks()
        .map(|h| Hunk {
            old: range(h.before),
            new: range(h.after),
        })
        .collect()
}

/// Hunks with fewer than this many units of unchanged text between them are one hunk: changes
/// that close together are read as one edit, and a view that groups changes by their distance
/// shows them as one block, so reviewing them apart would split what it shows together.
pub const JOIN_GAP: u32 = 4;

/// Joins the hunks that are close to each other (see [`JOIN_GAP`]). `gap` is the old text of the
/// lines between two hunks.
fn join_close_hunks(hunks: impl IntoIterator<Item = Hunk>, gap: impl Fn(LineRange) -> String) -> Vec<Hunk> {
    let mut out: Vec<Hunk> = Vec::new();
    for h in hunks {
        match out.last_mut() {
            Some(last)
                if unit_len(&gap(LineRange {
                    start: last.old.end,
                    end: h.old.start,
                })) < JOIN_GAP =>
            {
                last.old.end = h.old.end;
                last.new.end = h.new.end;
            }
            _ => out.push(h),
        }
    }
    out
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn hunks_with_little_between_them_are_joined() {
        let lines = ["a\n", "\n", "}\n", "abc\n", "b\n"];
        let gap = |r: LineRange| lines[r.start as usize..r.end as usize].concat();
        let joined = |hunks: &[Hunk]| join_close_hunks(hunks.iter().copied(), gap);
        // An empty line (1 unit) or a line of one character (2 units) between them.
        assert_eq!(
            joined(&[hunk((0, 1), (0, 1)), hunk((2, 3), (2, 3))]),
            [hunk((0, 3), (0, 3))]
        );
        assert_eq!(
            joined(&[hunk((1, 2), (1, 2)), hunk((3, 4), (3, 4))]),
            [hunk((1, 4), (1, 4))]
        );
        // Both together are 3 units, still joined; a line of 4 units is not.
        assert_eq!(
            joined(&[hunk((0, 1), (0, 1)), hunk((3, 4), (3, 4))]),
            [hunk((0, 4), (0, 4))]
        );
        assert_eq!(joined(&[hunk((2, 3), (2, 3)), hunk((4, 5), (4, 5))]).len(), 2);
    }

    fn spans(old: &str, new: &str) -> Vec<((u32, u32), (u32, u32))> {
        line_hunks(old.as_bytes(), new.as_bytes())
            .iter()
            .map(|h| ((h.old.start, h.old.end), (h.new.start, h.new.end)))
            .collect()
    }

    #[test]
    fn line_hunks_with_little_between_them_are_joined() {
        assert_eq!(spans("a\n\nb\n", "A\n\nB\n"), [((0, 3), (0, 3))]);
        assert_eq!(spans("a\n\nb\n\nc\n", "A\n\nB\n\nC\n"), [((0, 5), (0, 5))]);
        assert_eq!(spans("a\r\n\r\nb\r\n", "A\r\n\r\nB\r\n"), [((0, 3), (0, 3))]);
        assert_eq!(spans("a\n\n\nb\n", "A\n\n\nB\n"), [((0, 4), (0, 4))]);
        assert_eq!(spans("a\n}\nb\n", "A\n}\nB\n"), [((0, 3), (0, 3))]);
    }

    #[test]
    fn line_hunks_with_a_line_between_them_stay_apart() {
        assert_eq!(
            spans("a\nxyz\nb\n", "A\nxyz\nB\n"),
            [((0, 1), (0, 1)), ((2, 3), (2, 3))]
        );
        assert_eq!(spans("a\n \nb\n", "A\n \nB\n"), [((0, 3), (0, 3))]);
        assert_eq!(
            spans("a\n   \nb\n", "A\n   \nB\n"),
            [((0, 1), (0, 1)), ((2, 3), (2, 3))]
        );
    }
}
