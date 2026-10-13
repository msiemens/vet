//! Text search in a file: which lines of a diff's two sides a query is looked for in, and the lines
//! that match. How a line is matched is a port (`LineMatcher`): the adapter brings the regular
//! expression engine.
//!
//! Lines are the diff's: a context line is on both sides and counts once; a hunk's removed lines come
//! before its added ones, as a unified diff draws them. Multi-line patterns are not a thing: a line is
//! matched on its own.

use std::ops::Range;

use vet_core::completion::HunkState;
use vet_core::content::Hunk;
use vet_core::text::Units;

/// What is looked for, and how.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SearchPattern {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// Matches one line of text.
pub trait LineMatcher: Send + Sync {
    /// The non-empty matches in `line`, in order, as byte ranges.
    fn hits(&self, line: &str) -> Vec<Range<usize>>;
}

/// The sides of a diff to look in; a context line is on both.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchSide {
    Both,
    Target,
    Base,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchFilters {
    pub side: SearchSide,
    /// Only the lines the change added or removed.
    pub only_changes: bool,
    /// Only changed lines in hunks that are not seen: unseen, seen against an old base, or an
    /// interdiff hunk the old patch did not apply to.
    pub not_seen: bool,
}

/// What a text is: a diff of two texts, or the target's text alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sides {
    Both,
    TargetOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// A line the query matched.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FoundLine {
    pub kind: LineKind,
    /// The 0-based line: of the target text for a context or added line, of the base text for a
    /// removed one.
    pub line: u32,
    /// The hunk of an added or removed line.
    pub hunk: Option<u32>,
    /// The line, or the part of a long one around its first match.
    pub text: String,
    /// Where `text` starts in the line, in UTF-16 units.
    pub text_start: u32,
    /// The matches as columns of the line, in UTF-16 units, in order.
    pub hits: Vec<(u32, u32)>,
}

/// The most characters of a line that a result carries.
const MAX_TEXT: usize = 400;
/// Characters kept before the first match of a line that is cut.
const LEAD: usize = 60;

/// The text split into lines the way hunks count them: at `\n`, a final break starting one more
/// line, and a `\r` before the break not part of the line.
fn lines_of(text: &str) -> Vec<&str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect()
}

/// Whether a line of this kind in a hunk in this state is looked at.
fn looked_at(kind: LineKind, state: Option<HunkState>, filters: SearchFilters, sides: Sides) -> bool {
    if kind == LineKind::Context && (filters.only_changes || filters.not_seen) {
        return false;
    }
    if kind != LineKind::Context
        && filters.not_seen
        && matches!(state, Some(HunkState::Seen | HunkState::Informational))
    {
        return false;
    }
    match (sides, filters.side, kind) {
        (Sides::TargetOnly, _, LineKind::Removed) => false,
        (Sides::TargetOnly, _, _) => true,
        (Sides::Both, SearchSide::Target, LineKind::Removed) => false,
        (Sides::Both, SearchSide::Base, LineKind::Added) => false,
        _ => true,
    }
}

/// The matching lines of a file, in the unified diff's order. `states` has the review state of each
/// hunk, as far as it is known.
pub fn search_text(
    base: &str,
    target: &str,
    hunks: &[Hunk],
    states: &[HunkState],
    matcher: &dyn LineMatcher,
    filters: SearchFilters,
    sides: Sides,
) -> Vec<FoundLine> {
    let target_lines = lines_of(target);
    let base_lines = if sides == Sides::Both {
        lines_of(base)
    } else {
        Vec::new()
    };
    let mut found = Vec::new();
    let mut visit = |kind: LineKind, text: Option<&str>, line: usize, hunk: Option<usize>| {
        let state = hunk.and_then(|h| states.get(h)).copied();
        let Some(text) = text else { return };
        if !looked_at(kind, state, filters, sides) {
            return;
        }
        let hits = matcher.hits(text);
        if let Some(first) = hits.first() {
            found.push(found_line(
                kind,
                line as u32,
                hunk.map(|h| h as u32),
                text,
                first,
                &hits,
            ));
        }
    };
    let mut t = 0usize;
    for (i, h) in hunks.iter().enumerate() {
        let new = h.new.start as usize;
        while t < new {
            visit(LineKind::Context, target_lines.get(t).copied(), t, None);
            t += 1;
        }
        if sides == Sides::Both {
            for line in h.old.start as usize..h.old.end as usize {
                visit(LineKind::Removed, base_lines.get(line).copied(), line, Some(i));
            }
        }
        for line in new..h.new.end as usize {
            visit(LineKind::Added, target_lines.get(line).copied(), line, Some(i));
        }
        t = h.new.end as usize;
    }
    while t < target_lines.len() {
        visit(LineKind::Context, target_lines.get(t).copied(), t, None);
        t += 1;
    }
    found
}

/// The result for one matching line: its text cut to a window round the first match when it is long.
fn found_line(
    kind: LineKind,
    line: u32,
    hunk: Option<u32>,
    text: &str,
    first: &Range<usize>,
    hits: &[Range<usize>],
) -> FoundLine {
    let mut units = Units::new(text);
    let columns: Vec<(u32, u32)> = hits.iter().map(|h| (units.at(h.start), units.at(h.end))).collect();
    let (start_byte, end_byte) = if text.chars().count() <= MAX_TEXT {
        (0, text.len())
    } else {
        let lead = text[..first.start]
            .char_indices()
            .rev()
            .nth(LEAD - 1)
            .map_or(0, |(i, _)| i);
        let end = text[lead..]
            .char_indices()
            .nth(MAX_TEXT)
            .map_or(text.len(), |(i, _)| lead + i);
        (lead, end)
    };
    let (start, end) = (units.at(start_byte), units.at(end_byte));
    FoundLine {
        kind,
        line,
        hunk,
        text: text[start_byte..end_byte].to_owned(),
        text_start: start,
        hits: columns
            .into_iter()
            .filter(|&(from, to)| from >= start && to <= end)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::LineRange;

    use super::*;

    /// Finds every occurrence of a substring.
    struct Substring(&'static str);

    impl LineMatcher for Substring {
        fn hits(&self, line: &str) -> Vec<Range<usize>> {
            line.match_indices(self.0).map(|(i, m)| i..i + m.len()).collect()
        }
    }

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

    const ALL: SearchFilters = SearchFilters {
        side: SearchSide::Both,
        only_changes: false,
        not_seen: false,
    };

    /// Line 2 of the base is replaced by two lines; "delay" is also in context lines 0 and 4.
    fn file() -> (&'static str, &'static str, Vec<Hunk>) {
        (
            "delay a\nkeep\nold delay\nkeep\ndelay z",
            "delay a\nkeep\nnew delay\nnext delay\nkeep\ndelay z",
            vec![hunk((2, 3), (2, 4))],
        )
    }

    fn shape(found: &[FoundLine]) -> Vec<(LineKind, u32)> {
        found.iter().map(|f| (f.kind, f.line)).collect()
    }

    fn run(filters: SearchFilters, sides: Sides, states: &[HunkState]) -> Vec<FoundLine> {
        let (base, target, hunks) = file();
        search_text(base, target, &hunks, states, &Substring("delay"), filters, sides)
    }

    #[test]
    fn a_diff_lists_context_once_and_a_hunks_removed_lines_before_its_added_ones() {
        use LineKind::*;
        let found = run(ALL, Sides::Both, &[HunkState::Unseen]);
        assert_eq!(
            shape(&found),
            [(Context, 0), (Removed, 2), (Added, 2), (Added, 3), (Context, 5)]
        );
        assert_eq!(found[1].hunk, Some(0));
        assert_eq!(found[0].hunk, None);
    }

    #[test]
    fn the_side_narrows_the_search_and_a_context_line_stays_on_both() {
        use LineKind::*;
        let target = SearchFilters {
            side: SearchSide::Target,
            ..ALL
        };
        assert_eq!(
            shape(&run(target, Sides::Both, &[])),
            [(Context, 0), (Added, 2), (Added, 3), (Context, 5)]
        );
        let base = SearchFilters {
            side: SearchSide::Base,
            ..ALL
        };
        assert_eq!(
            shape(&run(base, Sides::Both, &[])),
            [(Context, 0), (Removed, 2), (Context, 5)]
        );
    }

    #[test]
    fn only_changes_drops_the_context_and_the_target_alone_has_no_removed_lines() {
        use LineKind::*;
        let only = SearchFilters {
            only_changes: true,
            ..ALL
        };
        assert_eq!(
            shape(&run(only, Sides::Both, &[])),
            [(Removed, 2), (Added, 2), (Added, 3)]
        );
        assert_eq!(shape(&run(only, Sides::TargetOnly, &[])), [(Added, 2), (Added, 3)]);
        // A target's text ignores the side asked for.
        let base = SearchFilters {
            side: SearchSide::Base,
            ..ALL
        };
        assert_eq!(
            shape(&run(base, Sides::TargetOnly, &[])),
            [(Context, 0), (Added, 2), (Added, 3), (Context, 5)]
        );
    }

    #[test]
    fn not_seen_keeps_changed_lines_of_hunks_that_are_not_seen() {
        let not_seen = SearchFilters { not_seen: true, ..ALL };
        assert_eq!(run(not_seen, Sides::Both, &[HunkState::Unseen]).len(), 3);
        assert_eq!(run(not_seen, Sides::Both, &[HunkState::SeenAgainstOldBase]).len(), 3);
        assert_eq!(run(not_seen, Sides::Both, &[HunkState::Seen]).len(), 0);
    }

    #[test]
    fn columns_are_utf16_units_of_the_line() {
        let found = search_text(
            "",
            "é𝄞 delay delay\r\n",
            &[],
            &[],
            &Substring("delay"),
            ALL,
            Sides::TargetOnly,
        );
        // 'é' is one unit and the clef two: "delay" starts at unit 4.
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].hits, [(4, 9), (10, 15)]);
        assert_eq!(found[0].text, "é𝄞 delay delay");
    }

    #[test]
    fn a_long_line_is_cut_round_its_first_match() {
        let line = format!("{}delay{}", "x".repeat(500), "y".repeat(500));
        let found = search_text("", &line, &[], &[], &Substring("delay"), ALL, Sides::TargetOnly);
        let f = &found[0];
        assert_eq!(f.text.chars().count(), MAX_TEXT);
        assert_eq!(f.text_start, 500 - LEAD as u32);
        assert_eq!(f.hits, [(500, 505)]);
        assert_eq!(&f.text[LEAD..LEAD + 5], "delay");
    }
}
