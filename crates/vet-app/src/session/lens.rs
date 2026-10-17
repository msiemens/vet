//! The by-commit lens: a committed changeset cut into its
//! commits, each with its own diff and its own progress. A commit is compared with its parent;
//! a merge is compared with the automatic merge of its parents, so it shows only what the person
//! who made it decided (`git show --remerge-diff`), and a merge that decided nothing has nothing
//! to review. Marks are shared with the changeset through content: the same lines are seen in
//! both views (I2).

use std::sync::Arc;

use vet_core::completion::Completion;
use vet_core::content::{LineDelta, SnapshotId, SnapshotStatus};

use super::derived::{CommitUnit, CommitsBetween, UnitKind};
use super::manifest::{excluded_groups, file_completion, file_views, review_of, target_view, update_from};
use super::state::{Commits, Remapped, State, View, acknowledged, lifecycle_of};
use super::views::{BaseView, ManifestView, ReviewUpdateView, ReviewView, ViewKind, empty_reason};
use super::{LiveSession, Result, SessionError};
use crate::changeset::{CommitId, Lens, Source};
use crate::mutex::lock;
use crate::ports::{CommitInfo, GitRepo, LiveStore, ReviewWrites};
use crate::review::SpanOrigin;

/// What a manifest says about the lenses, whatever view it is of.
pub(super) struct LensFacts {
    /// The lens the changeset is reviewed in.
    pub(super) lens: Lens,
    /// How many commits lie between the base and the tip.
    pub(super) commits: u32,
    /// How many files the changeset as a whole changes.
    pub(super) files: u32,
    /// The completion of the changeset as a whole.
    pub(super) completion: Completion,
    /// What the changeset as a whole is compared against.
    pub(super) base: BaseView,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommitKind {
    Commit,
    Merge {
        /// The merge equals the automatic merge of its parents: there is nothing to review.
        clean: bool,
        /// Its diff is against the automatic merge of its parents, not against its first parent.
        remerged: bool,
    },
}

/// A commit of the changeset, as the by-commit lens lists it.
#[derive(Clone, PartialEq, Debug)]
pub struct CommitUnitView {
    pub info: CommitInfo,
    /// The message after its subject.
    pub body: String,
    pub parents: Vec<CommitId>,
    pub kind: CommitKind,
    /// What the commit's own diff adds and removes.
    pub delta: LineDelta,
    /// Progress against the commit's own diff.
    pub completion: Completion,
}

#[derive(Clone, PartialEq, Debug)]
pub struct CommitsView {
    pub revision: u32,
    pub lens: Lens,
    /// The commits between the base and the tip, oldest first; none for the working tree.
    pub commits: Vec<CommitUnitView>,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The commits of the changeset on view, each with its own diff and progress, and the lens it
    /// is reviewed in. The working tree has no commits of its own.
    pub async fn commits(&self) -> Result<CommitsView> {
        let (st, _) = self.prepared(View::Changeset, true).await?;
        let lens = match &st.source {
            Source::WorkingTree => Lens::Squash,
            source => (self.acquisition.store.lens(self.repo.clone(), source.clone()).await?).unwrap_or_default(),
        };
        let commits = st
            .units()
            .iter()
            .enumerate()
            .map(|(index, unit)| CommitUnitView {
                info: unit.detail.info.clone(),
                body: unit.detail.body.clone(),
                parents: unit.detail.parents.clone(),
                kind: match unit.kind {
                    UnitKind::Commit => CommitKind::Commit,
                    UnitKind::Merge { remerged } => CommitKind::Merge {
                        clean: unit.clean_merge(),
                        remerged,
                    },
                },
                delta: unit.diffed.diff.files.iter().map(|f| LineDelta::of(&f.hunks)).sum(),
                completion: st.remapped_in(View::Commit { index: index as u32 }).completion,
            })
            .collect();
        Ok(CommitsView {
            revision: st.revision,
            lens,
            commits,
        })
    }

    /// Remembers the lens the changeset on view is reviewed in.
    pub async fn set_lens(&self, lens: Lens) -> Result<()> {
        let st = self.hold().await;
        if st.source == Source::WorkingTree {
            return Err(SessionError::Invalid(
                "the working tree has no commits of its own to review by".into(),
            ));
        }
        st.writes().set_lens(self.repo.clone(), st.source.clone(), lens).await?;
        Ok(())
    }

    /// What every manifest says about the lenses and the changeset as a whole; `current` is the
    /// branch the working tree is on.
    pub(super) async fn lens_facts(&self, st: &State, current: Option<&str>) -> Result<LensFacts> {
        let base = self.base_view(st, current).await?;
        let (lens, commits) = match &st.source {
            Source::WorkingTree => (Lens::Squash, 0),
            source => (
                (self.acquisition.store.lens(self.repo.clone(), source.clone()).await?).unwrap_or_default(),
                self.commit_count(&st.base_commit, &st.head).await?,
            ),
        };
        let changes = st.changes();
        let files = file_views(&changes.diffed, &st.filter, |_, _| {
            (
                ReviewView {
                    hunks: Vec::new(),
                    acknowledged: Vec::new(),
                    old_base_lines: Vec::new(),
                    flagged: Vec::new(),
                },
                Completion::default(),
            )
        });
        Ok(LensFacts {
            lens,
            commits,
            files: files.len() as u32,
            completion: changes.remapped.completion,
            base,
        })
    }

    /// How many commits lie on the first-parent line from `base` to `tip`, walked once while the
    /// session lives.
    async fn commit_count(&self, base: &CommitId, tip: &CommitId) -> Result<u32> {
        let key = (base.clone(), tip.clone());
        let found = lock(&self.commit_counts).get(&key).copied();
        if let Some(count) = found {
            return Ok(count);
        }
        let count = self.acquisition.git.first_parent_range(base, tip).await?.len() as u32;
        lock(&self.commit_counts).insert(key, count);
        Ok(count)
    }

    /// What the commits of the changeset of `st` are the commits between; none for the working
    /// tree, which has no commits of its own.
    pub(super) fn commits_key(&self, st: &State) -> Option<CommitsBetween> {
        (st.source != Source::WorkingTree).then(|| CommitsBetween {
            base: st.base_commit.clone(),
            tip: st.head.clone(),
        })
    }

    /// Makes `units` the commits of the changeset and remaps the review state onto each.
    pub(super) async fn install_commits(&self, st: &mut State, units: Arc<Vec<CommitUnit>>) -> Result<()> {
        let count = units.len();
        let remapped = (0..count).map(|_| Remapped::default()).collect();
        st.changes.as_mut().expect("computed").commits = Some(Commits { units, remapped });
        self.load_known(st).await?;
        for index in 0..count as u32 {
            self.remap_view(st, View::Commit { index }).await?;
        }
        Ok(())
    }

    /// Puts the snapshots a commit's marks are anchored in into the store, so that they can be
    /// loaded when the session no longer holds them.
    pub(super) async fn keep_snapshots(&self, writes: &S::Writes, st: &State, view: View) -> Result<()> {
        let View::Commit { index } = view else {
            return Ok(());
        };
        let unit = &st.units()[index as usize];
        let now = (self.clock)();
        for snapshot in [&unit.old, &unit.new] {
            writes
                .put_snapshot(snapshot.clone(), SnapshotStatus::of(snapshot), now)
                .await?;
        }
        Ok(())
    }

    /// The manifest of one commit's own diff, which `ensure_view` computed.
    pub(super) async fn commit_manifest(&self, st: &State, index: u32) -> Result<ManifestView> {
        let unit = &st.units()[index as usize];
        let remapped = st.remapped_in(View::Commit { index });
        let cov = remapped.coverage();
        let acked = acknowledged(st, &unit.diffed.diff.non_text);
        let compiled = st.filter.compile(&unit.diffed.diff.generated);
        let files = file_views(&unit.diffed, &st.filter, |diff, items| {
            (
                review_of(&cov, diff, items, &acked),
                file_completion(&cov, &compiled, diff, items, &acked),
            )
        });
        let git = &self.acquisition.git;
        let own = &unit.detail.info.id;
        let parent = unit.detail.parents.first().unwrap_or(own);
        let current = git.current_branch().await?;
        let facts = self.lens_facts(st, current.as_deref()).await?;
        let base = BaseView::of_commit(self.commit_info(parent).await?);
        let excluded = excluded_groups(&unit.diffed, st);
        let empty = empty_reason(!excluded.is_empty(), ViewKind::Commit, None, false, None);
        Ok(ManifestView {
            revision: st.revision,
            seq: st.seq,
            name: self.name.clone(),
            lifecycle: lifecycle_of(st),
            base,
            target: target_view(&st.source, &st.head, current.as_deref()),
            filter: st.project_filter().clone(),
            narrowing: st.narrowing(),
            completion: remapped.completion,
            delta: files.iter().map(|f| f.delta).sum(),
            excluded,
            files,
            degraded: None,
            interdiff: None,
            lens: facts.lens,
            commits: facts.commits,
            changeset_base: facts.base,
            changeset_files: facts.files,
            changeset_completion: facts.completion,
            empty,
        })
    }
}

/// The two snapshots of the changeset or commit view `view` and what marks made in it are
/// recorded as coming from.
pub(super) fn view_unit(st: &State, view: View) -> (SnapshotId, SnapshotId, SpanOrigin) {
    match view {
        View::Commit { index } => {
            let unit = &st.units()[index as usize];
            (
                unit.old.id,
                unit.new.id,
                SpanOrigin::Commit(unit.detail.info.id.clone()),
            )
        }
        _ => (st.base.id, st.target.id, SpanOrigin::Squash),
    }
}

/// The review state of `path` in the commit `index`, with the completions it implies.
pub(super) fn commit_update(st: &State, index: u32, path: &str) -> Result<ReviewUpdateView> {
    let unit = &st.units()[index as usize];
    update_from(st, &unit.diffed.diff, st.remapped_in(View::Commit { index }), path)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use vet_core::completion::HunkState;
    use vet_core::review::Side;

    use super::super::{CommentAt, NewThread, NoText};
    use super::*;
    use crate::session::tests::Fixture;

    fn numbered(edited: &[(usize, &str)]) -> String {
        (0..30)
            .map(|i| match edited.iter().find(|(n, _)| *n == i) {
                Some((_, text)) => format!("{text}\n"),
                None => format!("line {i}\n"),
            })
            .collect()
    }

    fn feature() -> Source {
        Source::Branch { name: "feature".into() }
    }

    /// `feature` has three commits on `a.txt`: the first edits line 3, the second line 20, the third
    /// line 3 again.
    async fn three_commits() -> Fixture {
        let base = numbered(&[]);
        let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
        let (c1, c2, c3) = (
            numbered(&[(3, "one")]),
            numbered(&[(3, "one"), (20, "two")]),
            numbered(&[(3, "three"), (20, "two")]),
        );
        fx.git.feature_commits(
            &[("a.txt", &base)],
            &[
                ("c1", &[("a.txt", &c1)]),
                ("c2", &[("a.txt", &c2)]),
                ("c3", &[("a.txt", &c3)]),
            ],
        );
        fx.session.open_changeset(feature()).await.unwrap().unwrap();
        fx
    }

    async fn hunks(fx: &Fixture, view: View) -> Vec<HunkState> {
        let m = fx.session.manifest(view).await.unwrap();
        m.files.iter().flat_map(|f| f.review.hunks.clone()).collect()
    }

    async fn complete(fx: &Fixture) -> Vec<bool> {
        let list = fx.session.commits().await.unwrap();
        list.commits.iter().map(|c| c.completion.is_complete()).collect()
    }

    /// `feature` has two commits on `a.txt`, opened.
    async fn two_commits() -> Fixture {
        let base = numbered(&[]);
        let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
        let (c1, c2) = (numbered(&[(3, "one")]), numbered(&[(3, "one"), (20, "two")]));
        fx.git.feature_commits(
            &[("a.txt", &base)],
            &[("c1", &[("a.txt", &c1)]), ("c2", &[("a.txt", &c2)])],
        );
        fx.session.open_changeset(feature()).await.unwrap().unwrap();
        fx
    }

    fn walks(fx: &Fixture) -> usize {
        fx.git.first_parent_walks.load(Ordering::Relaxed)
    }

    #[tokio::test]
    async fn two_manifests_of_a_branch_walk_its_commits_once() {
        let fx = two_commits().await;
        let before = walks(&fx);

        let first = fx.session.manifest(View::Changeset).await.unwrap();
        let second = fx.session.manifest(View::Changeset).await.unwrap();

        assert_eq!((first.commits, second.commits), (2, 2));
        assert_eq!(walks(&fx) - before, 1);
    }

    #[tokio::test]
    async fn a_commit_manifest_after_the_changeset_manifest_does_not_walk_again() {
        let fx = two_commits().await;
        fx.session.manifest(View::Changeset).await.unwrap();
        fx.session.commits().await.unwrap();
        let before = walks(&fx);

        let manifest = fx.session.manifest(View::Commit { index: 1 }).await.unwrap();

        assert_eq!(manifest.commits, 2);
        assert_eq!(walks(&fx), before);
    }

    #[tokio::test]
    async fn a_moved_tip_walks_again() {
        let fx = two_commits().await;
        assert_eq!(fx.session.manifest(View::Changeset).await.unwrap().commits, 2);
        let (c1, c2) = (numbered(&[(3, "one")]), numbered(&[(3, "one"), (20, "two")]));
        let c3 = numbered(&[(3, "three"), (20, "two")]);
        fx.git.feature_commits(
            &[("a.txt", &numbered(&[]))],
            &[
                ("c1", &[("a.txt", &c1)]),
                ("c2", &[("a.txt", &c2)]),
                ("c3", &[("a.txt", &c3)]),
            ],
        );
        fx.session.observe().await.unwrap().expect("the tip moved");
        let before = walks(&fx);

        let manifest = fx.session.manifest(View::Changeset).await.unwrap();

        assert_eq!(manifest.commits, 3);
        assert_eq!(walks(&fx) - before, 1);
    }

    #[tokio::test]
    async fn a_manifest_lists_no_branches() {
        let fx = two_commits().await;
        let before = fx.git.branch_listings.load(Ordering::Relaxed);

        fx.session.manifest(View::Changeset).await.unwrap();
        fx.session.manifest(View::Commit { index: 0 }).await.unwrap();
        fx.session.open_changeset(Source::WorkingTree).await.unwrap().unwrap();
        let tree = fx.session.manifest(View::Changeset).await.unwrap();

        assert_eq!(fx.git.branch_listings.load(Ordering::Relaxed), before);
        assert_eq!(tree.target.branch.as_deref(), Some("main"));
    }

    #[tokio::test]
    async fn the_commits_of_a_branch_are_listed_oldest_first_each_with_its_own_diff() {
        let fx = three_commits().await;

        let list = fx.session.commits().await.unwrap();

        let subjects: Vec<_> = list.commits.iter().map(|c| c.info.subject.as_str()).collect();
        assert_eq!(subjects, ["subject of c1", "subject of c2", "subject of c3"]);
        assert_eq!(list.lens, Lens::Squash);
        assert!(list.commits.iter().all(|c| c.kind == CommitKind::Commit));
        let deltas: Vec<_> = list.commits.iter().map(|c| (c.delta.added, c.delta.removed)).collect();
        assert_eq!(deltas, [(1, 1), (1, 1), (1, 1)]);
        assert_eq!(list.commits[2].parents, [CommitId("c2".into())]);
        let second = fx.session.manifest(View::Commit { index: 1 }).await.unwrap();
        assert_eq!(second.files[0].hunks.len(), 1);
        assert_eq!(
            second.files[0].hunks[0].new.start, 20,
            "its own change, not the branch's"
        );
        assert_eq!(second.base.commit.id, CommitId("c1".into()));
    }

    #[tokio::test]
    async fn the_working_tree_has_no_commits_and_a_lens_is_remembered_for_a_branch() {
        let fx = three_commits().await;
        fx.session.set_lens(Lens::PerCommit).await.unwrap();
        assert_eq!(fx.session.commits().await.unwrap().lens, Lens::PerCommit);

        fx.session.open_changeset(Source::WorkingTree).await.unwrap().unwrap();

        let list = fx.session.commits().await.unwrap();
        assert!(list.commits.is_empty());
        assert!(fx.session.set_lens(Lens::PerCommit).await.is_err());
    }

    #[tokio::test]
    async fn a_mark_in_a_commit_is_seen_in_the_changeset_and_in_any_commit_with_the_same_lines() {
        let fx = three_commits().await;
        let revision = fx.session.revision().await;
        assert_eq!(complete(&fx).await, [false, false, false]);

        fx.session
            .toggle_hunk(revision, View::Commit { index: 1 }, "a.txt", 0)
            .await
            .unwrap();

        assert_eq!(complete(&fx).await, [false, true, false]);
        assert_eq!(
            hunks(&fx, View::Changeset).await,
            [HunkState::Unseen, HunkState::Seen],
            "line 20 was written by that commit and is the branch's as well"
        );
        assert_eq!(hunks(&fx, View::Commit { index: 2 }).await, [HunkState::Unseen]);
    }

    #[tokio::test]
    async fn a_mark_in_the_changeset_is_seen_in_the_commits_but_only_for_content_they_share() {
        let fx = three_commits().await;
        let revision = fx.session.revision().await;

        fx.session
            .toggle_hunk(revision, View::Changeset, "a.txt", 0)
            .await
            .unwrap();

        // Line 3 as the branch ends up with it was written by the third commit; the first
        // commit's version of it is intermediate content nobody marked.
        assert_eq!(complete(&fx).await, [false, false, false]);
        assert_eq!(hunks(&fx, View::Commit { index: 0 }).await, [HunkState::Unseen]);
        let seen = fx.session.manifest(View::Commit { index: 2 }).await.unwrap();
        assert_eq!(
            seen.files[0].completion.seen_lines, 1,
            "its added line is the branch's and is seen, the line it replaced is not the branch's base line"
        );
    }

    #[tokio::test]
    async fn all_commits_seen_means_the_changeset_is_seen() {
        let fx = three_commits().await;
        let revision = fx.session.revision().await;

        for index in 0..3 {
            fx.session
                .toggle_hunk(revision, View::Commit { index }, "a.txt", 0)
                .await
                .unwrap();
        }

        assert_eq!(complete(&fx).await, [true, true, true]);
        assert_eq!(hunks(&fx, View::Changeset).await, [HunkState::Seen, HunkState::Seen]);
        let manifest = fx.session.manifest(View::Changeset).await.unwrap();
        assert!(manifest.completion.is_complete());
    }

    #[tokio::test]
    async fn marks_made_in_commits_survive_a_restart_of_the_session() {
        let fx = three_commits().await;
        let revision = fx.session.revision().await;
        fx.session
            .toggle_hunk(revision, View::Commit { index: 1 }, "a.txt", 0)
            .await
            .unwrap();

        fx.git.queue_scan(&[("a.txt", &numbered(&[]))]);
        let again = crate::session::LiveSession::open(fx.git.clone(), fx.store.clone(), "/repo".into(), || 0)
            .await
            .unwrap();
        again.open_changeset(feature()).await.unwrap().unwrap();

        let list = again.commits().await.unwrap();
        assert_eq!(
            list.commits
                .iter()
                .map(|c| c.completion.is_complete())
                .collect::<Vec<_>>(),
            [false, true, false]
        );
    }

    #[tokio::test]
    async fn un_marking_in_a_commit_takes_the_changeset_mark_with_it() {
        let fx = three_commits().await;
        let revision = fx.session.revision().await;
        let view = View::Commit { index: 1 };
        fx.session.toggle_hunk(revision, view, "a.txt", 0).await.unwrap();

        fx.session.toggle_hunk(revision, view, "a.txt", 0).await.unwrap();

        assert_eq!(complete(&fx).await, [false, false, false]);
        assert_eq!(
            hunks(&fx, View::Changeset).await,
            [HunkState::Unseen, HunkState::Unseen]
        );
    }

    #[tokio::test]
    async fn comments_are_made_in_the_changeset_not_in_a_commit() {
        let fx = three_commits().await;
        let revision = fx.session.revision().await;

        let spot = fx
            .session
            .comment_target(revision, View::Commit { index: 0 }, "a.txt", CommentAt::Hunk(0))
            .await;

        assert!(matches!(spot, Err(SessionError::Invalid(_))), "{spot:?}");
        let created = fx
            .session
            .create_thread(
                revision,
                View::Commit { index: 0 },
                NewThread::Range {
                    side: Side::Target,
                    path: "a.txt".into(),
                    range: vet_core::content::LineRange { start: 3, end: 4 },
                },
                "x".into(),
                0,
            )
            .await;
        assert!(matches!(created, Err(SessionError::Invalid(_))));
    }

    #[tokio::test]
    async fn a_commits_files_are_read_as_that_commit_leaves_them() {
        let base = [("a.txt", "a0\n"), ("b.txt", "b0\n"), ("gone.txt", "g0\n")];
        let fx = Fixture::open(&base, &base).await;
        fx.git.feature_commits(
            &base,
            &[
                ("c1", &[("a.txt", "a1\n"), ("b.txt", "b0\n"), ("gone.txt", "g0\n")]),
                ("c2", &[("a.txt", "a2\n"), ("b.txt", "b0\n"), ("late.txt", "l2\n")]),
            ],
        );
        fx.session.open_changeset(feature()).await.unwrap().unwrap();
        let text = |view: View, path: &'static str| {
            let session = &fx.session;
            async move { session.target_text(view, path).await.map(|t| t.content) }
        };
        let (first, second) = (View::Commit { index: 0 }, View::Commit { index: 1 });

        assert_eq!(
            text(first, "a.txt").await.unwrap(),
            Ok("a1\n".into()),
            "changed by the commit"
        );
        assert_eq!(
            text(first, "b.txt").await.unwrap(),
            Ok("b0\n".into()),
            "not touched by it"
        );
        assert_eq!(
            text(first, "gone.txt").await.unwrap(),
            Ok("g0\n".into()),
            "deleted by a later commit"
        );
        assert_eq!(
            text(first, "late.txt").await.unwrap(),
            Err(NoText::Absent),
            "added by a later one"
        );
        assert_eq!(text(second, "late.txt").await.unwrap(), Ok("l2\n".into()));
        assert_eq!(text(second, "gone.txt").await.unwrap(), Err(NoText::Absent));
        assert_eq!(text(View::Changeset, "a.txt").await.unwrap(), Ok("a2\n".into()));
        assert!(matches!(
            text(View::Changeset, "late.nope").await,
            Err(SessionError::NotFound(_))
        ));
        assert!(matches!(
            text(View::Commit { index: 9 }, "a.txt").await,
            Err(SessionError::NotFound(_))
        ));
    }

    /// `feature` is `c1`, then the merge `mg` of `main`'s `m2` into it, whose tree is `resolution`
    /// and whose parents merge to `remerged` by themselves.
    async fn merged(resolution: &str, remerged: &str) -> Fixture {
        let base = numbered(&[]);
        let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
        let c1 = numbered(&[(3, "one")]);
        fx.git.feature_commits(
            &[("a.txt", &base)],
            &[("c1", &[("a.txt", &c1)]), ("mg", &[("a.txt", resolution)])],
        );
        fx.git.set_parents("mg", &["c1", "m2"]);
        fx.git.set_remerge("mg", &[("a.txt", remerged)]);
        fx.session.open_changeset(feature()).await.unwrap().unwrap();
        fx
    }

    #[tokio::test]
    async fn a_merge_that_decided_nothing_is_clean_and_complete() {
        let both = numbered(&[(3, "one"), (25, "main")]);
        let fx = merged(&both, &both).await;

        let list = fx.session.commits().await.unwrap();

        assert_eq!(
            list.commits[1].kind,
            CommitKind::Merge {
                clean: true,
                remerged: true
            }
        );
        assert!(list.commits[1].completion.is_complete());
        assert_eq!(list.commits[1].completion.lines, 0);
        let manifest = fx.session.manifest(View::Commit { index: 1 }).await.unwrap();
        assert!(manifest.files.is_empty());
    }

    #[tokio::test]
    async fn a_merge_shows_only_what_it_decided_beyond_the_automatic_merge() {
        let automatic = numbered(&[(3, "one"), (25, "main")]);
        let resolved = numbered(&[(3, "one"), (25, "main"), (10, "decided in the merge")]);
        let fx = merged(&resolved, &automatic).await;

        let list = fx.session.commits().await.unwrap();

        assert_eq!(
            list.commits[1].kind,
            CommitKind::Merge {
                clean: false,
                remerged: true
            }
        );
        let m = fx.session.manifest(View::Commit { index: 1 }).await.unwrap();
        assert_eq!(m.files[0].hunks.len(), 1, "main's line 25 is not in it");
        assert_eq!(m.files[0].hunks[0].new.start, 10);
        assert_eq!((list.commits[1].delta.added, list.commits[1].delta.removed), (1, 1));
    }

    #[tokio::test]
    async fn a_merge_without_an_automatic_merge_is_compared_with_its_first_parent() {
        let base = numbered(&[]);
        let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
        let c1 = numbered(&[(3, "one")]);
        let octopus = numbered(&[(3, "one"), (25, "main")]);
        fx.git.feature_commits(
            &[("a.txt", &base)],
            &[("c1", &[("a.txt", &c1)]), ("mg", &[("a.txt", &octopus)])],
        );
        fx.git.set_parents("mg", &["c1", "m2", "m3"]);
        fx.session.open_changeset(feature()).await.unwrap().unwrap();

        let list = fx.session.commits().await.unwrap();

        assert_eq!(
            list.commits[1].kind,
            CommitKind::Merge {
                clean: false,
                remerged: false
            }
        );
        assert_eq!(list.commits[1].delta.added, 1);
    }
}
