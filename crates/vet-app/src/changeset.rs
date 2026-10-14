//! What is under review (§3): a changeset is something that evolves and is
//! reviewed as one, a source with a base rule. It has versions, each a base and
//! a tip; its reviews and its threads belong to it, not to each other.
//! `vet_core::content::Changeset` stays the pure pair of snapshots a version
//! resolves to.

use vet_core::content::{LineDelta, RepoPath, SnapshotId};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CommitId(pub String);

impl CommitId {
    /// The abbreviation shown for a commit with no better name.
    pub fn short(&self) -> &str {
        &self.0[..self.0.len().min(7)]
    }
}

/// How a changeset's base follows from its kind (§6.5). The target does not follow from it; that
/// is the changeset's source.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BaseChoice {
    /// The merge base of `reference` (a branch or any revision) and the changeset's tip, which
    /// moves with them.
    MergeBase { reference: String },
    /// A commit, stored and reported by its full id.
    Commit { id: CommitId },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChangesetId(pub u64);

pub use crate::platform::RemoteKey;

/// Where a changeset's target comes from. With the repository root it names the
/// changeset: it is stable across versions, rebases and restarts.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    /// The repository's live tree. Versions are settled snapshots.
    WorkingTree,
    /// A local branch's commits, or a remote-tracking branch's (`origin/feature`). Versions
    /// are the tips Vet observed.
    Branch { name: String },
    /// A pull request. Versions are the pushes the platform recorded.
    PullRequest { remote: RemoteKey, number: u64 },
}

/// How a committed changeset is cut into units for review: the whole of it, or one unit per commit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Lens {
    #[default]
    Squash,
    PerCommit,
}

/// One state of a changeset: a base and the target built on a tip. The first
/// version of a working tree is the snapshot current when its review started.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Version {
    /// 1-based, per changeset.
    pub n: u32,
    /// What the version's content is; none for a push of a pull request whose commits could not be
    /// had, which is listed but cannot be shown.
    pub snapshots: Option<Snapshots>,
    /// The commit the base is the tree of; empty for a version without snapshots whose base is
    /// not known.
    pub base_commit: CommitId,
    /// The commit the target is built on: `HEAD` for a working tree.
    pub tip: CommitId,
    /// Unix seconds.
    pub at: i64,
    pub origin: VersionOrigin,
    pub change: VersionChange,
}

/// The two trees a version is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshots {
    pub base: SnapshotId,
    pub target: SnapshotId,
}

/// What a version changed against the one before it, or for the first version against its
/// base. A move of the base is not a change: it is what the interdiff leaves out. Computed once
/// when the version is recorded.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct VersionChange {
    /// By path.
    pub files: Vec<ChangedFile>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangedFile {
    pub path: RepoPath,
    /// Zero for a file without text lines to count (a mode change, a binary file).
    pub delta: LineDelta,
}

impl VersionChange {
    pub fn delta(&self) -> LineDelta {
        self.files.iter().map(|f| f.delta).sum()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VersionOrigin {
    /// The tree settled, or the review of it started.
    Settled,
    /// A branch's tip or its merge base was seen to have moved: new commits, an
    /// amend or a rebase. For a pull request: a push the platform recorded.
    Observed,
}

impl Version {
    /// The base commit and the tip the version is built on.
    pub fn commits(&self) -> (&CommitId, &CommitId) {
        (&self.base_commit, &self.tip)
    }

    /// The commits of the last of `versions` that has content: the ones a version after them
    /// need not take a ref for again.
    pub fn last_commits(versions: &[Self]) -> Option<(&CommitId, &CommitId)> {
        versions.iter().rev().find(|v| v.snapshots.is_some()).map(Self::commits)
    }

    /// The commits this version needs a ref for to stay materializable: its own,
    /// unless the version before it, whose `commits` are `previous`, holds them already. A version
    /// without content has none to keep.
    pub fn commits_to_keep(&self, previous: Option<(&CommitId, &CommitId)>) -> Vec<(&'static str, &CommitId)> {
        let mut keep = Vec::new();
        if self.snapshots.is_none() {
            return keep;
        }
        if previous.is_none_or(|(base, _)| *base != self.base_commit) {
            keep.push(("base", &self.base_commit));
        }
        if previous.is_none_or(|(_, tip)| *tip != self.tip) {
            keep.push(("tip", &self.tip));
        }
        keep
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::Hash;

    use super::*;

    fn version(n: u32, base: &str, tip: &str) -> Version {
        Version {
            n,
            snapshots: Some(Snapshots {
                base: SnapshotId(Hash([0; 32])),
                target: SnapshotId(Hash([1; 32])),
            }),
            base_commit: CommitId(base.into()),
            tip: CommitId(tip.into()),
            at: 0,
            origin: VersionOrigin::Settled,
            change: VersionChange::default(),
        }
    }

    #[test]
    fn a_version_keeps_the_commits_an_earlier_one_does_not_hold() {
        let first = version(1, "b1", "h1");
        let names = |v: &Version, previous: Option<&Version>| -> Vec<(&str, String)> {
            v.commits_to_keep(previous.map(Version::commits))
                .into_iter()
                .map(|(n, c)| (n, c.0.clone()))
                .collect()
        };
        assert_eq!(
            names(&first, None),
            [("base", "b1".to_string()), ("tip", "h1".to_string())]
        );
        assert!(names(&version(2, "b1", "h1"), Some(&first)).is_empty());
        assert_eq!(
            names(&version(3, "b2", "h1"), Some(&first)),
            [("base", "b2".to_string())]
        );
        assert_eq!(
            names(&version(3, "b1", "h2"), Some(&first)),
            [("tip", "h2".to_string())]
        );
    }
}
