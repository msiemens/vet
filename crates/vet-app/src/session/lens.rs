//! The by-commit lens (design-changesets §1.5, §2.3): a committed changeset cut into its
//! commits, each with its own diff and its own progress. A commit is compared with its parent;
//! a merge is compared with the automatic merge of its parents, so it shows only what the person
//! who made it decided (`git show --remerge-diff`), and a merge that decided nothing has nothing
//! to review. Marks are shared with the changeset through content: the same lines are seen in
//! both views (I3).

use std::collections::BTreeMap;

use vet_core::completion::{Completion, Coverage};
use vet_core::content::{LineDelta, Snapshot, SnapshotId, SnapshotStatus};
use vet_core::merkle;

use super::manifest::{changeset_completion, file_completion, file_views, review_of, target_view};
use super::remap::Frame;
use super::state::{Changes, State, acknowledged, commits, lifecycle_of};
use super::views::{BaseView, ManifestView, ReviewUpdateView};
use super::{LiveSession, Result, SessionError};
use crate::changeset::{BaseChoice, CommitId, Lens, Source};
use crate::ports::{CommitDetail, CommitInfo, GitRepo, LiveStore};
use crate::progress::TaskKind;
use crate::review::SpanOrigin;
use vet_core::content::RenameMap;

/// One commit of the changeset with its own diff.
pub(super) struct CommitUnit {
    pub(super) detail: CommitDetail,
    pub(super) kind: UnitKind,
    /// What the commit is compared with: its parent, or the automatic merge of a merge's parents.
    pub(super) old: Snapshot,
    pub(super) new: Snapshot,
    pub(super) changes: Changes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum UnitKind {
    Commit,
    /// `remerged`: compared with the automatic merge of its parents. A merge of more than two
    /// parents, or one whose automatic merge could not be made, is compared with its first
    /// parent instead.
    Merge {
        remerged: bool,
    },
}

impl CommitUnit {
    /// A merge whose automatic merge it equals: nothing was decided in it.
    fn clean_merge(&self) -> bool {
        self.kind == UnitKind::Merge { remerged: true }
            && self.changes.diff.files.is_empty()
            && self.changes.diff.non_text.is_empty()
    }
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

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// The commits of the changeset on view, each with its own diff and progress, and the lens it
    /// is reviewed in. The working tree has no commits of its own.
    pub async fn commits(&self) -> Result<CommitsView> {
        let mut st = self.state.lock().await;
        self.ensure_commits(&mut st).await?;
        let lens = match &st.source {
            Source::WorkingTree => Lens::Squash,
            source => (self.acquisition.store.lens(self.repo.clone(), source.clone()).await?).unwrap_or_default(),
        };
        let changes = st.changes.as_ref().expect("computed");
        let acked = |unit: &CommitUnit| acknowledged(&st, &unit.changes.diff);
        let commits = commits(changes)
            .iter()
            .map(|unit| {
                let cov = Coverage::new(&unit.changes.flat);
                CommitUnitView {
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
                    delta: unit.changes.diff.files.iter().map(|f| LineDelta::of(&f.hunks)).sum(),
                    completion: changeset_completion(&cov, &st.filter, &unit.changes.diff, &acked(unit)),
                }
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
        let st = self.state.lock().await;
        if st.source == Source::WorkingTree {
            return Err(SessionError::Invalid(
                "the working tree has no commits of its own to review by".into(),
            ));
        }
        self.acquisition
            .store
            .set_lens(self.repo.clone(), st.source.clone(), lens)
            .await?;
        Ok(())
    }

    /// Computes the commits of the changeset, each with its own diff, if that is not done.
    pub(super) async fn ensure_commits(&self, st: &mut State) -> Result<()> {
        self.ensure_changes(st).await?;
        if st.changes.as_ref().is_some_and(|c| c.commits.is_some()) {
            return Ok(());
        }
        let task = self.acquisition.tasks.start(TaskKind::Diff, "Comparing the commits");
        let units = self.units(st, &task).await;
        task.conclude(&units);
        let units = units?;
        let count = units.len();
        st.changes.as_mut().expect("computed").commits = Some(units);
        self.load_known(st).await?;
        for index in 0..count as u32 {
            self.remap_view(st, super::interdiff::View::Commit { index }).await?;
        }
        Ok(())
    }

    /// Computes the commits, and that of `index` is among them.
    pub(super) async fn ensure_commit(&self, st: &mut State, index: u32) -> Result<()> {
        self.ensure_commits(st).await?;
        if (index as usize) < commits(st.changes.as_ref().expect("computed")).len() {
            Ok(())
        } else {
            Err(SessionError::NotFound(format!("commit {index}")))
        }
    }

    async fn units(&self, st: &State, task: &crate::progress::Running) -> Result<Vec<CommitUnit>> {
        if st.source == Source::WorkingTree {
            return Ok(Vec::new());
        }
        let acquisition = &self.acquisition;
        let (git, store) = (&acquisition.git, &acquisition.store);
        let range = git.first_parent_range(&st.base_commit, &st.head).await?;
        let mut units: Vec<CommitUnit> = Vec::new();
        for (n, commit) in range.iter().enumerate() {
            let detail = git.commit_detail(commit).await?;
            let new = git.commit_snapshot(commit).await?;
            acquisition.record_commit(new.id, commit.clone());
            let previous = units
                .last()
                .filter(|u| detail.parents.first() == Some(&u.detail.info.id));
            let parent = async |parent: &CommitId| match previous {
                Some(unit) => Ok(unit.new.clone()),
                None => git.commit_snapshot(parent).await,
            };
            let (old, kind) = match detail.parents.as_slice() {
                [] => (empty_snapshot(), UnitKind::Commit),
                [first] => (parent(first).await?, UnitKind::Commit),
                [first, _] => match git.remerge(commit).await {
                    Ok(Some(merged)) => {
                        // The merged files are in no git object: the store holds them.
                        for content in merged.contents {
                            store.put_blob(content).await?;
                        }
                        (merged.snapshot, UnitKind::Merge { remerged: true })
                    }
                    Ok(None) | Err(_) => (parent(first).await?, UnitKind::Merge { remerged: false }),
                },
                [first, ..] => (parent(first).await?, UnitKind::Merge { remerged: false }),
            };
            if !matches!(kind, UnitKind::Merge { remerged: true })
                && let Some(first) = detail.parents.first()
            {
                acquisition.record_commit(old.id, first.clone());
            }
            let (diff, blobs) = self.diffed(&old, &new, task).await?;
            let changes = self.changes_of(diff, blobs).await;
            task.report(
                Some((n as u32 + 1, range.len() as u32)),
                Some(commit.short().to_owned()),
            );
            units.push(CommitUnit {
                detail,
                kind,
                old,
                new,
                changes,
            });
        }
        Ok(units)
    }

    /// Puts the snapshots a commit's marks are anchored in into the store, so that they can be
    /// loaded when the session no longer holds them.
    pub(super) async fn keep_snapshots(&self, st: &State, view: super::interdiff::View) -> Result<()> {
        let super::interdiff::View::Commit { index } = view else {
            return Ok(());
        };
        let unit = &commits(st.changes.as_ref().expect("computed"))[index as usize];
        let now = (self.clock)();
        for snapshot in [&unit.old, &unit.new] {
            self.acquisition
                .store
                .put_snapshot(snapshot.clone(), SnapshotStatus::of(snapshot), now)
                .await?;
        }
        Ok(())
    }

    /// The manifest of one commit's own diff, which `ensure_view` computed.
    pub(super) async fn commit_manifest(&self, st: &State, index: u32) -> Result<ManifestView> {
        let unit = &commits(st.changes.as_ref().expect("computed"))[index as usize];
        let cov = Coverage::new(&unit.changes.flat);
        let acked = acknowledged(st, &unit.changes.diff);
        let files = file_views(&unit.changes, &st.filter, |diff, items| {
            (
                review_of(&cov, diff, items, &acked),
                file_completion(&cov, &st.filter, diff, items, &acked),
            )
        });
        let git = &self.acquisition.git;
        let own = &unit.detail.info.id;
        let parent = unit.detail.parents.first().unwrap_or(own);
        let base = BaseView {
            choice: BaseChoice::Commit { id: parent.clone() },
            commit: git.commit_info(parent).await?,
            name: parent.short().to_owned(),
            behind: None,
        };
        Ok(ManifestView {
            revision: st.revision,
            name: self.name.clone(),
            lifecycle: lifecycle_of(st),
            base,
            target: target_view(&st.source, &git.branches().await?),
            filter: st.filter.clone(),
            completion: changeset_completion(&cov, &st.filter, &unit.changes.diff, &acked),
            delta: files.iter().map(|f| f.delta).sum(),
            files,
            degraded: None,
            interdiff: None,
        })
    }
}

/// The frame of the commit `index`: its own two snapshots, the marks made in it being the ones of its own origin.
pub(super) fn commit_frame(st: &State, index: u32, renames: RenameMap) -> Frame<'_> {
    let unit = &commits(st.changes.as_ref().expect("computed"))[index as usize];
    Frame {
        base: &unit.old,
        target: &unit.new,
        origin: SpanOrigin::Commit(unit.detail.info.id.clone()),
        renames,
    }
}

/// The two snapshots of the changeset or commit view `view` and what marks made in it are
/// recorded as coming from.
pub(super) fn view_unit(st: &State, view: super::interdiff::View) -> (SnapshotId, SnapshotId, SpanOrigin) {
    match view {
        super::interdiff::View::Commit { index } => {
            let unit = &commits(st.changes.as_ref().expect("computed"))[index as usize];
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
    use super::state::{find_diff, shown_items};
    let unit = &commits(st.changes.as_ref().expect("computed"))[index as usize];
    let cov = Coverage::new(&unit.changes.flat);
    let acked = acknowledged(st, &unit.changes.diff);
    let diff = find_diff(&unit.changes.diff, path);
    let items = shown_items(&unit.changes.diff, &st.filter, path);
    let file = diff
        .map(|d| d.path().clone())
        .or_else(|| items.first().map(|n| n.path.clone()))
        .ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    Ok(ReviewUpdateView {
        revision: st.revision,
        path: file,
        review: review_of(&cov, diff, &items, &acked),
        completion: file_completion(&cov, &st.filter, diff, &items, &acked),
        changeset: changeset_completion(&cov, &st.filter, &unit.changes.diff, &acked),
    })
}

fn empty_snapshot() -> Snapshot {
    let entries = BTreeMap::new();
    Snapshot {
        id: merkle::snapshot_id(&entries),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use vet_core::completion::HunkState;
    use vet_core::review::Side;

    use super::super::interdiff::View;
    use super::super::{CommentAt, NewThread};
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
