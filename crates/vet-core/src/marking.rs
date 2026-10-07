//! Creating spans for hunks (SPEC §5.2).

use crate::content::{FileDiff, Hunk, LineRange, RepoPath, SnapshotId};
use crate::review::{Anchor, ReviewedSpan, Side, SpanState};

/// Lines of one file on one side.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Region {
    pub file: RepoPath,
    pub side: Side,
    pub range: LineRange,
}

/// The non-empty sides of `hunk`.
pub fn hunk_regions(diff: &FileDiff, hunk: &Hunk) -> Vec<Region> {
    (diff.sides(hunk).into_iter())
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

/// A `Seen` span over the lines of `hunk` on both sides, anchored in the given
/// snapshots and texts. `None` if the hunk has no lines or a text is missing.
pub fn hunk_span(
    diff: &FileDiff,
    hunk: &Hunk,
    snapshots: (SnapshotId, SnapshotId),
    texts: (Option<&[u8]>, Option<&[u8]>),
) -> Option<ReviewedSpan> {
    let regions = hunk_regions(diff, hunk);
    let anchor = |side, snapshot, text: Option<&[u8]>| {
        let r = regions.iter().find(|r| r.side == side)?;
        Anchor::from_text(side, snapshot, r.file.clone(), text?, r.range)
    };
    let base = anchor(Side::Base, snapshots.0, texts.0);
    let target = anchor(Side::Target, snapshots.1, texts.1);
    let wanted = |side| regions.iter().any(|r| r.side == side);
    if regions.is_empty() || (wanted(Side::Base) && base.is_none()) || (wanted(Side::Target) && target.is_none()) {
        return None;
    }
    Some(ReviewedSpan {
        base,
        target,
        state: SpanState::Seen,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Hash;

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
        let s = hunk_span(&diff, &hunk, (SNAP, SNAP), (Some(&t), Some(&t))).unwrap();
        assert_eq!(s.base.unwrap().range, range(2, 4));
        assert_eq!(s.target.unwrap().range, range(2, 5));
        assert!(hunk_span(&diff, &hunk, (SNAP, SNAP), (None, Some(&t))).is_none());
    }
}
