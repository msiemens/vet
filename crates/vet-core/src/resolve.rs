//! Anchor resolution (SPEC §5.1).

use crate::content::{BlobSource, Hash, LineRange, RepoPath};
use crate::review::{
    Anchor, AnchorState, Confidence, FileRemapInput, Location, StaleReason, Strategy,
};
use crate::text::{NO_LINE, Text, unique_map};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resolution {
    pub state: AnchorState,
    /// Where the anchor now is; `None` for `Stale` and `Ambiguous`.
    pub location: Option<Location>,
}

/// Score units: a content match is worth `CONTENT`, context adds 0..=100.
const CONTENT: u32 = 200;
/// A context-only candidate (edited content) needs at least this much context.
const MIN_CONTEXT: u32 = 80;
/// Candidates within this distance of the best one compete with it.
const MARGIN: u32 = 50;

/// Resolve `anchor` into the same side of the current changeset, directly from
/// its recorded snapshot. `file` is the anchor's file there; `changed` are the
/// other changed files on that side, searched for moved content.
pub fn resolve_anchor(
    anchor: &Anchor,
    file: &FileRemapInput,
    changed: &[FileRemapInput],
    blobs: &dyn BlobSource,
) -> Resolution {
    let text = |b| blobs.blob(b).map(Text::new);
    let old = file.old_blob.and_then(text);
    let new = file.new_blob.and_then(text);

    if let (Some(old), Some(new), Some(path)) = (&old, &new, &file.new_path) {
        let same = file.old_path == file.new_path && file.old_blob == file.new_blob;
        if let Some((range, whitespace_changed)) = exact(anchor, same, old, new) {
            return Resolution {
                state: AnchorState::Exact { whitespace_changed },
                location: Some(Location {
                    file: path.clone(),
                    range,
                }),
            };
        }
    }

    let mut candidates = Vec::new();
    if let (Some(new), Some(path)) = (&new, &file.new_path) {
        scan(anchor, new, path, false, &mut candidates);
    }
    for other in changed.iter().filter(|c| c.new_path != file.new_path) {
        if let (Some(new), Some(path)) = (other.new_blob.and_then(text), &other.new_path) {
            scan(anchor, &new, path, true, &mut candidates);
        }
    }
    infer(candidates, file.new_blob.is_none())
}

fn exact(anchor: &Anchor, same_blob: bool, old: &Text, new: &Text) -> Option<(LineRange, bool)> {
    let r = anchor.range;
    if same_blob && new.raw_hash(r) == Some(anchor.content_hash_raw) {
        return Some((r, false));
    }
    if let Some(m) = map_range(&old.raw, &new.raw, r)
        && new.raw_hash(m) == Some(anchor.content_hash_raw)
    {
        return Some((m, false));
    }
    let m = map_range(&old.norm, &new.norm, r)?;
    (new.norm_hash(m) == Some(anchor.content_hash_norm)).then_some((m, true))
}

/// The new range of `r` if every line of it maps uniquely and contiguously.
fn map_range(old: &[&[u8]], new: &[&[u8]], r: LineRange) -> Option<LineRange> {
    if r.is_empty() || r.end as usize > old.len() {
        return None;
    }
    let map = unique_map(old, new);
    let start = map[r.start as usize]?;
    (0..r.len() as usize)
        .all(|k| map[r.start as usize + k] == Some(start + k))
        .then(|| LineRange {
            start: start as u32,
            end: start as u32 + r.len(),
        })
}

struct Candidate {
    location: Location,
    score: u32,
    same_file: bool,
}

/// Windows of the anchor's length that match its content, or, unless
/// `content_only`, its surrounding context (the content was edited).
fn scan(
    anchor: &Anchor,
    text: &Text,
    path: &RepoPath,
    content_only: bool,
    out: &mut Vec<Candidate>,
) {
    let n = anchor.range.len();
    if n == 0 || n as usize > text.raw.len() {
        return;
    }
    for start in 0..=(text.raw.len() as u32 - n) {
        let range = LineRange {
            start,
            end: start + n,
        };
        let content = text.norm_hash(range) == Some(anchor.content_hash_norm);
        let context = context_score(&anchor.context_hashes, &text.context(range));
        if content || (!content_only && context >= MIN_CONTEXT) {
            out.push(Candidate {
                location: Location {
                    file: path.clone(),
                    range,
                },
                score: if content { CONTENT } else { 0 } + context,
                same_file: !content_only,
            });
        }
    }
}

/// Percentage of the anchor's real context lines found at the same position.
fn context_score(anchor: &[Hash], candidate: &[Hash]) -> u32 {
    let real = anchor.iter().filter(|&&h| h != NO_LINE).count() as u32;
    if real == 0 {
        return 0;
    }
    let hits = anchor
        .iter()
        .zip(candidate)
        .filter(|(a, c)| **a != NO_LINE && a == c)
        .count() as u32;
    hits * 100 / real
}

fn infer(mut candidates: Vec<Candidate>, file_deleted: bool) -> Resolution {
    candidates.sort_by_key(|c| std::cmp::Reverse(c.score));
    let Some(best) = candidates.first() else {
        let reason = if file_deleted {
            StaleReason::FileDeleted
        } else {
            StaleReason::ContextChanged
        };
        return Resolution {
            state: AnchorState::Stale { reason },
            location: None,
        };
    };
    let competing: Vec<_> = candidates
        .iter()
        .take_while(|c| best.score - c.score < MARGIN)
        .collect();
    if competing.len() > 1 {
        let candidates = competing.iter().map(|c| c.location.clone()).collect();
        return Resolution {
            state: AnchorState::Ambiguous { candidates },
            location: None,
        };
    }
    let confidence = match best.score {
        275.. => Confidence::High,
        100.. => Confidence::Medium,
        _ => Confidence::Low,
    };
    let strategy = if best.same_file {
        Strategy::ContextWindow
    } else {
        Strategy::CrossFileMove
    };
    Resolution {
        state: AnchorState::Inferred {
            strategy,
            confidence,
        },
        location: Some(best.location.clone()),
    }
}
