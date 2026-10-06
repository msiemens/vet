//! Span remapping as a partition (SPEC §5.2).

use crate::content::{BlobSource, Changeset, LineRange, SnapshotId};
use crate::review::{Anchor, FileRemapInput, ReviewedSpan, SpanState};
use crate::text::{Text, unique_map};

/// The runs of one side's recorded lines that are still identical and uniquely
/// mapped, anchored in the current snapshot.
struct Side {
    runs: Vec<Anchor>,
    /// Every recorded line survived as one run.
    full: bool,
}

/// Remap `span` from its recorded snapshots to the changeset `to`. Pass the
/// recorded span each time, not an earlier remap, so reverting an edit restores
/// coverage. `None` for a side whose file no longer exists. Only exact line
/// identity carries coverage; a span may become several or none.
///
/// Target lines that survive while any base line changed are
/// `SeenAgainstOldBase`.
pub fn remap_span(
    span: &ReviewedSpan,
    to: Changeset,
    base: Option<&FileRemapInput>,
    target: Option<&FileRemapInput>,
    blobs: &dyn BlobSource,
) -> Vec<ReviewedSpan> {
    let b = span
        .base
        .as_ref()
        .map(|a| remap_side(a, base, to.base, blobs));
    let t = span
        .target
        .as_ref()
        .map(|a| remap_side(a, target, to.target, blobs));

    if b.iter().chain(&t).all(|s| s.full) {
        let one =
            |s: Option<Side>| s.map(|s| s.runs.into_iter().next().expect("full side has a run"));
        return vec![ReviewedSpan {
            base: one(b),
            target: one(t),
            state: span.state,
        }];
    }

    let base_changed = b.as_ref().is_some_and(|s| !s.full);
    let target_state = if base_changed {
        SpanState::SeenAgainstOldBase
    } else {
        span.state
    };
    let pieces =
        |s: Option<Side>, state| s.into_iter().flat_map(|s| s.runs).map(move |a| (a, state));
    let base_pieces = pieces(b, span.state).map(|(a, state)| ReviewedSpan {
        base: Some(a),
        target: None,
        state,
    });
    let target_pieces = pieces(t, target_state).map(|(a, state)| ReviewedSpan {
        base: None,
        target: Some(a),
        state,
    });
    base_pieces.chain(target_pieces).collect()
}

fn remap_side(
    anchor: &Anchor,
    input: Option<&FileRemapInput>,
    snapshot: SnapshotId,
    blobs: &dyn BlobSource,
) -> Side {
    let none = Side {
        runs: Vec::new(),
        full: false,
    };
    let Some(FileRemapInput {
        new_path: Some(path),
        old_blob: Some(old_id),
        new_blob: Some(new_id),
        ..
    }) = input
    else {
        return none;
    };
    let (Some(old), Some(new)) = (blobs.blob(*old_id), blobs.blob(*new_id)) else {
        return none;
    };
    let (old, new) = (Text::new(old), Text::new(new));
    if old.raw_hash(anchor.range) != Some(anchor.content_hash_raw) {
        return none;
    }
    let map = if old_id == new_id {
        (0..old.raw.len()).map(Some).collect()
    } else {
        unique_map(&old.raw, &new.raw)
    };

    let mut ranges: Vec<LineRange> = Vec::new();
    for old_line in anchor.range.start..anchor.range.end {
        let Some(new_line) = map[old_line as usize] else {
            continue;
        };
        let new_line = new_line as u32;
        match ranges.last_mut() {
            Some(r) if r.end == new_line => r.end += 1,
            _ => ranges.push(LineRange {
                start: new_line,
                end: new_line + 1,
            }),
        }
    }
    let full = ranges.len() == 1 && ranges[0].len() == anchor.range.len();
    let runs = ranges
        .into_iter()
        .filter_map(|r| new.anchor(anchor.side, snapshot, path.clone(), r))
        .collect();
    Side { runs, full }
}
