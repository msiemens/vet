//! Interdiffs of the open review's versions (design-changesets §3, §2.4): what the author changed
//! between two versions, with what a move of the base explains left out, shown as a manifest of
//! its own. Its marks are not review state of its own: they are recorded in the changeset of the
//! newer version, which is the one the review remaps, so seen state carries over.

use std::sync::Arc;

use vet_core::completion::{Completion, Coverage, HunkState};
use vet_core::content::{BlobId, FileDiff, NonTextChange, RepoPath};
use vet_core::interdiff::squash_marks;
use vet_core::marking::Region;
use vet_core::review::{ReviewedSpan, Side};

use super::derived::{Compared, Interdiff};
use super::manifest::{file_views, target_view};
use super::state::{Interdiffed, Shown, State, ack_of, find_diff, find_non_text, lifecycle_of, shown_items};
use super::views::{InterdiffView, ManifestView, ReviewUpdateView, ReviewView};
use super::{LiveSession, Result, SessionError};
use crate::base;
use crate::changeset::{BaseChoice, Version};
use crate::ports::{GitRepo, LiveStore};
use crate::review::SpanOrigin;

/// An interdiff asked for: what it compares, before it is computed.
pub(super) struct Interdiffing {
    pub(super) key: Interdiff,
    from: Version,
    to: Version,
    markable: bool,
}

impl Interdiffing {
    pub(super) fn with(self, compared: Arc<Compared>) -> Interdiffed {
        Interdiffed {
            compared,
            from: self.from,
            to: self.to,
            markable: self.markable,
        }
    }
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The interdiff of the versions `from` and `to` of the open review.
    pub(super) async fn interdiffing(&self, st: &State, from: u32, to: u32) -> Result<Interdiffing> {
        let (Some(changeset), Some(review)) = (st.changeset, &st.review) else {
            return Err(SessionError::Invalid(
                "versions are recorded once a review has started".into(),
            ));
        };
        if from >= to {
            return Err(SessionError::Invalid(
                "an interdiff compares an earlier version with a later one".into(),
            ));
        }
        let versions = self.acquisition.store.versions(changeset).await?;
        let find = |n: u32| {
            (versions.iter().find(|v| v.n == n).cloned()).ok_or_else(|| SessionError::NotFound(format!("version {n}")))
        };
        let (a, b) = (find(from)?, find(to)?);
        let key = Interdiff {
            base_a: a.base,
            target_a: a.target,
            base_b: b.base,
            target_b: b.target,
        };
        Ok(Interdiffing {
            key,
            markable: b.n == review.version,
            from: a,
            to: b,
        })
    }

    /// The manifest of the interdiff `id`.
    pub(super) async fn interdiff_manifest(&self, st: &State, id: &Interdiffed) -> Result<ManifestView> {
        let lens = Lens::new(st, id);
        let files = file_views(&id.compared.diffed, &st.filter, |diff, items| lens.state(diff, items));
        let git = &self.acquisition.git;
        let branches = git.branches().await?;
        let commit = git.commit_info(&id.to.base_commit).await?;
        let base = super::BaseView {
            choice: BaseChoice::Commit {
                id: id.to.base_commit.clone(),
            },
            name: base::name_of(git, &branches, None, &id.to.base_commit).await?,
            commit,
            behind: None,
            contained_in: None,
        };
        let facts = self.lens_facts(st).await?;
        let conflicts = (id.compared.hunks.values().flatten()).any(|h| h.flags.conflict);
        let mut limits = vec![if id.markable {
            format!(
                "The old side is version {} rebased onto the base of version {}, a synthetic tree: hover, definitions and \
                 diagnostics work on the new side only.",
                id.from.n, id.to.n
            )
        } else {
            format!(
                "Version {} is not the latest, and the old side is version {} rebased onto its base: hover, definitions \
                 and diagnostics are not available on either side, and marks and comments are not possible.",
                id.to.n, id.from.n
            )
        }];
        if conflicts {
            limits.push(format!(
                "Hunks marked as not applied did not apply to the new base: their old side shows version {}'s lines unrebased.",
                id.from.n
            ));
        }
        Ok(ManifestView {
            revision: st.revision,
            name: self.name.clone(),
            lifecycle: lifecycle_of(st),
            base,
            target: target_view(&st.source, &st.head, &branches),
            filter: st.filter.clone(),
            completion: files.iter().map(|f| f.completion).sum(),
            delta: files.iter().map(|f| f.delta).sum(),
            files,
            degraded: self.degraded_for(&st.source),
            lens: facts.lens,
            commits: facts.commits,
            changeset_files: facts.files,
            changeset_completion: facts.completion,
            interdiff: Some(InterdiffView {
                from: id.from.n,
                to: id.to.n,
                rebased: id.from.base != id.to.base,
                markable: id.markable,
                limits,
            }),
        })
    }
}

/// The regions to cut out of the recorded spans and the spans to record, each with where it was
/// marked, for marking hunks seen or unseen.
pub(super) type HunkMarks = (Vec<Region>, Vec<(ReviewedSpan, SpanOrigin)>);

/// What marking the interdiff hunks (indices into the file's hunks) seen, or cutting them out
/// of the recorded spans, does to the changeset of the newer version.
pub(super) fn interdiff_hunk_marks(
    st: &State,
    id: &Interdiffed,
    path: &str,
    hunks: &[u32],
    seen: bool,
) -> Result<HunkMarks> {
    let changeset = &st.changes().diffed;
    let squash = find_diff(&changeset.diff, path);
    let diff =
        find_diff(&id.compared.diffed.diff, path).ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    let inter = &id.compared.hunks[diff.path()];
    let blob_text = |blob: Option<BlobId>| blob.and_then(|b| changeset.blobs.get(&b)).map(Vec::as_slice);
    let chosen = (hunks.iter())
        .map(|&i| (inter.get(i as usize)).ok_or_else(|| SessionError::NotFound(format!("hunk {i}"))))
        .collect::<Result<Vec<_>>>()?;
    let (regions, add) = match squash {
        Some(squash) => {
            let texts = (blob_text(squash.old_blob), blob_text(squash.new_blob));
            squash_marks(squash, &chosen, (st.base.id, st.target.id), texts)
                .ok_or_else(|| SessionError::NotText(path.into()))?
        }
        None => (Vec::new(), Vec::new()),
    };
    let add = if seen {
        add.into_iter().map(|s| (s, SpanOrigin::Squash)).collect()
    } else {
        Vec::new()
    };
    Ok((regions, add))
}

/// What a request in an interdiff view may do: marks and comments land in the changeset of the
/// newer version, which only the latest version's has.
pub(super) fn check_markable(shown: &Shown) -> Result<()> {
    if shown.markable() {
        Ok(())
    } else {
        Err(SessionError::Invalid(
            "marks and comments need an interdiff that ends at the latest version".into(),
        ))
    }
}

/// What a request to comment may do: comments are on the changeset and its interdiffs, not on one
/// commit's own diff.
pub(super) fn check_commentable(shown: &Shown) -> Result<()> {
    if matches!(shown, Shown::Commit { .. }) {
        return Err(SessionError::Invalid(
            "comments are made in All changes, not in one commit".into(),
        ));
    }
    check_markable(shown)
}

/// The review state of `path` in the view `shown` with the completions it implies.
pub(super) fn update_in(st: &State, shown: &Shown, path: &str) -> Result<ReviewUpdateView> {
    let id = match shown {
        Shown::Changeset => return super::manifest::update_of(st, path),
        Shown::Commit { index } => return super::lens::commit_update(st, *index, path),
        Shown::Interdiff(id) => id,
    };
    let lens = Lens::new(st, id);
    let diff = find_diff(&id.compared.diffed.diff, path);
    let items = shown_items(&id.compared.diffed.diff, &st.filter, path);
    let file = diff
        .map(|d| d.path().clone())
        .or_else(|| items.first().map(|n| n.path.clone()))
        .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    let (review, completion) = lens.state(diff, &items);
    let files = file_views(&id.compared.diffed, &st.filter, |d, i| lens.state(d, i));
    Ok(ReviewUpdateView {
        revision: st.revision,
        path: file,
        review,
        completion,
        changeset: files.iter().map(|f| f.completion).sum(),
    })
}

/// The squash change whose acknowledgement an interdiff item stands for, if the squash has one.
pub(super) fn squash_item<'a>(st: &'a State, item: &NonTextChange) -> Option<&'a NonTextChange> {
    find_non_text(&st.changes().diffed.diff, item.kind, &item.path.to_string_lossy())
}

/// Review state of an interdiff's files, read from the changeset of the newer version.
struct Lens<'a> {
    st: &'a State,
    id: &'a Interdiffed,
    coverage: Coverage<'a>,
}

impl<'a> Lens<'a> {
    fn new(st: &'a State, id: &'a Interdiffed) -> Self {
        // Coverage is remapped onto the latest version only.
        let coverage = if id.markable {
            st.changes().remapped.coverage()
        } else {
            Coverage::new([])
        };
        Self { st, id, coverage }
    }

    fn squash(&self, path: &RepoPath) -> Option<&'a FileDiff> {
        find_diff(&self.st.changes().diffed.diff, &path.to_string_lossy())
    }

    /// The state of each hunk of `diff`.
    fn hunk_states(&self, diff: &FileDiff) -> Vec<HunkState> {
        let squash = self.squash(diff.path());
        (self.id.compared.hunks[diff.path()].iter())
            .map(|h| {
                if h.flags.informational {
                    return HunkState::Informational;
                }
                let regions = squash.into_iter().flat_map(|s| {
                    let target = s.new_path.as_ref().map(|p| (p, Side::Target, &h.target_lines));
                    let base = s.old_path.as_ref().map(|p| (p, Side::Base, &h.base_lines));
                    [target, base]
                        .into_iter()
                        .flatten()
                        .flat_map(|(p, side, ranges)| ranges.iter().map(move |&r| (p, side, r)))
                });
                match self.coverage.state_of(regions) {
                    HunkState::Seen => HunkState::Seen,
                    _ if h.flags.conflict => HunkState::Unapplied,
                    state => state,
                }
            })
            .collect()
    }

    /// The review state and completion of a file: its text diff, if it has one, and its shown
    /// non-text changes. Only what the newer version's changeset has to review counts.
    fn state(&self, diff: Option<&FileDiff>, items: &[NonTextChange]) -> (ReviewView, Completion) {
        let states = diff.map(|d| self.hunk_states(d)).unwrap_or_default();
        let mut completion = Completion::default();
        let mut old_base_lines = vec![0; states.len()];
        if let Some(d) = diff {
            let squash = self.squash(d.path());
            for ((h, state), old_base) in self.id.compared.hunks[d.path()]
                .iter()
                .zip(&states)
                .zip(&mut old_base_lines)
            {
                if *state == HunkState::Informational {
                    continue;
                }
                completion.hunks += 1;
                completion.seen_hunks += u32::from(*state == HunkState::Seen);
                let Some(s) = squash else { continue };
                for (file, side, ranges) in [
                    (s.new_path.as_ref(), Side::Target, &h.target_lines),
                    (s.old_path.as_ref(), Side::Base, &h.base_lines),
                ] {
                    let Some(file) = file else { continue };
                    for &range in ranges {
                        let (seen, any) = self.coverage.lines(file, side, range);
                        completion.lines += range.len();
                        completion.seen_lines += seen;
                        completion.old_base_lines += any - seen;
                        *old_base += any - seen;
                    }
                }
            }
        }
        let mut acknowledged = Vec::new();
        for item in items {
            let Some(counterpart) = squash_item(self.st, item) else {
                continue;
            };
            completion.items += 1;
            if self.id.markable && self.st.acks.contains(&ack_of(counterpart)) {
                completion.acknowledged_items += 1;
                acknowledged.push(item.kind);
            }
        }
        (
            ReviewView {
                hunks: states,
                old_base_lines,
                acknowledged,
            },
            completion,
        )
    }
}

#[cfg(test)]
mod tests {
    use vet_core::completion::HunkState;
    use vet_core::review::Side;

    use super::super::state::View;
    use super::super::{CommentAt, NewThread};
    use super::*;
    use crate::changeset::{CommitId, VersionOrigin};
    use crate::session::tests::Fixture;

    fn lines(n: usize) -> String {
        (0..n).map(|i| format!("l{i}\n")).collect()
    }

    fn between(from: u32, to: u32) -> View {
        View::Interdiff { from, to }
    }

    async fn states(fx: &Fixture, view: View) -> Vec<HunkState> {
        let m = fx.session.manifest(view).await.unwrap();
        m.files.iter().flat_map(|f| f.review.hunks.clone()).collect()
    }

    /// The review moves to a base of `files`, the next version.
    async fn rebase_onto(fx: &Fixture, files: &[(&str, &str)]) {
        fx.git.add_commit("c1", "c1-full", files);
        let base = BaseChoice::Commit {
            id: CommitId("c1".into()),
        };
        fx.session.set_base(base).await.unwrap();
    }

    #[tokio::test]
    async fn an_interdiff_after_a_rebase_leaves_out_what_the_base_brought() {
        let base = lines(20);
        let upstream = base.replace("l2\n", "upstream\n");
        let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &base.replace("l15\n", "mine\n"))]).await;
        rebase_onto(&fx, &[("a.txt", &upstream)]).await;
        fx.settle(&[("a.txt", &upstream.replace("l15\n", "mine\n"))]).await;

        let m = fx.session.manifest(between(1, 3)).await.unwrap();

        assert!(m.files.is_empty(), "the author changed nothing between the versions");
        assert!(m.interdiff.unwrap().rebased);
        let squash = fx.session.manifest(View::Changeset).await.unwrap();
        assert_eq!(
            squash.files.len(),
            1,
            "the changeset itself still has the author's change"
        );
    }

    #[tokio::test]
    async fn an_interdiff_shows_what_the_author_changed_between_versions() {
        let base = lines(20);
        let first = base.replace("l15\n", "mine\n");
        let second = base.replace("l15\n", "mine, better\n");
        let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &first)]).await;
        fx.settle(&[("a.txt", &second)]).await;

        let m = fx.session.manifest(between(1, 2)).await.unwrap();
        let [file] = &m.files[..] else { panic!("{:?}", m.files) };
        let text = fx.session.file_text(between(1, 2), "a.txt").await.unwrap();

        assert_eq!(file.hunks.len(), 1);
        assert_eq!((file.hunks[0].new.start, file.hunks[0].old.start), (15, 15));
        assert_eq!(
            (text.base.as_deref(), text.target.as_deref()),
            (Some(first.as_str()), Some(second.as_str()))
        );
        assert_eq!(file.review.hunks, [HunkState::Unseen]);
    }

    #[tokio::test]
    async fn a_hunk_the_old_patch_did_not_apply_to_the_new_base_for_is_flagged() {
        let base = lines(20);
        let upstream = base.replace("l10\n", "upstream\n");
        let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &base.replace("l10\n", "mine\n"))]).await;
        rebase_onto(&fx, &[("a.txt", &upstream)]).await;
        fx.settle(&[("a.txt", &upstream.replace("upstream\n", "mine, redone\n"))])
            .await;

        assert_eq!(states(&fx, between(1, 3)).await, [HunkState::Unapplied]);
        let limits = fx
            .session
            .manifest(between(1, 3))
            .await
            .unwrap()
            .interdiff
            .unwrap()
            .limits;
        assert!(limits.iter().any(|l| l.contains("did not apply")), "{limits:?}");
    }

    #[tokio::test]
    async fn a_change_the_author_took_back_is_informational_and_cannot_be_marked() {
        let base = lines(20);
        let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &base.replace("l4\n", "mine\n"))]).await;
        fx.settle(&[("a.txt", &base)]).await;
        let revision = fx.session.revision().await;

        assert_eq!(states(&fx, between(1, 2)).await, [HunkState::Informational]);
        let m = fx.session.manifest(between(1, 2)).await.unwrap();
        assert_eq!(m.completion.lines, 0, "there is nothing to review in it");
        let marked = fx.session.toggle_hunk(revision, between(1, 2), "a.txt", 0).await;
        assert!(matches!(marked, Err(SessionError::Invalid(_))), "{marked:?}");
    }

    #[tokio::test]
    async fn marks_made_in_an_interdiff_are_seen_in_the_changeset() {
        let base = lines(30);
        let first = base.replace("l5\n", "mine\n");
        let second = first.replace("l25\n", "more\n");
        let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &first)]).await;
        fx.settle(&[("a.txt", &second)]).await;
        let revision = fx.session.revision().await;
        assert_eq!(
            states(&fx, View::Changeset).await,
            [HunkState::Unseen, HunkState::Unseen]
        );

        // Only the line the author added in version 2 is in the interdiff.
        let update = fx
            .session
            .toggle_hunk(revision, between(1, 2), "a.txt", 0)
            .await
            .unwrap();

        assert_eq!(update.review.hunks, [HunkState::Seen]);
        assert_eq!(states(&fx, View::Changeset).await, [HunkState::Unseen, HunkState::Seen]);
        assert!(fx.session.versions().await.unwrap()[1].reviewed_here);

        // The mark stays when the tree settles again, and an interdiff that no longer ends at the
        // latest version takes no marks.
        fx.settle(&[("a.txt", &second)]).await;
        assert_eq!(states(&fx, View::Changeset).await, [HunkState::Unseen, HunkState::Seen]);
        let revision = fx.session.revision().await;
        let stale = fx.session.toggle_hunk(revision, between(1, 2), "a.txt", 0).await;
        assert!(matches!(stale, Err(SessionError::Invalid(_))), "{stale:?}");
    }

    #[tokio::test]
    async fn comments_on_an_interdiff_go_on_the_new_side_only() {
        let base = lines(20);
        let fx = Fixture::reviewing(&[("a.txt", &base)], &[("a.txt", &base.replace("l3\n", "mine\n"))]).await;
        fx.settle(&[("a.txt", &base.replace("l3\n", "better\n"))]).await;
        let revision = fx.session.revision().await;
        let view = between(1, 2);

        let new = fx
            .session
            .comment_target(revision, view, "a.txt", CommentAt::Hunk(0))
            .await
            .unwrap();
        assert!(matches!(new.target, NewThread::Range { side: Side::Target, .. }));
        let old_side = CommentAt::Lines {
            side: Side::Base,
            start: 3,
            len: 1,
        };
        let old = fx.session.comment_target(revision, view, "a.txt", old_side).await;
        assert!(matches!(old, Err(SessionError::Invalid(_))), "{old:?}");
    }

    #[tokio::test]
    async fn the_versions_list_follows_settles_and_a_base_move() {
        let base = lines(10);
        let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base.replace("l1\n", "x\n"))]).await;
        assert!(
            fx.session.versions().await.unwrap().is_empty(),
            "browsing records no versions"
        );
        fx.begin().await;
        fx.settle(&[("a.txt", &base.replace("l1\n", "y\n"))]).await;
        rebase_onto(&fx, &[("a.txt", &base.replace("l8\n", "up\n"))]).await;

        let versions = fx.session.versions().await.unwrap();

        let shape: Vec<_> = versions.iter().map(|v| (v.n, v.origin, v.reviewed_here)).collect();
        assert_eq!(
            shape,
            [
                (1, VersionOrigin::Settled, false),
                (2, VersionOrigin::Settled, false),
                (3, VersionOrigin::BaseMoved, false)
            ]
        );
        assert_ne!(versions[2].base, versions[1].base, "the base moved");
        fx.session
            .set_seen(fx.session.revision().await, "a.txt", &[0], true)
            .await
            .unwrap();
        let reviewed: Vec<bool> = (fx.session.versions().await.unwrap().iter())
            .map(|v| v.reviewed_here)
            .collect();
        assert_eq!(reviewed, [false, false, true]);
    }
}
