//! Completion and per-hunk review state relative to a `ReviewFilter` (SPEC §5.2).

use std::collections::BTreeMap;

use crate::content::{FileDiff, LineRange, NonTextChange, RepoPath};
use crate::filter::ReviewFilter;
use crate::review::{ReviewedSpan, Side, SpanState};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Completion {
    /// Changed lines passing the filter, counted on both sides.
    pub lines: u32,
    /// Of those, covered by a `Seen` span.
    pub seen_lines: u32,
    /// Of those, covered only by `SeenAgainstOldBase` spans; they count as unseen.
    pub old_base_lines: u32,
    pub items: u32,
    pub acknowledged_items: u32,
    pub excluded_lines: u32,
    pub excluded_items: u32,
}

impl Completion {
    pub fn is_complete(&self) -> bool {
        self.seen_lines == self.lines && self.acknowledged_items == self.items
    }
}

/// Display state of one hunk, derived from spans.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HunkState {
    Unseen,
    /// Every changed line is covered by a `Seen` span.
    Seen,
    /// Every changed line is covered, some only by `SeenAgainstOldBase` spans.
    SeenAgainstOldBase,
}

type Ranges<'a> = BTreeMap<(&'a RepoPath, Side), Vec<LineRange>>;

/// The spans' line ranges per file and side, ready for coverage queries. Spans
/// must already be remapped into the current changeset.
pub struct Coverage<'a> {
    /// `Seen` spans only.
    seen: Ranges<'a>,
    /// `Seen` and `SeenAgainstOldBase` spans.
    any: Ranges<'a>,
}

impl<'a> Coverage<'a> {
    pub fn new(spans: &'a [ReviewedSpan]) -> Self {
        let mut seen = Ranges::new();
        let mut any = Ranges::new();
        for s in spans {
            for a in s.base.iter().chain(&s.target) {
                let key = (&a.file, a.side);
                any.entry(key).or_default().push(a.range);
                if s.state == SpanState::Seen {
                    seen.entry(key).or_default().push(a.range);
                }
            }
        }
        for v in seen.values_mut().chain(any.values_mut()) {
            v.sort_by_key(|r| r.start);
        }
        Self { seen, any }
    }

    /// Lines of `range` covered by `Seen` spans, and by any span.
    fn lines(&self, file: &RepoPath, side: Side, range: LineRange) -> (u32, u32) {
        let count = |m: &Ranges| m.get(&(file, side)).map_or(0, |v| covered(v, range));
        (count(&self.seen), count(&self.any))
    }

    pub fn hunk_states(&self, diff: &FileDiff) -> Vec<HunkState> {
        diff.hunks
            .iter()
            .map(|h| {
                let (mut total, mut seen, mut any) = (0, 0, 0);
                for (side, file, range) in diff.sides(h) {
                    let Some(file) = file else { continue };
                    let (s, a) = self.lines(file, side, range);
                    total += range.len();
                    seen += s;
                    any += a;
                }
                match (total > 0, seen == total, any == total) {
                    (true, true, _) => HunkState::Seen,
                    (true, false, true) => HunkState::SeenAgainstOldBase,
                    _ => HunkState::Unseen,
                }
            })
            .collect()
    }

    /// `acknowledged` holds the acknowledged non-text changes.
    pub fn completion(
        &self,
        filter: &ReviewFilter,
        diffs: &[FileDiff],
        non_text: &[NonTextChange],
        acknowledged: &[NonTextChange],
    ) -> Completion {
        let compiled = filter.compile();
        let mut c = Completion {
            lines: 0,
            seen_lines: 0,
            old_base_lines: 0,
            items: 0,
            acknowledged_items: 0,
            excluded_lines: 0,
            excluded_items: 0,
        };
        for d in diffs {
            let passes = compiled.passes(d.path());
            for h in &d.hunks {
                for (side, file, range) in d.sides(h) {
                    let Some(file) = file else { continue };
                    if !passes {
                        c.excluded_lines += range.len();
                        continue;
                    }
                    let (seen, any) = self.lines(file, side, range);
                    c.lines += range.len();
                    c.seen_lines += seen;
                    c.old_base_lines += any - seen;
                }
            }
        }
        for n in non_text {
            if !compiled.shows(n) {
                c.excluded_items += 1;
            } else {
                c.items += 1;
                c.acknowledged_items += acknowledged.contains(n) as u32;
            }
        }
        c
    }
}

/// Spans must already be remapped into the current changeset. Only `Seen` spans
/// count towards `seen_lines`; `acknowledged` holds the acknowledged non-text changes.
pub fn completion(
    filter: &ReviewFilter,
    diffs: &[FileDiff],
    non_text: &[NonTextChange],
    spans: &[ReviewedSpan],
    acknowledged: &[NonTextChange],
) -> Completion {
    Coverage::new(spans).completion(filter, diffs, non_text, acknowledged)
}

/// Lines of `r` inside `sorted` ranges, which may overlap.
fn covered(sorted: &[LineRange], r: LineRange) -> u32 {
    let mut total = 0;
    let mut from = r.start;
    for s in sorted {
        let start = s.start.max(from);
        let end = s.end.min(r.end);
        if start < end {
            total += end - start;
            from = end;
        }
    }
    total
}
