//! A line-level 3-way merge that says where each output line came from and where the two sides
//! disagreed. It rebases a patch onto a moved base (see [`crate::interdiff`]).

use imara_diff::sources::byte_lines;

use crate::content::{Hunk, LineRange};
use crate::hunks::changed_lines;

/// Where a line of a merge came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineOrigin {
    /// Written by `ours`, the patch being applied.
    Ours,
    /// Written by `theirs`, the side the patch is applied onto.
    Theirs,
    /// Unchanged from the base, or written identically by both.
    Both,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Merged {
    pub text: Vec<u8>,
    /// One entry per line of `text`.
    pub origin: Vec<LineOrigin>,
    /// Output line ranges where the sides changed the same base lines differently. They hold
    /// `ours`; the range is empty where `ours` deleted the lines.
    pub conflicts: Vec<LineRange>,
}

/// Merges `ours` and `theirs`, both derived from `base`. Changes of the two sides that touch or
/// overlap are one region: identical on both sides they apply once, else they conflict and
/// `ours` is kept.
pub fn merge3(base: &[u8], ours: &[u8], theirs: &[u8]) -> Merged {
    let base_lines: Vec<&[u8]> = byte_lines(base).collect();
    let ours_lines: Vec<&[u8]> = byte_lines(ours).collect();
    let theirs_lines: Vec<&[u8]> = byte_lines(theirs).collect();
    let (our_changes, their_changes) = (changed_lines(base, ours), changed_lines(base, theirs));

    let mut out = Out::default();
    let (mut o, mut t) = (0, 0);
    let mut pos = 0u32;
    while o < our_changes.len() || t < their_changes.len() {
        let start = match (our_changes.get(o), their_changes.get(t)) {
            (Some(a), Some(b)) => a.old.start.min(b.old.start),
            (Some(a), None) => a.old.start,
            (None, Some(b)) => b.old.start,
            (None, None) => unreachable!(),
        };
        let mut end = start;
        let (o0, t0) = (o, t);
        loop {
            if let Some(h) = our_changes.get(o).filter(|h| h.old.start <= end) {
                end = end.max(h.old.end);
                o += 1;
            } else if let Some(h) = their_changes.get(t).filter(|h| h.old.start <= end) {
                end = end.max(h.old.end);
                t += 1;
            } else {
                break;
            }
        }
        out.push(&base_lines[pos as usize..start as usize], LineOrigin::Both);
        let ours_region = region(&our_changes[o0..o], start, end);
        let theirs_region = region(&their_changes[t0..t], start, end);
        match (ours_region, theirs_region) {
            (Some(r), None) => out.push(&ours_lines[r.start as usize..r.end as usize], LineOrigin::Ours),
            (None, Some(r)) => out.push(&theirs_lines[r.start as usize..r.end as usize], LineOrigin::Theirs),
            (Some(a), Some(b)) => {
                let (a, b) = (
                    &ours_lines[a.start as usize..a.end as usize],
                    &theirs_lines[b.start as usize..b.end as usize],
                );
                if a == b {
                    out.push(a, LineOrigin::Both);
                } else {
                    let first = out.origin.len() as u32;
                    out.push(a, LineOrigin::Ours);
                    out.conflicts.push(LineRange {
                        start: first,
                        end: out.origin.len() as u32,
                    });
                }
            }
            (None, None) => unreachable!(),
        }
        pos = end;
    }
    out.push(&base_lines[pos as usize..], LineOrigin::Both);
    Merged {
        text: out.text,
        origin: out.origin,
        conflicts: out.conflicts,
    }
}

/// The lines of one side that replace the base lines `start..end`, given the side's changes
/// inside them; `None` if it has none.
fn region(changes: &[Hunk], start: u32, end: u32) -> Option<LineRange> {
    let (first, last) = (changes.first()?, changes.last()?);
    Some(LineRange {
        start: first.new.start - (first.old.start - start),
        end: last.new.end + (end - last.old.end),
    })
}

#[derive(Default)]
struct Out {
    text: Vec<u8>,
    origin: Vec<LineOrigin>,
    conflicts: Vec<LineRange>,
}

impl Out {
    fn push(&mut self, lines: &[&[u8]], origin: LineOrigin) {
        for line in lines {
            self.text.extend_from_slice(line);
        }
        self.origin.extend(std::iter::repeat_n(origin, lines.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LineOrigin::{Both, Ours, Theirs};

    fn merge(base: &str, ours: &str, theirs: &str) -> (String, Vec<LineOrigin>, Vec<(u32, u32)>) {
        let m = merge3(base.as_bytes(), ours.as_bytes(), theirs.as_bytes());
        (
            String::from_utf8(m.text).unwrap(),
            m.origin,
            m.conflicts.iter().map(|c| (c.start, c.end)).collect(),
        )
    }

    #[test]
    fn changes_to_different_regions_both_apply_with_their_origin() {
        let base = "a\nb\nc\nd\ne\nf\n";
        let (text, origin, conflicts) = merge(base, "a\nB\nc\nd\ne\nf\n", "a\nb\nc\nd\nE\nf\n");
        assert_eq!(text, "a\nB\nc\nd\nE\nf\n");
        assert_eq!(origin, [Both, Ours, Both, Both, Theirs, Both]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn an_identical_change_on_both_sides_applies_once() {
        let (text, origin, conflicts) = merge("a\nb\nc\n", "a\nX\nc\n", "a\nX\nc\n");
        assert_eq!(text, "a\nX\nc\n");
        assert_eq!(origin, [Both, Both, Both]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn different_changes_to_the_same_lines_conflict_and_keep_ours() {
        let (text, origin, conflicts) = merge("a\nb\nc\n", "a\nOURS\nc\n", "a\nTHEIRS\nc\n");
        assert_eq!(text, "a\nOURS\nc\n");
        assert_eq!(origin, [Both, Ours, Both]);
        assert_eq!(conflicts, [(1, 2)]);
    }

    #[test]
    fn a_deletion_against_an_edit_conflicts_with_an_empty_range() {
        let (text, _, conflicts) = merge("a\nb\nc\n", "a\nc\n", "a\nB\nc\n");
        assert_eq!(text, "a\nc\n");
        assert_eq!(conflicts, [(1, 1)]);
    }

    #[test]
    fn insertions_at_the_same_place_conflict_but_apart_they_do_not() {
        let (_, _, conflicts) = merge("a\nb\n", "a\nX\nb\n", "a\nY\nb\n");
        assert_eq!(conflicts, [(1, 2)]);
        let (text, origin, conflicts) = merge("a\nb\nc\nd\ne\n", "X\na\nb\nc\nd\ne\n", "a\nb\nc\nd\ne\nY\n");
        assert_eq!(text, "X\na\nb\nc\nd\ne\nY\n");
        assert_eq!(origin[0], Ours);
        assert_eq!(origin[6], Theirs);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn a_change_touching_the_other_sides_change_is_one_region() {
        // `ours` edits line 1, `theirs` the adjacent line 2: git reports this as a conflict.
        let (_, _, conflicts) = merge("a\nb\nc\n", "A\nb\nc\n", "a\nB\nc\n");
        assert_eq!(conflicts.len(), 1);
    }

    #[test]
    fn a_missing_final_newline_is_a_change_of_that_line() {
        let (text, origin, conflicts) = merge("a\nm\nn\nb", "a\nm\nn\nb\n", "A\nm\nn\nb");
        assert_eq!(text, "A\nm\nn\nb\n");
        assert_eq!(origin, [Theirs, Both, Both, Ours]);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn a_side_without_changes_yields_the_other() {
        assert_eq!(merge("a\nb\n", "a\nB\n", "a\nb\n").0, "a\nB\n");
        assert_eq!(merge("a\nb\n", "a\nb\n", "a\nB\n").0, "a\nB\n");
        assert_eq!(merge("", "x\n", "").0, "x\n");
    }
}
