//! Creating spans for hunks (SPEC §5.2).

use crate::content::{FileDiff, Hunk, LineRange, RepoPath, SnapshotId};
use crate::review::{ReviewedSpan, Side, SpanState, sides};
use crate::text::Text;

/// Lines of one file on one side.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Region {
    pub file: RepoPath,
    pub side: Side,
    pub range: LineRange,
}

/// The non-empty sides of `hunk`.
pub fn hunk_regions(diff: &FileDiff, hunk: &Hunk) -> Vec<Region> {
    (sides(diff, hunk).into_iter())
        .filter(|(_, _, range)| !range.is_empty())
        .filter_map(|(side, file, range)| {
            file.map(|file| Region {
                file: file.clone(),
                side,
                range,
            })
        })
        .collect()
}

/// For each of `hunks`, a `Seen` span over its lines on both sides, anchored in the given
/// snapshots and texts; `None` for a hunk with no lines or one whose text is missing. Each text
/// is split into lines and hashed once, however many hunks it has.
pub fn hunk_spans(
    diff: &FileDiff,
    hunks: &[&Hunk],
    snapshots: (SnapshotId, SnapshotId),
    texts: (Option<&[u8]>, Option<&[u8]>),
) -> Vec<Option<ReviewedSpan>> {
    let texts = (texts.0.map(Text::new), texts.1.map(Text::new));
    hunks
        .iter()
        .map(|hunk| {
            let regions = hunk_regions(diff, hunk);
            let anchor = |side, snapshot, text: Option<&Text>| {
                let r = regions.iter().find(|r| r.side == side)?;
                text?.anchor(side, snapshot, r.file.clone(), r.range)
            };
            let base = anchor(Side::Base, snapshots.0, texts.0.as_ref());
            let target = anchor(Side::Target, snapshots.1, texts.1.as_ref());
            let wanted = |side| regions.iter().any(|r| r.side == side);
            if regions.is_empty()
                || (wanted(Side::Base) && base.is_none())
                || (wanted(Side::Target) && target.is_none())
            {
                return None;
            }
            Some(ReviewedSpan {
                base,
                target,
                state: SpanState::Seen,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Hash;
    use crate::text::SPLITS;

    const SNAP: SnapshotId = SnapshotId(Hash([7; 32]));

    fn path(s: &str) -> RepoPath {
        RepoPath::from(s)
    }

    fn range(start: u32, end: u32) -> LineRange {
        LineRange { start, end }
    }

    #[test]
    fn a_hunk_span_covers_both_sides() {
        let diff = FileDiff {
            old_path: Some(path("a")),
            new_path: Some(path("a")),
            old_blob: None,
            new_blob: None,
            hunks: vec![],
        };
        let hunk = Hunk {
            old: range(2, 4),
            new: range(2, 5),
        };
        let t: Vec<u8> = (0..20).map(|i| format!("line {i}\n")).collect::<String>().into_bytes();
        let [s] = &hunk_spans(&diff, &[&hunk], (SNAP, SNAP), (Some(&t), Some(&t)))[..] else {
            panic!("one span per hunk");
        };
        let s = s.clone().unwrap();
        assert_eq!(s.base.unwrap().range, range(2, 4));
        assert_eq!(s.target.unwrap().range, range(2, 5));
        assert_eq!(hunk_spans(&diff, &[&hunk], (SNAP, SNAP), (None, Some(&t))), [None]);
    }

    /// Forty one-line hunks over a text of 400 lines.
    fn many_hunks() -> (String, String, FileDiff) {
        let old: String = (0..400).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("5\n", "5 edited\n");
        let hunks: Vec<Hunk> = (0..40)
            .map(|k| Hunk {
                old: range(10 * k + 5, 10 * k + 6),
                new: range(10 * k + 5, 10 * k + 6),
            })
            .collect();
        let diff = FileDiff {
            old_path: Some(path("a")),
            new_path: Some(path("a")),
            old_blob: None,
            new_blob: None,
            hunks,
        };
        (old, new, diff)
    }

    #[test]
    fn the_spans_of_many_hunks_anchor_each_hunk_as_alone() {
        let (old, new, diff) = many_hunks();
        let texts = (Some(old.as_bytes()), Some(new.as_bytes()));
        let all: Vec<&Hunk> = diff.hunks.iter().collect();

        let together = hunk_spans(&diff, &all, (SNAP, SNAP), texts);

        let alone: Vec<_> = all
            .iter()
            .flat_map(|h| hunk_spans(&diff, &[h], (SNAP, SNAP), texts))
            .collect();
        assert_eq!(together, alone);
        let ranges: Vec<_> = together
            .iter()
            .map(|s| {
                let s = s.as_ref().unwrap();
                (s.base.as_ref().unwrap().range, s.target.as_ref().unwrap().range)
            })
            .collect();
        let expected: Vec<_> = diff.hunks.iter().map(|h| (h.old, h.new)).collect();
        assert_eq!(ranges, expected);
    }

    #[test]
    fn each_text_is_split_once_however_many_hunks_there_are() {
        let (old, new, diff) = many_hunks();
        let all: Vec<&Hunk> = diff.hunks.iter().collect();

        let before = SPLITS.with(|n| n.get());
        hunk_spans(&diff, &all, (SNAP, SNAP), (Some(old.as_bytes()), Some(new.as_bytes())));

        assert_eq!(SPLITS.with(|n| n.get()) - before, 2);
    }
}
