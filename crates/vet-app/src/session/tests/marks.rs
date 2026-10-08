//! A mark remaps only the spans it records and adjusts each view's completion by the files they
//! are in; what it leaves is what remapping the whole review afresh gives.

use vet_core::completion::Completion;

use super::*;
use crate::session::state::{Remapped, State};

fn numbered(edited: &[(usize, &str)]) -> String {
    (0..30)
        .map(|i| match edited.iter().find(|(n, _)| *n == i) {
            Some((_, text)) => format!("{text}\n"),
            None => format!("line {i}\n"),
        })
        .collect()
}

/// `feature` has three commits on `a.txt`, `b.txt` and `c.txt`, the second of which also makes
/// `run.sh` executable.
async fn branch() -> Fixture {
    let base = numbered(&[]);
    let untouched: &[(&str, &str)] = &[
        ("a.txt", &base),
        ("b.txt", &base),
        ("c.txt", &base),
        ("run.sh", "echo\n"),
    ];
    let fx = Fixture::open(untouched, untouched).await;
    let (a1, a2, a3) = (
        numbered(&[(3, "a1"), (10, "a1")]),
        numbered(&[(3, "a1"), (10, "a1"), (20, "a2")]),
        numbered(&[(3, "a3"), (10, "a1"), (20, "a2")]),
    );
    let (b1, b3) = (numbered(&[(5, "b1")]), numbered(&[(5, "b1"), (25, "b3")]));
    let c2 = numbered(&[(2, "c2"), (15, "c2")]);
    fx.git.feature_commits(
        untouched,
        &[
            (
                "c1",
                &[("a.txt", &a1), ("b.txt", &b1), ("c.txt", &base), ("run.sh", "echo\n")],
            ),
            (
                "c2",
                &[("a.txt", &a2), ("b.txt", &b1), ("c.txt", &c2), ("run.sh", "echo\n")],
            ),
            (
                "c3",
                &[("a.txt", &a3), ("b.txt", &b3), ("c.txt", &c2), ("run.sh", "echo\n")],
            ),
        ],
    );
    for (_, snapshot) in fx.git.commits.lock().unwrap().values_mut() {
        if snapshot.entries.contains_key(&path("c.txt"))
            && snapshot.entries[&path("c.txt")].blob == BlobId::of(c2.as_bytes())
        {
            snapshot.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
            snapshot.id = merkle::snapshot_id(&snapshot.entries);
        }
    }
    fx.session
        .open_changeset(Source::Branch { name: "feature".into() })
        .await
        .unwrap()
        .unwrap();
    fx.session.commits().await.unwrap();
    fx
}

/// A small deterministic generator, so that a failing sequence can be replayed.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// Remaps the whole review afresh and asserts that every view's review state and completion are
/// what the marks left, and that the stored progress is the fresh one's.
async fn assert_as_remapped_afresh(fx: &Fixture, step: &str) -> HashMap<View, Completion> {
    let mut st = fx.session.state.lock().await;
    let views = st.remapped_views();
    assert_eq!(views.len(), 4, "the changeset and its three commits");
    let left: Vec<Remapped> = views.iter().map(|&v| std::mem::take(st.remapped_in_mut(v))).collect();
    let summary = st.review.as_ref().and_then(|r| r.summary);
    fx.session.remap(&mut st).await.unwrap();
    for (&view, left) in views.iter().zip(&left) {
        assert_eq!(left, st.remapped_in(view), "{view:?} after {step}");
    }
    if let Some(summary) = summary {
        assert_eq!(
            summary,
            ReviewSummary::of(&st.changes().remapped.completion),
            "stored progress after {step}"
        );
    }
    views.iter().map(|&v| (v, st.remapped_in(v).completion)).collect()
}

#[tokio::test]
async fn marks_in_any_view_leave_the_review_state_a_fresh_remap_gives() {
    let fx = branch().await;
    let views = [
        View::Changeset,
        View::Commit { index: 0 },
        View::Commit { index: 1 },
        View::Commit { index: 2 },
    ];
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let excluding = ReviewFilter {
        exclude: vec!["c.txt".into()],
        ..ReviewFilter::default()
    };
    let mut marks = 0;
    for step in 0..60 {
        let view = views[rng.below(views.len())];
        let manifest = fx.session.manifest(view).await.unwrap();
        let revision = manifest.revision;
        if rng.below(12) == 0 {
            let filter = if manifest.filter == excluding {
                ReviewFilter::default()
            } else {
                excluding.clone()
            };
            fx.session.set_filter(filter).await.unwrap();
            assert_as_remapped_afresh(&fx, &format!("step {step}, a filter change")).await;
            continue;
        }
        if manifest.files.is_empty() {
            continue;
        }
        let file = &manifest.files[rng.below(manifest.files.len())];
        let path = file.path.to_string_lossy();
        let (what, answer) = match rng.below(3) {
            0 if !file.hunks.is_empty() => {
                let hunk = rng.below(file.hunks.len()) as u32;
                let answer = fx.session.toggle_hunk(revision, view, &path, hunk).await;
                (format!("hunk {hunk} of {path} in {view:?}"), answer)
            }
            1 if !file.non_text.is_empty() => {
                let kind = file.non_text[0].kind;
                let answer = fx.session.toggle_item(revision, view, &path, kind).await;
                (format!("{kind:?} of {path} in {view:?}"), answer)
            }
            _ => {
                let answer = fx.session.toggle_file_seen(revision, view, &path).await;
                (format!("all of {path} in {view:?}"), answer)
            }
        };
        let answer = answer.unwrap().value;
        marks += 1;
        let step = format!("step {step}, {what}");

        // The answer is the file as the whole view's manifest shows it, and the view's completion
        // as a fresh count gives it.
        let shown = fx.session.manifest(view).await.unwrap();
        let file = shown.files.iter().find(|f| f.path == answer.path).unwrap();
        assert_eq!(
            (&answer.review, answer.completion),
            (&file.review, file.completion),
            "{step}"
        );
        let fresh = assert_as_remapped_afresh(&fx, &step).await;
        assert_eq!(answer.changeset, fresh[&view], "{step}");
    }
    assert!(marks > 40, "{marks} marks");
    let complete = fx.session.manifest(View::Changeset).await.unwrap().completion;
    assert!(complete.seen_lines > 0, "the sequence leaves something seen");
}

#[tokio::test]
async fn a_mark_remaps_only_the_spans_of_its_file() {
    let a = numbered(&[(2, "a"), (8, "a"), (14, "a"), (20, "a"), (26, "a")]);
    let b = numbered(&[(4, "b"), (16, "b")]);
    let base = numbered(&[]);
    let fx = Fixture::reviewing(&[("a.txt", &base), ("b.txt", &base)], &[("a.txt", &a), ("b.txt", &b)]).await;
    fx.session.set_seen(0, "a.txt", &[0, 2, 4], true).await.unwrap();
    fx.session.set_seen(0, "a.txt", &[1], true).await.unwrap();
    let remaps = |st: &State, file: &str| st.changes().remaps.get(&path(file)).copied().unwrap_or(0);
    let marked_a = remaps(&*fx.session.state.lock().await, "a.txt");
    assert!(marked_a > 0);

    fx.session.set_seen(0, "b.txt", &[0], true).await.unwrap();
    fx.session.set_seen(0, "b.txt", &[1], true).await.unwrap();
    fx.session.set_seen(0, "b.txt", &[0], false).await.unwrap();
    fx.session.set_seen(0, "a.txt", &[3], true).await.unwrap();

    let st = fx.session.state.lock().await;
    assert_eq!(
        remaps(&st, "a.txt"),
        marked_a + 2,
        "the marks in b.txt remap none of a.txt's spans, the last mark in a.txt only its own span's two sides"
    );
    assert_eq!(
        remaps(&st, "b.txt"),
        4,
        "each mark in b.txt remaps the span it records, on both sides; the unmark records none"
    );
    drop(st);

    // A new version is another diff: every span is remapped onto it.
    let edited = numbered(&[(2, "a"), (8, "a"), (14, "a"), (20, "a"), (26, "a"), (29, "new")]);
    fx.settle(&[("a.txt", &edited), ("b.txt", &b)]).await;
    fx.session.manifest(View::Changeset).await.unwrap();
    let st = fx.session.state.lock().await;
    let anchors_in_a = (st.recorded.iter())
        .flat_map(|r| r.span.base.iter().chain(&r.span.target))
        .filter(|a| a.file == path("a.txt"))
        .count();
    assert_eq!(anchors_in_a, 10, "five hunks seen, on both sides");
    assert_eq!(remaps(&st, "a.txt"), anchors_in_a);
}

#[tokio::test]
async fn unmarking_cuts_spans_with_only_one_side() {
    let base = numbered(&[]);
    let fx = Fixture::reviewing(&[("gone.txt", &base)], &[("new.txt", &base)]).await;
    for file in ["new.txt", "gone.txt"] {
        fx.session.toggle_hunk(0, View::Changeset, file, 0).await.unwrap();
        assert_eq!(fx.hunks(file).await, [HunkState::Seen], "{file}");
    }

    for file in ["new.txt", "gone.txt"] {
        let answer = fx.session.toggle_hunk(0, View::Changeset, file, 0).await.unwrap().value;
        assert_eq!(answer.review.hunks, [HunkState::Unseen], "{file}");
    }

    assert!(fx.session.state.lock().await.recorded.is_empty());
}

#[tokio::test]
async fn the_answer_of_a_mark_is_the_file_and_the_changeset_as_the_manifest_shows_them() {
    let a = numbered(&[(2, "a"), (8, "a"), (14, "a")]);
    let b = numbered(&[(4, "b"), (16, "b")]);
    let base = numbered(&[]);
    let fx = Fixture::reviewing(&[("a.txt", &base), ("b.txt", &base)], &[("a.txt", &a), ("b.txt", &b)]).await;
    fx.session.set_seen(0, "b.txt", &[1], true).await.unwrap();

    let answer = fx
        .session
        .toggle_hunk(0, View::Changeset, "a.txt", 1)
        .await
        .unwrap()
        .value;

    let manifest = fx.session.manifest(View::Changeset).await.unwrap();
    let file = manifest.files.iter().find(|f| f.path == path("a.txt")).unwrap();
    assert_eq!(answer.path, path("a.txt"));
    assert_eq!(
        answer.review.hunks,
        [HunkState::Unseen, HunkState::Seen, HunkState::Unseen]
    );
    assert_eq!((&answer.review, answer.completion), (&file.review, file.completion));
    assert_eq!(answer.changeset, manifest.completion);
    assert_eq!(
        (
            answer.changeset.seen_hunks,
            answer.changeset.hunks,
            answer.changeset.seen_lines
        ),
        (2, 5, 4),
        "the hunk of a.txt and the one of b.txt, two lines each"
    );
}
