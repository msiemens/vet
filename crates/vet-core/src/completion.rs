//! Completion and per-hunk review state relative to a compiled `ReviewFilter` (SPEC §5.2).

use std::collections::BTreeMap;
use std::iter::Sum;
use std::ops::{Add, Sub};

use crate::content::{FileDiff, LineRange, NonTextChange, RepoPath};
use crate::filter::CompiledFilter;
use crate::review::{ReviewedSpan, Side, SpanState, sides};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Completion {
    /// Changed lines passing the filter, counted on both sides.
    pub lines: u32,
    /// Of those, covered by a `Seen` span.
    pub seen_lines: u32,
    /// Of those, covered only by `SeenAgainstOldBase` spans; they count as unseen.
    pub old_base_lines: u32,
    pub items: u32,
    pub acknowledged_items: u32,
    /// Text hunks of the files passing the filter.
    pub hunks: u32,
    /// Of those, in state `Seen`.
    pub seen_hunks: u32,
    /// Of those, flagged for later review.
    pub flagged_hunks: u32,
    pub excluded_lines: u32,
    pub excluded_items: u32,
}

impl Completion {
    pub fn is_complete(&self) -> bool {
        self.seen_lines == self.lines && self.acknowledged_items == self.items
    }

    /// Share of the changed lines and non-text items that are seen, in percent; 0 with nothing to review.
    pub fn seen_percent(&self) -> f32 {
        self.percent(self.seen_lines + self.acknowledged_items)
    }

    /// Share of the changed lines and non-text items that are seen against an old base only, in percent.
    pub fn old_base_percent(&self) -> f32 {
        self.percent(self.old_base_lines)
    }

    fn percent(&self, part: u32) -> f32 {
        let total = self.lines + self.items;
        if total == 0 {
            0.0
        } else {
            part as f32 / total as f32 * 100.0
        }
    }

    fn zip(self, other: Self, op: impl Fn(u32, u32) -> u32) -> Self {
        Self {
            lines: op(self.lines, other.lines),
            seen_lines: op(self.seen_lines, other.seen_lines),
            old_base_lines: op(self.old_base_lines, other.old_base_lines),
            items: op(self.items, other.items),
            acknowledged_items: op(self.acknowledged_items, other.acknowledged_items),
            hunks: op(self.hunks, other.hunks),
            seen_hunks: op(self.seen_hunks, other.seen_hunks),
            flagged_hunks: op(self.flagged_hunks, other.flagged_hunks),
            excluded_lines: op(self.excluded_lines, other.excluded_lines),
            excluded_items: op(self.excluded_items, other.excluded_items),
        }
    }
}

/// The completion of two disjoint parts of a changeset together.
impl Add for Completion {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        self.zip(other, |a, b| a + b)
    }
}

/// The completion of a changeset without one of its parts.
impl Sub for Completion {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self.zip(other, |a, b| a - b)
    }
}

impl Sum for Completion {
    fn sum<I: Iterator<Item = Self>>(parts: I) -> Self {
        parts.fold(Self::default(), Add::add)
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
    /// Only an interdiff has these: nothing is left to review in the hunk (the author took a
    /// change back to the base content), so there is nothing to mark.
    Informational,
    /// Only an interdiff has these: unseen, and the old version's patch did not apply to the new
    /// base there, so the old side shows its lines unrebased.
    Unapplied,
}

type Ranges<'a> = BTreeMap<(&'a RepoPath, Side), Vec<LineRange>>;

/// The spans' line ranges per file and side, ready for coverage queries. Spans
/// must already be remapped into the current changeset.
pub struct Coverage<'a> {
    /// `Seen` spans only.
    seen: Ranges<'a>,
    /// `Seen` and `SeenAgainstOldBase` spans.
    any: Ranges<'a>,
    /// `Flagged` spans, which cover nothing.
    flagged: Ranges<'a>,
}

impl<'a> Coverage<'a> {
    pub fn new(spans: impl IntoIterator<Item = &'a ReviewedSpan>) -> Self {
        let mut seen = Ranges::new();
        let mut any = Ranges::new();
        let mut flagged = Ranges::new();
        for s in spans {
            for a in s.base.iter().chain(&s.target) {
                let key = (&a.file, a.side);
                match s.state {
                    SpanState::Flagged => flagged.entry(key).or_default().push(a.range),
                    SpanState::Seen => {
                        any.entry(key).or_default().push(a.range);
                        seen.entry(key).or_default().push(a.range);
                    }
                    SpanState::SeenAgainstOldBase => any.entry(key).or_default().push(a.range),
                }
            }
        }
        for v in seen.values_mut().chain(any.values_mut()).chain(flagged.values_mut()) {
            v.sort_by_key(|r| r.start);
        }
        Self { seen, any, flagged }
    }

    /// Whether any of the lines `regions` name is flagged, as of a hunk that covers exactly them.
    pub fn flagged_in<'r>(&self, regions: impl IntoIterator<Item = (&'r RepoPath, Side, LineRange)>) -> bool {
        regions
            .into_iter()
            .any(|(file, side, range)| self.flagged.get(&(file, side)).is_some_and(|v| covered(v, range) > 0))
    }

    /// Per hunk, whether it has a flagged line.
    pub fn hunk_flags(&self, diff: &FileDiff) -> Vec<bool> {
        diff.hunks
            .iter()
            .map(|h| {
                self.flagged_in(
                    sides(diff, h)
                        .into_iter()
                        .filter_map(|(side, file, range)| file.map(|f| (f, side, range))),
                )
            })
            .collect()
    }

    /// Lines of `range` covered by `Seen` spans, and by any span.
    pub fn lines(&self, file: &RepoPath, side: Side, range: LineRange) -> (u32, u32) {
        let count = |m: &Ranges| m.get(&(file, side)).map_or(0, |v| covered(v, range));
        (count(&self.seen), count(&self.any))
    }

    pub fn hunk_states(&self, diff: &FileDiff) -> Vec<HunkState> {
        diff.hunks
            .iter()
            .map(|h| {
                self.state_of(
                    sides(diff, h)
                        .into_iter()
                        .filter_map(|(side, file, range)| file.map(|f| (f, side, range))),
                )
            })
            .collect()
    }

    /// Per hunk, the changed lines covered only by `SeenAgainstOldBase` spans.
    pub fn old_base_lines(&self, diff: &FileDiff) -> Vec<u32> {
        diff.hunks
            .iter()
            .map(|h| {
                (sides(diff, h).into_iter())
                    .filter_map(|(side, file, range)| file.map(|f| (f, side, range)))
                    .map(|(f, side, range)| {
                        let (seen, any) = self.lines(f, side, range);
                        any - seen
                    })
                    .sum()
            })
            .collect()
    }

    /// The state of the lines `regions` name, as of a hunk that covers exactly them.
    pub fn state_of<'r>(&self, regions: impl IntoIterator<Item = (&'r RepoPath, Side, LineRange)>) -> HunkState {
        let (mut total, mut seen, mut any) = (0, 0, 0);
        for (file, side, range) in regions {
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
    }

    /// `acknowledged` holds the acknowledged non-text changes.
    pub fn completion<'d>(
        &self,
        compiled: &CompiledFilter,
        diffs: impl IntoIterator<Item = &'d FileDiff>,
        non_text: &[NonTextChange],
        acknowledged: &[NonTextChange],
    ) -> Completion {
        let mut c = Completion::default();
        for d in diffs {
            let passes = compiled.passes(d.path());
            if passes {
                let states = self.hunk_states(d);
                c.hunks += states.len() as u32;
                c.seen_hunks += states.iter().filter(|&&s| s == HunkState::Seen).count() as u32;
                c.flagged_hunks += self.hunk_flags(d).into_iter().filter(|&f| f).count() as u32;
            }
            for h in &d.hunks {
                for (side, file, range) in sides(d, h) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn completion(lines: u32, seen_lines: u32, old_base_lines: u32, items: u32, acknowledged_items: u32) -> Completion {
        Completion {
            lines,
            seen_lines,
            old_base_lines,
            items,
            acknowledged_items,
            hunks: 0,
            seen_hunks: 0,
            flagged_hunks: 0,
            excluded_lines: 0,
            excluded_items: 0,
        }
    }

    #[test]
    fn lines_and_items_count_alike_and_an_old_base_is_a_share_of_the_same_total() {
        let c = completion(6, 3, 1, 2, 1);
        assert_eq!((c.seen_percent(), c.old_base_percent()), (50.0, 12.5));
    }

    #[test]
    fn nothing_to_review_is_zero_percent() {
        let c = completion(0, 0, 0, 0, 0);
        assert_eq!((c.seen_percent(), c.old_base_percent()), (0.0, 0.0));
    }
}
