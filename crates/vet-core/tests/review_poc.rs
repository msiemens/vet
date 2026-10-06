//! SPEC §12 POC criterion 1, at the level vet-core can express. Amend and rebase
//! are modeled as base or target content changes.

use std::collections::BTreeMap;

use vet_core::completion::completion;
use vet_core::content::{
    BlobId, Changeset, EntryKind, FileDiff, FileEntry, Hash, Hunk, LineRange, NonTextChange, NonTextKind, RepoPath,
    ReviewFilter, SnapshotId,
};
use vet_core::remap::remap_span;
use vet_core::resolve::resolve_anchor;
use vet_core::review::{
    Anchor, AnchorState, Confidence, FileRemapInput, Location, ReviewedSpan, Side, SpanState,
    StaleReason, Strategy,
};

const RECORDED: SnapshotId = SnapshotId(Hash([1; 32]));
const NOW: Changeset = Changeset {
    base: SnapshotId(Hash([2; 32])),
    target: SnapshotId(Hash([3; 32])),
};

#[derive(Default)]
struct Fix(BTreeMap<BlobId, Vec<u8>>);

impl Fix {
    fn blob(&mut self, text: &str) -> BlobId {
        let id = BlobId::of(text.as_bytes());
        self.0.insert(id, text.as_bytes().to_vec());
        id
    }
}

fn path(s: &str) -> RepoPath {
    RepoPath(s.as_bytes().to_vec())
}

fn range(start: u32, end: u32) -> LineRange {
    LineRange { start, end }
}

/// `n` distinct lines "line 00", "line 01", ...
fn numbered(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("line {i:02}")).collect()
}

fn join(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn anchor(side: Side, file: &str, text: &str, r: LineRange) -> Anchor {
    Anchor::from_text(side, RECORDED, path(file), text.as_bytes(), r).unwrap()
}

fn input(path_: &str, old: BlobId, new: BlobId) -> FileRemapInput {
    FileRemapInput {
        old_path: Some(path(path_)),
        new_path: Some(path(path_)),
        old_blob: Some(old),
        new_blob: Some(new),
    }
}

fn target_span(a: Anchor) -> ReviewedSpan {
    ReviewedSpan {
        base: None,
        target: Some(a),
        state: SpanState::Seen,
    }
}

fn ranges(spans: &[ReviewedSpan], side: Side) -> Vec<(LineRange, SpanState)> {
    spans
        .iter()
        .filter_map(|s| {
            match side {
                Side::Base => s.base.as_ref(),
                Side::Target => s.target.as_ref(),
            }
            .map(|a| (a.range, s.state))
        })
        .collect()
}

#[test]
fn split_seen_region_around_an_edit() {
    let mut fx = Fix::default();
    let old = numbered(40);
    let mut new = old.clone();
    new.splice(19..22, ["edit a".into(), "edit b".into(), "edit c".into()]);
    let (o, n) = (fx.blob(&join(&old)), fx.blob(&join(&new)));
    let span = target_span(anchor(Side::Target, "a.rs", &join(&old), range(10, 30)));

    let out = remap_span(&span, NOW, None, Some(&input("a.rs", o, n)), &fx.0);

    let got = ranges(&out, Side::Target);
    assert_eq!(
        got,
        vec![
            (range(10, 19), SpanState::Seen),
            (range(22, 30), SpanState::Seen)
        ]
    );
    assert_eq!(out[0].target.as_ref().unwrap().snapshot, NOW.target);
}

#[test]
fn revert_restores_coverage() {
    let mut fx = Fix::default();
    let old = numbered(40);
    let mut edited = old.clone();
    edited[15] = "changed".into();
    let (o, e) = (fx.blob(&join(&old)), fx.blob(&join(&edited)));
    let span = target_span(anchor(Side::Target, "a.rs", &join(&old), range(10, 30)));

    let during = remap_span(&span, NOW, None, Some(&input("a.rs", o, e)), &fx.0);
    assert_eq!(ranges(&during, Side::Target).len(), 2);

    let after = remap_span(&span, NOW, None, Some(&input("a.rs", o, o)), &fx.0);
    assert_eq!(
        ranges(&after, Side::Target),
        vec![(range(10, 30), SpanState::Seen)]
    );
}

#[test]
fn lines_inserted_above_shift_the_span() {
    let mut fx = Fix::default();
    let old = numbered(40);
    let mut new = old.clone();
    new.splice(0..0, ["new 1".into(), "new 2".into()]);
    let (o, n) = (fx.blob(&join(&old)), fx.blob(&join(&new)));
    let span = target_span(anchor(Side::Target, "a.rs", &join(&old), range(10, 30)));

    let out = remap_span(&span, NOW, None, Some(&input("a.rs", o, n)), &fx.0);

    assert_eq!(
        ranges(&out, Side::Target),
        vec![(range(12, 32), SpanState::Seen)]
    );
}

#[test]
fn base_change_marks_seen_against_old_base() {
    let mut fx = Fix::default();
    let base_old = numbered(30);
    let mut base_new = base_old.clone();
    base_new[12] = "rebased".into();
    let target: Vec<String> = (0..30).map(|i| format!("target {i:02}")).collect();
    let (bo, bn, t) = (
        fx.blob(&join(&base_old)),
        fx.blob(&join(&base_new)),
        fx.blob(&join(&target)),
    );
    let span = ReviewedSpan {
        base: Some(anchor(Side::Base, "a.rs", &join(&base_old), range(10, 15))),
        target: Some(anchor(Side::Target, "a.rs", &join(&target), range(10, 15))),
        state: SpanState::Seen,
    };

    let out = remap_span(
        &span,
        NOW,
        Some(&input("a.rs", bo, bn)),
        Some(&input("a.rs", t, t)),
        &fx.0,
    );

    assert_eq!(
        ranges(&out, Side::Target),
        vec![(range(10, 15), SpanState::SeenAgainstOldBase)]
    );
    assert!(
        ranges(&out, Side::Base)
            .iter()
            .all(|(r, s)| *s == SpanState::Seen && !r.is_empty())
    );
}

#[test]
fn seen_pure_deletion_survives_unrelated_change() {
    let mut fx = Fix::default();
    let old = numbered(40);
    let mut new = old.clone();
    new[30] = "unrelated".into();
    new.insert(0, "header".into());
    let (o, n) = (fx.blob(&join(&old)), fx.blob(&join(&new)));
    let span = ReviewedSpan {
        base: Some(anchor(Side::Base, "a.rs", &join(&old), range(5, 8))),
        target: None,
        state: SpanState::Seen,
    };

    let out = remap_span(&span, NOW, Some(&input("a.rs", o, n)), None, &fx.0);

    assert_eq!(
        ranges(&out, Side::Base),
        vec![(range(6, 9), SpanState::Seen)]
    );
}

#[test]
fn crlf_only_change_is_whitespace_exact_and_unseen() {
    let mut fx = Fix::default();
    let lf = join(&numbered(10));
    let crlf = lf.replace('\n', "\r\n");
    let (o, n) = (fx.blob(&lf), fx.blob(&crlf));
    let a = anchor(Side::Target, "a.rs", &lf, range(3, 6));

    let r = resolve_anchor(&a, &input("a.rs", o, n), &[], &fx.0);

    assert_eq!(
        r.state,
        AnchorState::Exact {
            whitespace_changed: true
        }
    );
    assert_eq!(
        r.location,
        Some(Location {
            file: path("a.rs"),
            range: range(3, 6)
        })
    );
    assert!(
        remap_span(
            &target_span(a),
            NOW,
            None,
            Some(&input("a.rs", o, n)),
            &fx.0
        )
        .is_empty()
    );
}

#[test]
fn identical_blob_and_diff_mapped_are_exact() {
    let mut fx = Fix::default();
    let old = numbered(20);
    let mut new = old.clone();
    new.insert(0, "x".into());
    let (o, n) = (fx.blob(&join(&old)), fx.blob(&join(&new)));
    let a = anchor(Side::Target, "a.rs", &join(&old), range(5, 8));

    let r = resolve_anchor(&a, &input("a.rs", o, o), &[], &fx.0);
    assert_eq!(
        (r.state, r.location.unwrap().range),
        (
            AnchorState::Exact {
                whitespace_changed: false
            },
            range(5, 8)
        )
    );

    let r = resolve_anchor(&a, &input("a.rs", o, n), &[], &fx.0);
    assert_eq!(
        (r.state, r.location.unwrap().range),
        (
            AnchorState::Exact {
                whitespace_changed: false
            },
            range(6, 9)
        )
    );
}

#[test]
fn rename_is_exact_at_the_new_path() {
    let mut fx = Fix::default();
    let old = numbered(20);
    let mut new = old.clone();
    new[15] = "edited elsewhere".into();
    let (o, n) = (fx.blob(&join(&old)), fx.blob(&join(&new)));
    let a = anchor(Side::Target, "a.rs", &join(&old), range(2, 5));
    let file = FileRemapInput {
        old_path: Some(path("a.rs")),
        new_path: Some(path("b.rs")),
        old_blob: Some(o),
        new_blob: Some(n),
    };

    let r = resolve_anchor(&a, &file, &[], &fx.0);

    assert_eq!(
        r.state,
        AnchorState::Exact {
            whitespace_changed: false
        }
    );
    assert_eq!(
        r.location,
        Some(Location {
            file: path("b.rs"),
            range: range(2, 5)
        })
    );
}

#[test]
fn edited_anchor_range_is_inferred_from_context() {
    let mut fx = Fix::default();
    let old = numbered(30);
    let mut new = old.clone();
    new[11] = "edited".into();
    let (o, n) = (fx.blob(&join(&old)), fx.blob(&join(&new)));
    let a = anchor(Side::Target, "a.rs", &join(&old), range(10, 13));

    let r = resolve_anchor(&a, &input("a.rs", o, n), &[], &fx.0);

    assert_eq!(
        r.state,
        AnchorState::Inferred {
            strategy: Strategy::ContextWindow,
            confidence: Confidence::Medium
        }
    );
    assert_eq!(r.location.unwrap().range, range(10, 13));
}

#[test]
fn duplicate_content_is_ambiguous() {
    let mut fx = Fix::default();
    let old = "a\nb\nneedle\nc\nd\n";
    let new = "w\nneedle\nx\ny\nneedle\nz\n";
    let (o, n) = (fx.blob(old), fx.blob(new));
    let a = anchor(Side::Target, "a.rs", old, range(2, 3));

    let r = resolve_anchor(&a, &input("a.rs", o, n), &[], &fx.0);

    let candidates = vec![
        Location {
            file: path("a.rs"),
            range: range(1, 2),
        },
        Location {
            file: path("a.rs"),
            range: range(4, 5),
        },
    ];
    assert_eq!(r.state, AnchorState::Ambiguous { candidates });
    assert_eq!(r.location, None);
}

#[test]
fn cross_file_move_is_inferred() {
    let mut fx = Fix::default();
    let old = "a\nb\nmoved 1\nmoved 2\nc\nd\n";
    let a_new = "a\nb\nc\nd\n";
    let b_new = "x\nb\nmoved 1\nmoved 2\nc\ny\n";
    let (o, an, bn) = (fx.blob(old), fx.blob(a_new), fx.blob(b_new));
    let a = anchor(Side::Target, "a.rs", old, range(2, 4));
    let changed = [FileRemapInput {
        old_path: None,
        new_path: Some(path("b.rs")),
        old_blob: None,
        new_blob: Some(bn),
    }];

    let r = resolve_anchor(&a, &input("a.rs", o, an), &changed, &fx.0);

    assert_eq!(
        r.state,
        AnchorState::Inferred {
            strategy: Strategy::CrossFileMove,
            confidence: Confidence::Medium
        }
    );
    assert_eq!(
        r.location,
        Some(Location {
            file: path("b.rs"),
            range: range(2, 4)
        })
    );
}

#[test]
fn deleted_file_is_stale() {
    let mut fx = Fix::default();
    let old = join(&numbered(10));
    let o = fx.blob(&old);
    let a = anchor(Side::Target, "a.rs", &old, range(2, 5));
    let file = FileRemapInput {
        old_path: Some(path("a.rs")),
        new_path: None,
        old_blob: Some(o),
        new_blob: None,
    };

    let r = resolve_anchor(&a, &file, &[], &fx.0);

    assert_eq!(
        r.state,
        AnchorState::Stale {
            reason: StaleReason::FileDeleted
        }
    );
}

#[test]
fn rewritten_content_is_stale() {
    let mut fx = Fix::default();
    let old = join(&numbered(10));
    let (o, n) = (fx.blob(&old), fx.blob("completely\ndifferent\n"));
    let a = anchor(Side::Target, "a.rs", &old, range(2, 5));

    let r = resolve_anchor(&a, &input("a.rs", o, n), &[], &fx.0);

    assert_eq!(
        r.state,
        AnchorState::Stale {
            reason: StaleReason::ContextChanged
        }
    );
}

#[test]
fn repeated_line_with_rewritten_context_is_unseen() {
    let mut fx = Fix::default();
    let old = "fn a() {\n    return 0;\n}\nfn b() {\n    return 0;\n}\n";
    let rewritten = "fn c() {\n    return 0;\n}\nfn d() {\n    return 0;\n}\n";
    let (o, n) = (fx.blob(old), fx.blob(rewritten));
    let span = target_span(anchor(Side::Target, "a.rs", old, range(1, 2)));

    assert!(remap_span(&span, NOW, None, Some(&input("a.rs", o, n)), &fx.0).is_empty());

    let unrelated = "fn a() {\n    return 0;\n}\nfn b() {\n    return 1;\n}\n";
    let u = fx.blob(unrelated);
    let out = remap_span(&span, NOW, None, Some(&input("a.rs", o, u)), &fx.0);
    assert_eq!(
        ranges(&out, Side::Target),
        vec![(range(1, 2), SpanState::Seen)]
    );
}

fn diff(path_: &str, hunks: &[(LineRange, LineRange)]) -> FileDiff {
    FileDiff {
        old_path: Some(path(path_)),
        new_path: Some(path(path_)),
        old_blob: None,
        new_blob: None,
        hunks: hunks.iter().map(|&(old, new)| Hunk { old, new }).collect(),
    }
}

fn seen_target(file: &str, r: LineRange) -> ReviewedSpan {
    let text = join(&numbered(100));
    target_span(anchor(Side::Target, file, &text, r))
}

#[test]
fn filter_change_keeps_state_and_reports_excluded() {
    let diffs = [
        diff("src/a.rs", &[(range(0, 0), range(0, 4))]),
        diff("docs/b.md", &[(range(0, 0), range(0, 6))]),
    ];
    let spans = [seen_target("src/a.rs", range(0, 4))];
    let all = ReviewFilter::default();
    let code = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..ReviewFilter::default()
    };

    let c = completion(&all, &diffs, &[], &spans, &[]);
    assert_eq!(
        (c.lines, c.seen_lines, c.excluded_lines, c.is_complete()),
        (10, 4, 0, false)
    );

    let c = completion(&code, &diffs, &[], &spans, &[]);
    assert_eq!(
        (c.lines, c.seen_lines, c.excluded_lines, c.is_complete()),
        (4, 4, 6, true)
    );

    let c = completion(&all, &diffs, &[], &spans, &[]);
    assert!(!c.is_complete());
}

#[test]
fn old_base_spans_do_not_complete() {
    let diffs = [diff("a.rs", &[(range(0, 0), range(0, 4))])];
    let mut span = seen_target("a.rs", range(0, 4));
    span.state = SpanState::SeenAgainstOldBase;

    let c = completion(&ReviewFilter::default(), &diffs, &[], &[span], &[]);

    assert_eq!((c.seen_lines, c.is_complete()), (0, false));
}

#[test]
fn modified_lines_need_both_sides_seen() {
    let diffs = [diff("a.rs", &[(range(2, 4), range(2, 5))])];
    let text = join(&numbered(10));
    let target = target_span(anchor(Side::Target, "a.rs", &text, range(2, 5)));
    let base = ReviewedSpan {
        base: Some(anchor(Side::Base, "a.rs", &text, range(2, 4))),
        target: None,
        state: SpanState::Seen,
    };

    let f = ReviewFilter::default();
    assert!(!completion(&f, &diffs, &[], std::slice::from_ref(&target), &[]).is_complete());
    assert!(completion(&f, &diffs, &[], &[target, base], &[]).is_complete());
}

#[test]
fn mode_only_change_needs_acknowledgement() {
    let entry = |mode| FileEntry {
        blob: BlobId::of(b"x"),
        mode,
        kind: EntryKind::File,
        unfiltered: false,
    };
    let change = NonTextChange {
        path: path("run.sh"),
        before: Some(entry(0o100644)),
        after: Some(entry(0o100755)),
        from: None,
        kind: NonTextKind::Mode,
    };
    let f = ReviewFilter::default();

    let c = completion(&f, &[], std::slice::from_ref(&change), &[], &[]);
    assert_eq!(
        (c.items, c.acknowledged_items, c.is_complete()),
        (1, 0, false)
    );

    let c = completion(
        &f,
        &[],
        std::slice::from_ref(&change),
        &[],
        std::slice::from_ref(&change),
    );
    assert!(c.is_complete());
}

#[test]
fn filter_roots_globs_and_binary() {
    let f = ReviewFilter {
        roots: vec![path("src")],
        include: vec!["*.rs".into()],
        exclude: vec!["src/gen/**".into()],
        hide_binary: true,
    };
    assert!(f.passes(&path("src/a/b.rs")));
    assert!(!f.passes(&path("srcx/b.rs")));
    assert!(!f.passes(&path("src/a/b.md")));
    assert!(!f.passes(&path("src/gen/x/b.rs")));

    let bin = NonTextChange {
        path: path("src/img.rs"),
        before: None,
        after: None,
        from: None,
        kind: NonTextKind::Binary,
    };
    let c = completion(&f, &[], &[bin], &[], &[]);
    assert_eq!((c.items, c.excluded_items), (0, 1));
}
