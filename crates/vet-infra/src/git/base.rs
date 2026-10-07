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

/// The tip of the local branch `name`, else of the remote-tracking branch of that name
/// (`origin/feature`).
pub(super) fn branch_tip(repo: &gix::Repository, name: &str) -> Result<CommitId, GitError> {
    resolve_commit(repo, &format!("refs/heads/{name}"))
        .or_else(|_| resolve_commit(repo, &format!("refs/remotes/{name}")))
        .map_err(|_| GitError(format!("unknown branch {name}")))
}

pub(super) fn merge_base(repo: &gix::Repository, reference: &str, tip: &CommitId) -> Result<CommitId, GitError> {
    let other =
        commit_of(repo, reference).ok_or_else(|| GitError(format!("unknown branch or revision {reference}")))?;
    let base = repo
        .merge_base(other.id, object_id(tip)?)
        .map_err(|_| GitError(format!("no common ancestor with {reference}")))?;
    Ok(CommitId(base.to_string()))
}

/// `origin/HEAD` if the repository has it and it points at a branch that exists, else
/// the local branch `main` or `master`.
pub(super) fn default_reference(repo: &gix::Repository) -> Result<Option<String>, GitError> {
    if let Some(mut head) = repo.try_find_reference("refs/remotes/origin/HEAD").map_err(err)?
        && head.peel_to_id().is_ok()
    {
        return Ok(Some("refs/remotes/origin/HEAD".to_owned()));
    }
    for name in ["main", "master"] {
        let full = format!("refs/heads/{name}");
        if repo.try_find_reference(full.as_str()).map_err(err)?.is_some() {
            return Ok(Some(full));
        }
    }
    Ok(None)
}

pub(super) fn first_parent_range(
    repo: &gix::Repository,
    base: &CommitId,
    tip: &CommitId,
) -> Result<Vec<CommitId>, GitError> {
    let walk = repo
        .rev_walk([object_id(tip)?])
        .with_hidden([object_id(base)?])
        .first_parent_only()
        .all()
        .map_err(err)?;
    let mut range = Vec::new();
    for info in walk {
        range.push(CommitId(info.map_err(err)?.id.to_string()));
    }
    range.reverse();
    Ok(range)
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

/// The remote-tracking ref the local branch `name` follows: the one it is configured to track,
/// else the default remote's branch of the same name.
fn tracking_ref(repo: &gix::Repository, name: &gix::refs::FullNameRef) -> Option<String> {
    repo.branch_remote_tracking_ref_name(name, Direction::Fetch)
        .and_then(Result::ok)
        .map(|tracking| tracking.to_string())
        .or_else(|| {
            let remote = repo.remote_default_name(Direction::Fetch)?;
            Some(format!("refs/remotes/{remote}/{}", name.shorten()))
        })
}

/// Where the remote had the local branch `name` at the last fetch.
fn upstream(repo: &gix::Repository, name: &gix::refs::FullNameRef) -> Option<CommitId> {
    let mut tracked = repo.try_find_reference(tracking_ref(repo, name)?.as_str()).ok()??;
    Some(CommitId(tracked.peel_to_id().ok()?.to_string()))
}

/// The local branches, then the remote-tracking branches (`origin/feature`, without the remote's
/// `HEAD`) that no local branch follows at the same commit, each group by name.
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
    // Remote-tracking refs a local branch follows, with the commit that branch is at.
    let mut followed = Vec::new();
    for reference in repo.references().map_err(err)?.local_branches().map_err(err)? {
        let mut reference = reference.map_err(err)?;
        let name = reference.name().shorten().to_string();
        let commit = CommitId(reference.peel_to_id().map_err(err)?.to_string());
        if let Some(tracking) = tracking_ref(repo, reference.name()) {
            followed.push((tracking, commit.clone()));
        }
        branches.push(BranchInfo {
            current: current.as_ref().is_some_and(|c| c.as_ref() == reference.name()),
            default: default.as_deref() == Some(name.as_str()),
            upstream: upstream(repo, reference.name()),
            remote: false,
            name,
            commit,
        });
    }
    branches.sort_by(|a, b| a.name.cmp(&b.name));

    let mut remote = Vec::new();
    for reference in repo.references().map_err(err)?.remote_branches().map_err(err)? {
        let mut reference = reference.map_err(err)?;
        let full = reference.name().as_bstr().to_string();
        let name = reference.name().shorten().to_string();
        if name.ends_with("/HEAD") || branches.iter().any(|b| b.name == name) {
            continue;
        }
        let Ok(commit) = reference.peel_to_id() else { continue };
        let commit = CommitId(commit.to_string());
        if followed.iter().any(|(tracking, at)| *tracking == full && *at == commit) {
            continue;
        }
        remote.push(BranchInfo {
            current: false,
            default: false,
            upstream: None,
            remote: true,
            name,
            commit,
        });
    }
    remote.sort_by(|a, b| a.name.cmp(&b.name));
    branches.extend(remote);
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
