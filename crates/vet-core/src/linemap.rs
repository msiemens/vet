//! Where a line of one revision of a text is in another: the one answer behind remapped marks and
//! anchors, lines found in an earlier text, and where a unified diff shows a base line.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::content::Hunk;
use crate::hunks::{changed_lines, changed_slices};
use crate::review::Side;

/// Where a line of an old text is in the new one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MappedLine {
    /// 0-based. For a line that is gone, the nearest line of the new text that took its place.
    pub line: u32,
    /// The line itself is still there; false when an edit replaced or deleted it.
    pub kept: bool,
}

/// The hunks that turn an old text into a new one, in order, which answer where its lines went.
pub struct LineMap<'a> {
    hunks: Cow<'a, [Hunk]>,
}

impl LineMap<'static> {
    /// The map from the unjoined changed lines of two texts ([`changed_lines`]).
    pub fn between(old: &[u8], new: &[u8]) -> Self {
        Self {
            hunks: Cow::Owned(changed_lines(old, new)),
        }
    }

    /// The map between two texts already split into lines, compared as given.
    pub(crate) fn between_lines(old: &[&[u8]], new: &[&[u8]]) -> Self {
        Self {
            hunks: Cow::Owned(changed_slices(old.iter().copied(), new.iter().copied())),
        }
    }
}

impl<'a> LineMap<'a> {
    /// The map of a diff's own hunks, which are the lines of the diff a view shows.
    pub fn of(hunks: &'a [Hunk]) -> Self {
        Self {
            hunks: Cow::Borrowed(hunks),
        }
    }

    /// Where `line` (0-based) of the old text is in the new text.
    pub fn map_line(&self, line: u32) -> MappedLine {
        let mut shift: i64 = 0;
        for h in self.hunks.iter() {
            if line >= h.old.end {
                shift += i64::from(h.new.len()) - i64::from(h.old.len());
            } else if line >= h.old.start {
                // Replaced lines map one to one as far as the replacement reaches; a deletion
                // leaves the line that now stands where it was.
                let within = (line - h.old.start).min(h.new.len().saturating_sub(1));
                return MappedLine {
                    line: h.new.start + within,
                    kept: false,
                };
            } else {
                break;
            }
        }
        MappedLine {
            line: u32::try_from(i64::from(line) + shift).unwrap_or(0),
            kept: true,
        }
    }

    /// The line of the new text that line `line` of `side` faces in a unified diff: itself on the
    /// target side; on the base side the last target line of the hunk that deleted or replaced it
    /// (where a comment on it is shown, below its replacement), else the line it shifted to. Lines
    /// deleted at the end of the file face one past the target's last line.
    pub fn facing(&self, side: Side, line: u32) -> u32 {
        if side == Side::Target {
            return line;
        }
        let mut shift = 0i64;
        for h in self.hunks.iter() {
            if line < h.old.start {
                break;
            }
            if line < h.old.end {
                return h.new.start + h.new.len().max(1) - 1;
            }
            shift += i64::from(h.new.len()) - i64::from(h.old.len());
        }
        u32::try_from((i64::from(line) + shift).max(0)).unwrap_or(u32::MAX)
    }

    /// For each of the `old_len` lines of the old text, its line in the new text if the map leaves
    /// it unchanged.
    pub(crate) fn kept(&self, old_len: usize) -> Vec<Option<usize>> {
        let mut map = vec![None; old_len];
        let mut shift = 0isize;
        let mut line = 0;
        for h in self.hunks.iter() {
            while line < h.old.start as usize {
                map[line] = Some(line.wrapping_add_signed(shift));
                line += 1;
            }
            line = h.old.end as usize;
            shift += h.new.len() as isize - h.old.len() as isize;
        }
        while line < old_len {
            map[line] = Some(line.wrapping_add_signed(shift));
            line += 1;
        }
        map
    }
}

/// Like [`LineMap::kept`] of two texts' lines, but drops mappings the diff chose between equal
/// alternatives: a line stays mapped only if its content is unique in both texts or both its
/// neighbours map to the new line's neighbours.
pub(crate) fn unique_map(old: &[&[u8]], new: &[&[u8]]) -> Vec<Option<usize>> {
    let map = LineMap::between_lines(old, new).kept(old.len());
    fn count<'a>(lines: &[&'a [u8]]) -> HashMap<&'a [u8], u32> {
        let mut c = HashMap::new();
        for l in lines {
            *c.entry(*l).or_default() += 1;
        }
        c
    }
    let (co, cn) = (count(old), count(new));
    let stable = |i: usize, j: usize| {
        let before = if i == 0 {
            j == 0
        } else {
            j > 0 && map[i - 1] == Some(j - 1)
        };
        let after = if i + 1 == old.len() {
            j + 1 == new.len()
        } else {
            map[i + 1] == Some(j + 1)
        };
        before && after
    };
    map.iter()
        .enumerate()
        .map(|(i, &j)| j.filter(|&j| (co[old[i]] == 1 && cn[new[j]] == 1) || stable(i, j)))
        .collect()
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

    fn mapped(old: &str, new: &str, line: u32) -> (u32, bool) {
        let m = LineMap::between(old.as_bytes(), new.as_bytes()).map_line(line);
        (m.line, m.kept)
    }

    #[test]
    fn a_line_moves_by_what_was_added_or_removed_above_it() {
        let old = "a\nb\nc\nd\ne\n";
        assert_eq!(mapped(old, "x\ny\na\nb\nc\nd\ne\n", 3), (5, true));
        assert_eq!(mapped(old, "a\nb\nd\ne\n", 3), (2, true));
        assert_eq!(mapped(old, "a\nb\nc\nd\ne\nf\n", 1), (1, true));
    }

    #[test]
    fn an_edited_or_deleted_line_is_gone_and_maps_to_what_stands_there() {
        let old = "a\nb\nc\nd\ne\n";
        assert_eq!(mapped(old, "a\nB\nc\nd\ne\n", 1), (1, false));
        assert_eq!(mapped(old, "a\nB\nB2\nB3\nd\ne\n", 2), (2, false));
        assert_eq!(mapped(old, "a\nB\nB2\nB3\nd\ne\n", 3), (4, true));
        assert_eq!(mapped(old, "a\nb\nd\ne\n", 2), (2, false));
        assert_eq!(mapped(old, "a\nb\nc\nd\n", 4), (4, false));
    }

    #[test]
    fn the_target_side_faces_itself() {
        assert_eq!(LineMap::of(&[hunk((10, 12), (10, 11))]).facing(Side::Target, 7), 7);
        assert_eq!(LineMap::of(&[hunk((0, 5), (0, 0))]).facing(Side::Target, 3), 3);
    }

    #[test]
    fn a_base_line_faces_the_hunk_it_was_changed_in_or_where_it_shifted_to() {
        // Lines 10..12 of the base became one target line at 10; two lines were added at 17.
        let hunks = [hunk((10, 12), (10, 11)), hunk((18, 18), (17, 19))];
        let map = LineMap::of(&hunks);
        assert_eq!(map.facing(Side::Base, 5), 5);
        // Inside the replaced region: its last target line.
        assert_eq!(map.facing(Side::Base, 10), 10);
        assert_eq!(map.facing(Side::Base, 11), 10);
        // One line fewer after the first hunk, and two more after the second.
        assert_eq!(map.facing(Side::Base, 12), 11);
        assert_eq!(map.facing(Side::Base, 14), 13);
        assert_eq!(map.facing(Side::Base, 30), 31);
    }

    #[test]
    fn a_replaced_region_faces_the_end_of_its_replacement() {
        let hunks = [hunk((4, 6), (4, 9))];
        let map = LineMap::of(&hunks);
        assert_eq!((4..6).map(|l| map.facing(Side::Base, l)).collect::<Vec<_>>(), [8, 8]);
        assert_eq!(map.facing(Side::Base, 6), 9);
    }

    #[test]
    fn deleted_lines_face_the_line_that_took_their_place() {
        let hunks = [hunk((5, 8), (5, 5))];
        let map = LineMap::of(&hunks);
        assert_eq!(map.facing(Side::Base, 6), 5);
        assert_eq!(map.facing(Side::Base, 8), 5);
        assert_eq!(map.facing(Side::Base, 0), 0);
        assert_eq!(LineMap::of(&[hunk((0, 2), (0, 0))]).facing(Side::Base, 1), 0);
    }

    #[test]
    fn an_insertion_moves_the_lines_below_it_but_not_those_above() {
        let hunks = [hunk((3, 3), (3, 5))];
        let map = LineMap::of(&hunks);
        assert_eq!(map.facing(Side::Base, 2), 2);
        assert_eq!(map.facing(Side::Base, 3), 5);
    }

    #[test]
    fn without_hunks_a_base_line_faces_itself() {
        assert_eq!(LineMap::of(&[]).facing(Side::Base, 9), 9);
    }

    fn lines(text: &str) -> Vec<&[u8]> {
        text.split_inclusive('\n').map(str::as_bytes).collect()
    }

    fn unique(old: &str, new: &str) -> Vec<Option<usize>> {
        unique_map(&lines(old), &lines(new))
    }

    #[test]
    fn unchanged_lines_follow_what_was_added_or_removed_around_them() {
        assert_eq!(unique("a\nb\nc\n", "x\na\nb\nc\n"), [Some(1), Some(2), Some(3)]);
        assert_eq!(unique("a\nb\nc\nd\n", "a\nd\n"), [Some(0), None, None, Some(1)]);
        assert_eq!(
            unique("a\nb\nc\nd\n", "a\nB\nc\nd\n"),
            [Some(0), None, Some(2), Some(3)]
        );
    }

    #[test]
    fn a_repeated_line_stays_mapped_only_where_its_neighbours_stay() {
        // The new brace is one of three; the last brace, whose neighbour moved, is not trusted.
        assert_eq!(
            unique("x\n}\n}\ny\n", "x\n}\n}\n}\ny\n"),
            [Some(0), Some(1), None, Some(4)]
        );
        // Away from the change the braces keep their place.
        assert_eq!(
            unique("}\n}\nx\ny\n", "}\n}\nx\nY\n"),
            [Some(0), Some(1), Some(2), None]
        );
    }

    #[test]
    fn a_moved_block_maps_lines_to_equal_lines_in_order() {
        let old = "a1\na2\na3\nb1\nb2\nb3\n";
        let new = "b1\nb2\nb3\na1\na2\na3\n";
        let (o, n) = (lines(old), lines(new));
        let map = unique_map(&o, &n);
        let kept: Vec<usize> = map.iter().flatten().copied().collect();
        assert!(!kept.is_empty());
        assert!(kept.windows(2).all(|w| w[0] < w[1]));
        assert!(map.iter().enumerate().all(|(i, j)| j.is_none_or(|j| o[i] == n[j])));
    }

    #[test]
    fn every_question_gets_the_same_answer_for_a_line_that_stayed() {
        let old = "a\nb\nc\nd\ne\nf\n";
        let new = "x\na\nB\nc\nd\nf\n";
        let map = LineMap::between(old.as_bytes(), new.as_bytes());
        let (kept, unique) = (map.kept(6), unique(old, new));
        for line in 0..6u32 {
            let m = map.map_line(line);
            if m.kept {
                assert_eq!(kept[line as usize], Some(m.line as usize), "line {line}");
                assert_eq!(map.facing(Side::Base, line), m.line, "line {line}");
                assert_eq!(unique[line as usize], Some(m.line as usize), "line {line}");
            } else {
                assert_eq!(kept[line as usize], None, "line {line}");
            }
        }
    }
}
