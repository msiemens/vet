//! What the review derives from snapshots and commits alone: the diff of two snapshots with its
//! texts, the interdiff of two versions, and the commits between a base and a tip, each with its
//! own diff. Each is a keyed flight of `vet-engine`: computed once however many requests ask for
//! it, and kept while the engine has room, as its key names every input. A computation owns what
//! it reads and never touches a session's state; the session remaps its review state onto the
//! values it gets.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use vet_core::content::{BlobId, NonTextKind, RepoPath, Snapshot, SnapshotDiff, SnapshotId};
use vet_core::interdiff::InterdiffHunk;
use vet_core::merkle::empty_snapshot;
use vet_core::version_diff::{VersionSides, version_diff};
use vet_engine::{Computed, Engine, Kind, Limits, Progress, Retention};

use super::blobs::image_media_type;
use super::views::{ItemView, ModeChange};
use super::{LiveSession, Result, SessionError};
use crate::acquisition::Acquisition;
use crate::changeset::CommitId;
use crate::ports::{CommitDetail, GitRepo, LiveStore, TextDiff};
use crate::progress::{TaskKind, Tasks};

/// A diff with what showing and marking it needs.
pub(super) struct Diffed {
    pub(super) diff: SnapshotDiff,
    /// Every path the diff mentions, on either side.
    pub(super) changed: BTreeSet<RepoPath>,
    /// The text of both sides of the diffed files.
    pub(super) blobs: BTreeMap<BlobId, Vec<u8>>,
    /// Both sides of each `diff.non_text` change.
    pub(super) items: Vec<ItemView>,
}

impl Diffed {
    /// `blobs` has the text of the files of `diff`.
    async fn of<G: GitRepo + Sync, S: LiveStore + Sync>(
        acquisition: &Acquisition<G, S>,
        diff: SnapshotDiff,
        blobs: BTreeMap<BlobId, Vec<u8>>,
    ) -> Self {
        let mut items = Vec::new();
        for n in &diff.non_text {
            let before_path = n.from.as_ref().unwrap_or(&n.path);
            items.push(ItemView {
                kind: n.kind,
                before: acquisition.describe(before_path, n.before.as_ref(), n.kind).await,
                after: acquisition.describe(&n.path, n.after.as_ref(), n.kind).await,
                has_preview: n.kind == NonTextKind::Binary
                    && image_media_type(&n.path).is_some()
                    && (n.before.is_some() || n.after.is_some()),
                mode_change: (n.kind == NonTextKind::Mode).then(|| ModeChange::of(n.before.as_ref(), n.after.as_ref())),
            });
        }
        let files = diff
            .files
            .iter()
            .flat_map(|d| [d.old_path.as_ref(), d.new_path.as_ref()]);
        let non_text = diff.non_text.iter().flat_map(|n| [Some(&n.path), n.from.as_ref()]);
        let changed = files.chain(non_text).flatten().cloned().collect();
        Self {
            diff,
            changed,
            blobs,
            items,
        }
    }

    /// What it counts against the bytes an engine keeps.
    fn weight(&self) -> usize {
        self.blobs.values().map(Vec::len).sum::<usize>() + 1
    }
}

/// The diff of two snapshots, a function of their ids (content addresses).
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct DiffOf {
    pub(super) old: SnapshotId,
    pub(super) new: SnapshotId,
}

impl Kind for DiffOf {
    type Value = Arc<Diffed>;
    type Error = SessionError;

    fn retention(&self) -> Retention {
        Retention::Kept
    }

    fn weight(value: &Arc<Diffed>) -> usize {
        value.weight()
    }
}

/// An interdiff: version A's target rebased onto version B's base, against B's target.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Interdiff {
    pub(super) base_a: SnapshotId,
    pub(super) target_a: SnapshotId,
    pub(super) base_b: SnapshotId,
    pub(super) target_b: SnapshotId,
}

pub(super) struct Compared {
    /// Shaped like a changeset's diff; the old side's rebased texts are among its blobs.
    pub(super) diffed: Diffed,
    /// The hunks of each text file in `diffed.diff`, with what they cover in B's changeset.
    pub(super) hunks: BTreeMap<RepoPath, Vec<InterdiffHunk>>,
}

impl Kind for Interdiff {
    type Value = Arc<Compared>;
    type Error = SessionError;

    fn retention(&self) -> Retention {
        Retention::Kept
    }

    fn weight(value: &Arc<Compared>) -> usize {
        value.diffed.weight()
    }
}

/// The commits of a changeset: the first-parent history from `base` (left out) to `tip`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct CommitsBetween {
    pub(super) base: CommitId,
    pub(super) tip: CommitId,
}

/// One commit of the changeset with its own diff.
pub(super) struct CommitUnit {
    pub(super) detail: CommitDetail,
    pub(super) kind: UnitKind,
    /// What the commit is compared with: its parent, or the automatic merge of a merge's parents.
    pub(super) old: Snapshot,
    pub(super) new: Snapshot,
    pub(super) diffed: Diffed,
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
    pub(super) fn clean_merge(&self) -> bool {
        self.kind == UnitKind::Merge { remerged: true }
            && self.diffed.diff.files.is_empty()
            && self.diffed.diff.non_text.is_empty()
    }
}

impl Kind for CommitsBetween {
    type Value = Arc<Vec<CommitUnit>>;
    type Error = SessionError;

    fn retention(&self) -> Retention {
        Retention::Kept
    }

    fn weight(value: &Arc<Vec<CommitUnit>>) -> usize {
        value.iter().map(|u| u.diffed.weight()).sum::<usize>() + 1
    }
}

/// How many snapshot diffs a session keeps, and how many bytes of text they may hold together.
const KEPT_DIFFS: Limits = Limits {
    entries: 8,
    weight: 256 << 20,
};
const KEPT_INTERDIFFS: Limits = Limits {
    entries: 16,
    weight: 128 << 20,
};
const KEPT_COMMITS: Limits = Limits {
    entries: 4,
    weight: 128 << 20,
};

/// The engines of the review's derived kinds, each reporting its flights as a task.
pub(super) struct Derived {
    pub(super) diffs: Engine<DiffOf>,
    pub(super) interdiffs: Engine<Interdiff>,
    pub(super) commits: Engine<CommitsBetween>,
}

impl Derived {
    pub(super) fn new(tasks: &Tasks) -> Self {
        Self {
            diffs: Engine::new(KEPT_DIFFS).observing(task(tasks, "Diffing the changes")),
            interdiffs: Engine::new(KEPT_INTERDIFFS).observing(task(tasks, "Comparing versions")),
            commits: Engine::new(KEPT_COMMITS).observing(task(tasks, "Comparing the commits")),
        }
    }
}

/// Starts the task a flight of any key reports as.
fn task<K>(tasks: &Tasks, title: &'static str) -> impl Fn(&K) -> Option<Arc<dyn vet_engine::Task>> + use<K> {
    let tasks = tasks.clone();
    move |_| Some(Arc::new(tasks.start(TaskKind::Diff, title)))
}

impl<G, S> LiveSession<G, S>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
{
    /// The diff of `old` and `new`: kept, joined while it is computed, or computed.
    pub(super) async fn diff_of(&self, old: &Snapshot, new: &Snapshot) -> Result<Arc<Diffed>> {
        let key = DiffOf {
            old: old.id,
            new: new.id,
        };
        let compute = |_| {
            let acquisition = Arc::clone(&self.acquisition);
            let (old, new) = (old.clone(), new.clone());
            async move {
                let diffed = diffed(&acquisition, &old, &new).await?;
                Ok(Computed::new(Arc::new(diffed), true))
            }
        };
        Ok(self.derived.diffs.get(key, compute).await?.value)
    }

    /// The interdiff of `key`, as [`LiveSession::diff_of`].
    pub(super) async fn interdiff_of(&self, key: Interdiff) -> Result<Arc<Compared>> {
        let compute = |progress| {
            let acquisition = Arc::clone(&self.acquisition);
            let key = key.clone();
            async move {
                let compared = compared(&acquisition, &key, &progress).await?;
                Ok(Computed::new(Arc::new(compared), true))
            }
        };
        Ok(self.derived.interdiffs.get(key.clone(), compute).await?.value)
    }

    /// The commits of `key`, as [`LiveSession::diff_of`].
    pub(super) async fn commits_of(&self, key: CommitsBetween) -> Result<Arc<Vec<CommitUnit>>> {
        let compute = |progress| {
            let acquisition = Arc::clone(&self.acquisition);
            let key = key.clone();
            async move {
                let units = units(&acquisition, &key, &progress).await?;
                Ok(Computed::new(Arc::new(units), true))
            }
        };
        Ok(self.derived.commits.get(key.clone(), compute).await?.value)
    }
}

/// The diff of `old` and `new` with the text of the files it changes.
async fn diffed<G: GitRepo + Sync, S: LiveStore + Sync>(
    acquisition: &Acquisition<G, S>,
    old: &Snapshot,
    new: &Snapshot,
) -> Result<Diffed> {
    let TextDiff { diff, texts } = acquisition.git.diff(&acquisition.store, old, new).await?;
    Ok(Diffed::of(acquisition, diff, texts).await)
}

/// The interdiff of `key`, whose snapshots the store holds, as every version's does.
async fn compared<G: GitRepo + Sync, S: LiveStore + Sync>(
    acquisition: &Acquisition<G, S>,
    key: &Interdiff,
    progress: &Progress,
) -> Result<Compared> {
    let snapshot = async |id: SnapshotId| match acquisition.store.snapshot(id).await? {
        Some((snapshot, _)) => Ok(snapshot),
        None => Err(SessionError::NotFound("snapshot of the version".into())),
    };
    let base_a = snapshot(key.base_a).await?;
    let target_a = snapshot(key.target_a).await?;
    let base_b = snapshot(key.base_b).await?;
    let target_b = snapshot(key.target_b).await?;
    let sides = VersionSides {
        base_a: &base_a,
        target_a: &target_a,
        base_b: &base_b,
        target_b: &target_b,
    };
    let paths = sides.paths();
    let wanted = sides.blobs(&paths);
    let mut blobs = BTreeMap::new();
    for (done, (path, blob)) in wanted.iter().enumerate() {
        progress.report(Some((done as u32, wanted.len() as u32)), Some(path.to_string_lossy()));
        acquisition.load(&mut blobs, path, *blob).await?;
    }
    let compared = version_diff(sides, &paths, &mut blobs);
    Ok(Compared {
        diffed: Diffed::of(acquisition, compared.diff, blobs).await,
        hunks: compared.hunks,
    })
}

/// The commits of `key`, oldest first, each with its own diff.
async fn units<G: GitRepo + Sync, S: LiveStore + Sync>(
    acquisition: &Acquisition<G, S>,
    key: &CommitsBetween,
    progress: &Progress,
) -> Result<Vec<CommitUnit>> {
    let (git, store) = (&acquisition.git, &acquisition.store);
    let range = git.first_parent_range(&key.base, &key.tip).await?;
    let mut units: Vec<CommitUnit> = Vec::new();
    for (n, commit) in range.iter().enumerate() {
        progress.report(Some((n as u32, range.len() as u32)), Some(commit.short().to_owned()));
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
                    store.put_blobs(merged.contents).await?;
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
        let diffed = diffed(acquisition, &old, &new).await?;
        units.push(CommitUnit {
            detail,
            kind,
            old,
            new,
            diffed,
        });
    }
    Ok(units)
}
