//! Anchor resolution (SPEC §5.1).

use crate::content::{Hash, LineRange, RepoPath};
use crate::prepared::PreparedFile;
use crate::review::{Anchor, AnchorState, Confidence, Location, StaleReason, Strategy};
use crate::text::{CONTEXT, NO_LINE, Text};

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
/// Lines a context-only candidate may exceed twice the anchor's length by.
const GROWTH: usize = 3;
/// Candidates within this distance of the best one compete with it.
const MARGIN: u32 = 50;

/// Resolve `anchor` into the same side of the current changeset, directly from
/// its recorded snapshot. `file` is the anchor's file there; `changed` are the
/// changed files on that side (it may include `file`), searched for moved
/// content.
pub fn resolve_anchor(anchor: &Anchor, file: &PreparedFile, changed: &[PreparedFile]) -> Resolution {
    let path = file.input.new_path.as_ref();
    if let (Some(new), Some(path)) = (&file.new, path)
        && let Some((range, whitespace_changed)) = exact(anchor, file, new)
    {
        return Resolution {
            state: AnchorState::Exact { whitespace_changed },
            location: Some(Location {
                file: path.clone(),
                range,
            }),
        };
    }

    // The recorded lines, when the old text still has them, make the content
    // check a comparison of line hashes instead of a hash per window.
    let lines = file
        .old
        .as_ref()
        .filter(|old| old.norm_hash(anchor.range) == Some(anchor.content_hash_norm))
        .map(|old| &old.hashes()[anchor.range.start as usize..anchor.range.end as usize]);
    let mut candidates = Vec::new();
    if let (Some(new), Some(path)) = (&file.new, path) {
        scan(anchor, lines, new, path, false, &mut candidates);
    }
    for other in changed.iter().filter(|c| c.input.new_path.as_ref() != path) {
        if let (Some(new), Some(path)) = (&other.new, &other.input.new_path) {
            scan(anchor, lines, new, path, true, &mut candidates);
        }
    }
    let none = candidates.is_empty();
    let found = infer(candidates, file.input.new_blob.is_none());
    let weak = match found.state {
        AnchorState::Stale { .. } => true,
        AnchorState::Inferred { confidence, .. } => confidence == Confidence::Low,
        _ => false,
    };
    // Deleted lines are not found by a weak context match either: that one lands on neighbours.
    if weak
        && let Some(path) = path
        && let Some((range, strategy)) = between_survivors(anchor.range, file)
        && (none || strategy == Strategy::Deleted)
    {
        return Resolution {
            state: AnchorState::Inferred {
                strategy,
                confidence: Confidence::Low,
            },
            location: Some(Location {
                file: path.clone(),
                range,
            }),
        };
    }
    found
}

/// Where an anchor whose lines were rewritten went, when text around it survived: the lines
/// between the nearest unchanged line before it and the one after it, in the new text, however
/// far apart those are. Of the same length as before, the anchor keeps its offset in them;
/// otherwise it covers all of them. When nothing took the place of its lines it is on the line
/// where they were, as `Strategy::Deleted`.
fn between_survivors(r: LineRange, file: &PreparedFile) -> Option<(LineRange, Strategy)> {
    let map = file.norm_map()?;
    let old = file.old.as_ref()?;
    let new_lines = file.new.as_ref()?.raw.len();
    // A line of only brackets or punctuation says little about where code went.
    let survivor = |i: usize| {
        let j = map[i]?;
        has_text(old.raw[i]).then_some((i, j))
    };
    let before = (0..r.start as usize).rev().find_map(survivor);
    let after = (r.end as usize..map.len()).find_map(survivor);
    if before.is_none() && after.is_none() {
        return None;
    }
    // The gap of old lines between the survivors, and where it starts and ends in the new text.
    let (old_first, new_first) = before.map_or((0, 0), |(i, j)| (i + 1, j + 1));
    let (old_end, new_end) = after.unwrap_or((map.len(), new_lines));
    // Lines that took the place of the old ones, not old lines that stayed or brackets.
    let kept: std::collections::HashSet<usize> = map.iter().flatten().copied().collect();
    let new = file.new.as_ref()?;
    if (new_first..new_end).all(|j| kept.contains(&j) || !has_text(new.raw[j])) {
        let line = (new_first + (r.start as usize - old_first)).min(new_end);
        let line = line.min(new_lines.checked_sub(1)?) as u32;
        return Some((
            LineRange {
                start: line,
                end: line + 1,
            },
            Strategy::Deleted,
        ));
    }
    let new_len = new_end - new_first;
    let (start, len) = if new_len == old_end - old_first {
        (new_first + (r.start as usize - old_first), r.len() as usize)
    } else {
        (new_first, new_len)
    };
    let range = LineRange {
        start: start as u32,
        end: (start + len) as u32,
    };
    Some((range, Strategy::ContextWindow))
}

fn has_text(line: &[u8]) -> bool {
    line.iter().any(|b| b.is_ascii_alphanumeric() || *b >= 0x80)
}

fn exact(anchor: &Anchor, file: &PreparedFile, new: &Text) -> Option<(LineRange, bool)> {
    let r = anchor.range;
    if let Some(m) = map_range(file.raw_map()?, r)
        && new.raw_hash(m) == Some(anchor.content_hash_raw)
    {
        return Some((m, false));
    }
    let m = map_range(file.norm_map()?, r)?;
    (new.norm_hash(m) == Some(anchor.content_hash_norm)).then_some((m, true))
}

/// The new range of `r` if every line of it maps uniquely and contiguously.
fn map_range(map: &[Option<usize>], r: LineRange) -> Option<LineRange> {
    if r.is_empty() || r.end as usize > map.len() {
        return None;
    }
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
/// `content_only`, its surrounding context (the content was edited). Context
/// matches may also differ in length from the anchor, up to `2n + GROWTH`
/// lines, when the lines between intact context were added or removed.
fn scan(
    anchor: &Anchor,
    recorded: Option<&[Hash]>,
    text: &Text,
    path: &RepoPath,
    content_only: bool,
    out: &mut Vec<Candidate>,
) {
    let n = anchor.range.len() as usize;
    let lines = text.raw.len();
    if n == 0 {
        return;
    }
    let context = ContextHits::new(&anchor.context_hashes, text.hashes());
    let mut push = |start: usize, end: usize, content: bool, context: u32| {
        out.push(Candidate {
            location: Location {
                file: path.clone(),
                range: LineRange {
                    start: start as u32,
                    end: end as u32,
                },
            },
            score: if content { CONTENT } else { 0 } + context,
            same_file: !content_only,
        });
    };
    for start in 0..lines {
        if start + n <= lines {
            let range = LineRange {
                start: start as u32,
                end: (start + n) as u32,
            };
            let content = match recorded {
                Some(l) => text.hashes()[start..start + n] == *l,
                None => text.norm_hash(range) == Some(anchor.content_hash_norm),
            };
            let score = context.score(start, start + n);
            if content || (!content_only && score >= MIN_CONTEXT) {
                push(start, start + n, content, score);
            }
        }
        if content_only || context.best_score(start) < MIN_CONTEXT {
            continue;
        }
        for len in (1..=(2 * n + GROWTH).min(lines - start)).filter(|&l| l != n) {
            let score = context.score(start, start + len);
            if score >= MIN_CONTEXT {
                push(start, start + len, false, score);
            }
        }
    }
}

/// Anchor context lines found around each line of a text. An anchor at the
/// start or end of its file only matches candidates there.
struct ContextHits {
    /// Leading context lines found before line `i`, for `i` in `0..=lines`.
    lead: Vec<u32>,
    /// Trailing context lines found from line `i` on.
    trail: Vec<u32>,
    /// Real (existing) lines in the anchor's context.
    real: u32,
    trail_real: u32,
    at_start: bool,
    end_only: Option<usize>,
}

impl ContextHits {
    fn new(context: &[Hash], hashes: &[Hash]) -> Self {
        let (lead_ctx, trail_ctx) = context.split_at(CONTEXT.min(context.len()));
        let real = |c: &[Hash]| c.iter().filter(|&&h| h != NO_LINE).count() as u32;
        let hit = |want: Hash, at: Option<usize>| want != NO_LINE && at.and_then(|i| hashes.get(i)) == Some(&want);
        let lead = (0..=hashes.len())
            .map(|i| {
                (lead_ctx.iter().enumerate())
                    .filter(|&(k, &h)| hit(h, i.checked_sub(lead_ctx.len() - k)))
                    .count() as u32
            })
            .collect();
        let trail = (0..=hashes.len())
            .map(|i| {
                (trail_ctx.iter().enumerate())
                    .filter(|&(k, &h)| hit(h, Some(i + k)))
                    .count() as u32
            })
            .collect();
        Self {
            lead,
            trail,
            real: real(lead_ctx) + real(trail_ctx),
            trail_real: real(trail_ctx),
            at_start: lead_ctx.last() == Some(&NO_LINE),
            end_only: (trail_ctx.first() == Some(&NO_LINE)).then_some(hashes.len()),
        }
    }

    /// Percentage of the anchor's real context lines found at the same position
    /// around `start..end`.
    fn score(&self, start: usize, end: usize) -> u32 {
        if self.end_only.is_some_and(|n| end != n) {
            return 0;
        }
        self.percent(start, self.lead[start] + self.trail[end])
    }

    /// Upper bound of `score(start, _)`.
    fn best_score(&self, start: usize) -> u32 {
        self.percent(start, self.lead[start] + self.trail_real)
    }

    fn percent(&self, start: usize, hits: u32) -> u32 {
        if self.at_start && start != 0 {
            return 0;
        }
        (hits * 100).checked_div(self.real).unwrap_or(0)
    }
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
        state: AnchorState::Inferred { strategy, confidence },
        location: Some(best.location.clone()),
    }
}
