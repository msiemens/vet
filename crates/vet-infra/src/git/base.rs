//! Base resolution (§6.5): merge bases, commit lookup, and the branches and
//! recent commits a base can be picked from.

use gix::bstr::ByteSlice;
use gix::remote::Direction;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;
use vet_app::changeset::CommitId;
use vet_app::ports::{BaseCandidates, BranchInfo, CommitInfo, GitError};

use super::{err, object_id};

fn commit_of<'r>(repo: &'r gix::Repository, spec: &str) -> Option<gix::Commit<'r>> {
    let id = repo.rev_parse_single(spec).ok()?;
    id.object().ok()?.peel_to_commit().ok()
}

pub(super) fn resolve_commit(repo: &gix::Repository, spec: &str) -> Result<CommitId, GitError> {
    commit_of(repo, spec)
        .map(|c| CommitId(c.id.to_string()))
        .ok_or_else(|| GitError(format!("unknown commit {spec}")))
}

pub(super) fn merge_base(repo: &gix::Repository, reference: &str) -> Result<CommitId, GitError> {
    let other =
        commit_of(repo, reference).ok_or_else(|| GitError(format!("unknown branch or revision {reference}")))?;
    let head = repo.head_id().map_err(err)?;
    let base = repo
        .merge_base(other.id, head)
        .map_err(|_| GitError(format!("no common ancestor with {reference}")))?;
    Ok(CommitId(base.to_string()))
}

pub(super) fn commit_info(repo: &gix::Repository, commit: &CommitId) -> Result<CommitInfo, GitError> {
    let found = repo.find_commit(object_id(commit)?).map_err(err)?;
    let author = found.author().map_err(err)?;
    Ok(CommitInfo {
        id: commit.clone(),
        subject: found.message().map_err(err)?.summary().to_string(),
        author: author.name.to_string(),
        time: author.time().map_err(err)?.seconds,
    })
}

pub(super) fn first_parent_distance(
    repo: &gix::Repository,
    tip: &CommitId,
    commit: &CommitId,
    max: usize,
) -> Result<Option<usize>, GitError> {
    let (mut at, wanted) = (object_id(tip)?, object_id(commit)?);
    for distance in 0..=max {
        if at == wanted {
            return Ok(Some(distance));
        }
        match repo.find_commit(at).map_err(err)?.parent_ids().next() {
            Some(parent) => at = parent.detach(),
            None => break,
        }
    }
    Ok(None)
}

pub(super) fn commits_behind(
    repo: &gix::Repository,
    tip: &CommitId,
    base: &CommitId,
    max: usize,
) -> Result<usize, GitError> {
    let walk = repo
        .rev_walk([object_id(tip)?])
        .with_hidden([object_id(base)?])
        .all()
        .map_err(err)?;
    let mut count = 0;
    for info in walk.take(max) {
        info.map_err(err)?;
        count += 1;
    }
    Ok(count)
}

/// Where the remote had the local branch `name` at the last fetch: the remote-tracking ref the
/// branch is configured to track, else the default remote's branch of the same name.
fn upstream(repo: &gix::Repository, name: &gix::refs::FullNameRef) -> Option<CommitId> {
    let tracking = repo
        .branch_remote_tracking_ref_name(name, Direction::Fetch)
        .and_then(Result::ok)
        .map(|tracking| tracking.to_string())
        .or_else(|| {
            let remote = repo.remote_default_name(Direction::Fetch)?;
            Some(format!("refs/remotes/{remote}/{}", name.shorten()))
        })?;
    let mut tracked = repo.try_find_reference(tracking.as_str()).ok()??;
    Some(CommitId(tracked.peel_to_id().ok()?.to_string()))
}

pub(super) fn branches(repo: &gix::Repository) -> Result<Vec<BranchInfo>, GitError> {
    let current = repo.head_name().map_err(err)?;
    let default = repo
        .try_find_reference("refs/remotes/origin/HEAD")
        .map_err(err)?
        .and_then(|r| {
            let target = r.target().try_name()?.as_bstr().to_str().ok()?.to_owned();
            target.strip_prefix("refs/remotes/origin/").map(str::to_owned)
        });

    let mut branches = Vec::new();
    for reference in repo.references().map_err(err)?.local_branches().map_err(err)? {
        let mut reference = reference.map_err(err)?;
        let name = reference.name().shorten().to_string();
        let commit = reference.peel_to_id().map_err(err)?;
        branches.push(BranchInfo {
            current: current.as_ref().is_some_and(|c| c.as_ref() == reference.name()),
            default: default.as_deref() == Some(name.as_str()),
            upstream: upstream(repo, reference.name()),
            name,
            commit: CommitId(commit.to_string()),
        });
    }
    branches.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(branches)
}

pub(super) fn candidates(repo: &gix::Repository, max_commits: usize) -> Result<BaseCandidates, GitError> {
    let mut commits = Vec::new();
    let head = repo.head_id().map_err(err)?;
    let walk = repo
        .rev_walk([head.detach()])
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
        .all()
        .map_err(err)?;
    for info in walk.take(max_commits) {
        commits.push(commit_info(repo, &CommitId(info.map_err(err)?.id.to_string()))?);
    }
    Ok(BaseCandidates {
        branches: branches(repo)?,
        commits,
    })
}
