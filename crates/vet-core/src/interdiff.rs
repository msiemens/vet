//! The interdiff of one file: what the author changed between two versions of a changeset, with
//! what a move of the base explains left out (design-changesets §3.2, §2.4).
//!
//! Version A is rebased onto B's base with [`merge3`], and the result is diffed against B. A
//! change the base move brought is in both and vanishes.

use crate::content::{FileDiff, Hunk, LineRange, RepoPath, SnapshotId};
use crate::hunks::{changed_lines, line_hunks};
use crate::marking::Region;
use crate::merge::{LineOrigin, merge3};
use crate::review::{Anchor, ReviewedSpan, Side, SpanState};
use crate::text::{Text, diff_map};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct HunkFlags {
    /// Nothing is left to review in the hunk: it neither adds a line of the newer version's
    /// changeset nor removes a line it deletes from the base, as when the author reverted a
    /// change back to the base content.
    pub informational: bool,
    /// A's patch did not apply to the new base here, so the old side shows A's lines unrebased.
    pub conflict: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InterdiffHunk {
    /// `old` in [`FileInterdiff::old`], `new` in B's target.
    pub hunk: Hunk,
    pub flags: HunkFlags,
    /// The lines of B's target the hunk adds that B's changeset adds, ascending.
    pub target_lines: Vec<LineRange>,
    /// The lines of B's base the hunk removes from the old side that B's changeset deletes,
    /// ascending.
    pub base_lines: Vec<LineRange>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileInterdiff {
    /// A's target rebased onto B's base: the old side of the interdiff.
    pub old: Vec<u8>,
    pub hunks: Vec<InterdiffHunk>,
}

/// The interdiff of a file whose content in version A is `base_a` and `target_a`, and in version
/// B `base_b` and `target_b`. A file a side does not have is empty.
pub fn interdiff(base_a: &[u8], target_a: &[u8], base_b: &[u8], target_b: &[u8]) -> FileInterdiff {
    let merged = merge3(base_a, target_a, base_b);
    let hunks = line_hunks(&merged.text, target_b);

    let squash = changed_lines(base_b, target_b);
    let old_text = Text::new(&merged.text);
    let base_text = Text::new(base_b);
    let to_base = diff_map(&old_text.raw, &base_text.raw);

    let hunks = hunks
        .into_iter()
        .map(|hunk| {
            let target_lines: Vec<_> = squash
                .iter()
                .map(|s| LineRange {
                    start: s.new.start.max(hunk.new.start),
                    end: s.new.end.min(hunk.new.end),
                })
                .filter(|r| !r.is_empty())
                .collect();
            // Only lines that came from the base can be lines B's changeset deletes.
            let from_base = (hunk.old.start..hunk.old.end)
                .filter(|&l| merged.origin[l as usize] != LineOrigin::Ours)
                .filter_map(|l| to_base[l as usize])
                .filter(|&b| {
                    squash
                        .iter()
                        .any(|s| s.old.start as usize <= b && b < s.old.end as usize)
                });
            let base_lines = runs(from_base.map(|b| b as u32));
            let conflict = merged
                .conflicts
                .iter()
                .any(|&c| hunk.old.intersects(c) || (c.is_empty() && hunk.old.contains(c)));
            InterdiffHunk {
                hunk,
                flags: HunkFlags {
                    informational: target_lines.is_empty() && base_lines.is_empty(),
                    conflict,
                },
                target_lines,
                base_lines,
            }
        })
        .collect();
    FileInterdiff {
        old: merged.text,
        hunks,
    }
}

/// What marking interdiff hunks seen records in B's changeset (design-changesets §2.4): for each
/// of `hunks`, the lines of `squash` (B's own diff of the file) it covers, and a `Seen` span for
/// them per squash hunk they fall in, two-sided where both sides have one range there. `texts`
/// are the base and target text of the file in the snapshots `snapshots` (B's base and target),
/// each split into lines and hashed once. `None` if a text is missing or the squash lacks the
/// file on a side a hunk has lines on.
pub fn squash_marks(
    squash: &FileDiff,
    hunks: &[&InterdiffHunk],
    snapshots: (SnapshotId, SnapshotId),
    texts: (Option<&[u8]>, Option<&[u8]>),
) -> Option<(Vec<Region>, Vec<ReviewedSpan>)> {
    let clip = |lines: &[LineRange], within: LineRange| -> Vec<LineRange> {
        let clipped = lines.iter().map(|r| LineRange {
            start: r.start.max(within.start),
            end: r.end.min(within.end),
        });
        clipped.filter(|r| !r.is_empty()).collect()
    };
    let texts = (texts.0.map(Text::new), texts.1.map(Text::new));
    let anchor = |side, file: &Option<RepoPath>, snapshot, text: Option<&Text>, range| {
        text?.anchor(side, snapshot, file.clone()?, range)
    };
    let (mut regions, mut spans) = (Vec::new(), Vec::new());
    for (hunk, h) in hunks
        .iter()
        .flat_map(|hunk| squash.hunks.iter().map(move |h| (hunk, h)))
    {
        let target = clip(&hunk.target_lines, h.new);
        let base = clip(&hunk.base_lines, h.old);
        for &range in &target {
            regions.push(Region {
                file: squash.new_path.clone()?,
                side: Side::Target,
                range,
            });
        }
        for &range in &base {
            regions.push(Region {
                file: squash.old_path.clone()?,
                side: Side::Base,
                range,
            });
        }
        let span = |base: Option<Anchor>, target: Option<Anchor>| ReviewedSpan {
            base,
            target,
            state: SpanState::Seen,
        };
        let on_target = |range| anchor(Side::Target, &squash.new_path, snapshots.1, texts.1.as_ref(), range);
        let on_base = |range| anchor(Side::Base, &squash.old_path, snapshots.0, texts.0.as_ref(), range);
        if let ([t], [b]) = (&target[..], &base[..]) {
            spans.push(span(Some(on_base(*b)?), Some(on_target(*t)?)));
        } else {
            for &range in &base {
                spans.push(span(Some(on_base(range)?), None));
            }
            for &range in &target {
                spans.push(span(None, Some(on_target(range)?)));
            }
        }
    }
    Some((regions, spans))
}

/// Ascending line numbers as the ranges of consecutive ones.
fn runs(lines: impl IntoIterator<Item = u32>) -> Vec<LineRange> {
    let mut out: Vec<LineRange> = Vec::new();
    for l in lines {
        match out.last_mut() {
            Some(r) if r.end == l => r.end += 1,
            _ => out.push(LineRange { start: l, end: l + 1 }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(prefix: &str, n: usize) -> String {
        (0..n).map(|i| format!("{prefix}{i}\n")).collect()
    }

    fn run(base_a: &str, target_a: &str, base_b: &str, target_b: &str) -> FileInterdiff {
        interdiff(
            base_a.as_bytes(),
            target_a.as_bytes(),
            base_b.as_bytes(),
            target_b.as_bytes(),
        )
    }

    #[test]
    fn the_marks_of_many_hunks_split_each_text_once_and_equal_those_of_each_hunk_alone() {
        use crate::content::Hash;
        use crate::text::SPLITS;

        let base = lines("l", 300);
        let target = base.replace("5\n", "5 edited\n");
        // Version A changed nothing, so every hunk of the interdiff is one of B's changeset.
        let inter = run(&base, &base, &base, &target);
        let squash = FileDiff {
            old_path: Some(RepoPath::from("a")),
            new_path: Some(RepoPath::from("a")),
            old_blob: None,
            new_blob: None,
            hunks: changed_lines(base.as_bytes(), target.as_bytes()),
        };
        let snapshots = (SnapshotId(Hash([1; 32])), SnapshotId(Hash([2; 32])));
        let texts = (Some(base.as_bytes()), Some(target.as_bytes()));
        let hunks: Vec<&InterdiffHunk> = inter.hunks.iter().collect();
        assert!(hunks.len() >= 10);

        let before = SPLITS.with(|n| n.get());
        let (regions, spans) = squash_marks(&squash, &hunks, snapshots, texts).unwrap();
        assert_eq!(SPLITS.with(|n| n.get()) - before, 2);

        let (mut alone_regions, mut alone_spans) = (Vec::new(), Vec::new());
        for hunk in &hunks {
            let (r, s) = squash_marks(&squash, &[hunk], snapshots, texts).unwrap();
            alone_regions.extend(r);
            alone_spans.extend(s);
        }
        assert_eq!((regions, spans), (alone_regions, alone_spans));
    }

    #[test]
    fn what_a_base_move_brought_vanishes() {
        // The author's change is at the bottom; the rebase brought an upstream edit at the top.
        let base_a = lines("l", 20);
        let target_a = base_a.replace("l15\n", "mine\n");
        let base_b = base_a.replace("l2\n", "upstream\n");
        let target_b = base_b.replace("l15\n", "mine\n");
        let result = run(&base_a, &target_a, &base_b, &target_b);
        assert!(result.hunks.is_empty());
        assert_eq!(String::from_utf8(result.old).unwrap(), target_b);
    }

    #[test]
    fn a_change_the_author_made_between_versions_survives() {
        let base_a = lines("l", 20);
        let target_a = base_a.replace("l15\n", "mine\n");
        let base_b = base_a.replace("l2\n", "upstream\n");
        let target_b = base_b.replace("l15\n", "mine, better\n");
        let result = run(&base_a, &target_a, &base_b, &target_b);
        assert_eq!(result.hunks.len(), 1);
        let h = &result.hunks[0];
        assert_eq!((h.hunk.old.start, h.hunk.new.start), (15, 15));
        assert_eq!(h.flags, HunkFlags::default());
        assert_eq!(h.target_lines, [LineRange { start: 15, end: 16 }]);
        assert!(h.base_lines.is_empty());
    }

    #[test]
    fn the_same_base_gives_the_plain_diff() {
        let base = lines("l", 10);
        let result = run(&base, &base.replace("l3\n", "a\n"), &base, &base.replace("l3\n", "b\n"));
        assert_eq!(result.hunks.len(), 1);
        assert_eq!(result.hunks[0].flags, HunkFlags::default());
        assert_eq!(result.hunks[0].target_lines, [LineRange { start: 3, end: 4 }]);
        assert!(
            result.hunks[0].base_lines.is_empty(),
            "A already replaced the base line, so no base line is removed"
        );
    }

    #[test]
    fn a_patch_that_does_not_apply_to_the_new_base_is_flagged_and_keeps_a() {
        let base_a = lines("l", 20);
        let target_a = base_a.replace("l10\n", "mine\n");
        // The upstream edited the same line differently, and the author redid their change.
        let base_b = base_a.replace("l10\n", "upstream\n");
        let target_b = base_b.replace("upstream\n", "mine, redone\n");
        let result = run(&base_a, &target_a, &base_b, &target_b);
        assert_eq!(result.hunks.len(), 1);
        assert!(result.hunks[0].flags.conflict);
        assert_eq!(String::from_utf8(result.old).unwrap(), target_a);
    }

    #[test]
    fn a_hunk_away_from_a_conflict_is_not_flagged() {
        let base_a = lines("l", 30);
        let target_a = base_a.replace("l5\n", "mine\n");
        let base_b = base_a.replace("l5\n", "upstream\n");
        let target_b = base_b.replace("upstream\n", "redone\n").replace("l25\n", "extra\n");
        let result = run(&base_a, &target_a, &base_b, &target_b);
        let flags: Vec<_> = result.hunks.iter().map(|h| h.flags.conflict).collect();
        assert_eq!(flags, [true, false]);
    }

    #[test]
    fn a_revert_to_the_base_is_informational() {
        let base = lines("l", 20);
        let target_a = base.replace("l7\n", "changed\n");
        let result = run(&base, &target_a, &base, &base);
        assert_eq!(result.hunks.len(), 1);
        assert_eq!(
            result.hunks[0].flags,
            HunkFlags {
                informational: true,
                conflict: false
            }
        );
        assert!(result.hunks[0].target_lines.is_empty());
        assert!(result.hunks[0].base_lines.is_empty());
    }

    #[test]
    fn dropping_a_line_the_author_added_is_informational_but_dropping_a_base_line_is_not() {
        let base = lines("l", 20);
        let target_a = base.replace("l7\n", "l7\nadded\n");
        let dropped = interdiff(base.as_bytes(), target_a.as_bytes(), base.as_bytes(), base.as_bytes());
        assert!(dropped.hunks[0].flags.informational);

        let target_b = base.replace("l7\n", "");
        let removed = interdiff(base.as_bytes(), base.as_bytes(), base.as_bytes(), target_b.as_bytes());
        assert!(!removed.hunks[0].flags.informational);
        assert_eq!(removed.hunks[0].base_lines, [LineRange { start: 7, end: 8 }]);
    }
}
