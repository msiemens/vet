//! Base resolution (§6.5): merge bases, commit lookup and the branches.

use gix::bstr::ByteSlice;
use gix::remote::Direction;
use vet_app::changeset::CommitId;
use vet_app::ports::{BranchInfo, CommitInfo, GitError};

use super::{err, err_chain, object_id};

/// The commit `spec` names, or none when it names nothing or something that is not a commit. A
/// spec git cannot resolve is none (gix does not tell it from a failure to read the refs); an
/// object that cannot be read or peeled is an error.
fn commit_of<'r>(repo: &'r gix::Repository, spec: &str) -> Result<Option<gix::Commit<'r>>, GitError> {
    let Ok(id) = repo.rev_parse_single(spec) else {
        return Ok(None);
    };
    let object = id
        .object()
        .map_err(|error| err_chain(&error))?
        .peel_tags_to_end()
        .map_err(|error| err_chain(&error))?;
    Ok((object.kind == gix::object::Kind::Commit).then(|| object.into_commit()))
}

pub(super) fn resolve_commit(repo: &gix::Repository, spec: &str) -> Result<Option<CommitId>, GitError> {
    Ok(commit_of(repo, spec)?.map(|c| CommitId(c.id.to_string())))
}

/// The tip of the local branch `name`, else of the remote-tracking branch of that name
/// (`origin/feature`).
pub(super) fn branch_tip(repo: &gix::Repository, name: &str) -> Result<Option<CommitId>, GitError> {
    match resolve_commit(repo, &format!("refs/heads/{name}"))? {
        Some(tip) => Ok(Some(tip)),
        None => resolve_commit(repo, &format!("refs/remotes/{name}")),
    }
}

pub(super) fn merge_base(repo: &gix::Repository, reference: &str, tip: &CommitId) -> Result<CommitId, GitError> {
    let other = commit_of(repo, reference)?
        .ok_or_else(|| GitError::UnknownRevision(format!("unknown branch or revision {reference}")))?;
    let base = repo
        .merge_base(other.id, object_id(tip)?)
        .map_err(|_| GitError::Unrelated(format!("no common ancestor with {reference}")))?;
    Ok(CommitId(base.to_string()))
}

/// `origin/HEAD` if the repository has it and it points at a branch that exists, else
/// the local branch `main` or `master`.
pub(super) fn default_reference(repo: &gix::Repository) -> Result<Option<String>, GitError> {
    if let Some(mut head) = repo
        .try_find_reference("refs/remotes/origin/HEAD")
        .map_err(|error| err_chain(&error))?
        && head.peel_to_id().is_ok()
    {
        return Ok(Some("refs/remotes/origin/HEAD".to_owned()));
    }
    for name in ["main", "master"] {
        let full = format!("refs/heads/{name}");
        if repo
            .try_find_reference(full.as_str())
            .map_err(|error| err_chain(&error))?
            .is_some()
        {
            return Ok(Some(full));
        }
    }
    Ok(None)
}

/// The branch `origin/HEAD` points at if the repository has it and the branch exists, else the
/// local `main` or `master`.
pub(super) fn default_branch(repo: &gix::Repository) -> Result<Option<String>, GitError> {
    if let Some(head) = repo
        .try_find_reference("refs/remotes/origin/HEAD")
        .map_err(|error| err_chain(&error))?
        && let Some(target) = head.target().try_name()
        && let Some(name) = target
            .as_bstr()
            .to_str()
            .ok()
            .and_then(|t| t.strip_prefix("refs/remotes/origin/"))
        && repo
            .try_find_reference(target)
            .map_err(|error| err_chain(&error))?
            .is_some()
    {
        return Ok(Some(name.to_owned()));
    }
    for name in ["main", "master"] {
        if repo
            .try_find_reference(format!("refs/heads/{name}").as_str())
            .map_err(|error| err_chain(&error))?
            .is_some()
        {
            return Ok(Some(name.to_owned()));
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
        .map_err(|error| err_chain(&error))?;
    let mut range = Vec::new();
    for info in walk {
        range.push(CommitId(info.map_err(|error| err_chain(&error))?.id.to_string()));
    }
    range.reverse();
    Ok(range)
}

pub(super) fn commit_info(repo: &gix::Repository, commit: &CommitId) -> Result<CommitInfo, GitError> {
    let found = repo
        .find_commit(object_id(commit)?)
        .map_err(|error| err_chain(&error))?;
    let author = found.author().map_err(err)?;
    Ok(CommitInfo {
        id: commit.clone(),
        subject: found.message().map_err(err)?.summary().to_string(),
        author: author.name.to_string(),
        time: author.time().map_err(err)?.seconds,
    })
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
        .map_err(|error| err_chain(&error))?;
    let mut count = 0;
    for info in walk.take(max) {
        info.map_err(|error| err_chain(&error))?;
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

/// The local branches, then the remote-tracking branches (`origin/feature`, without the remote's
/// `HEAD`) that no local branch follows at the same commit, each group by name.
pub(super) fn branches(repo: &gix::Repository) -> Result<Vec<BranchInfo>, GitError> {
    let current = repo.head_name().map_err(|error| err_chain(&error))?;
    let default = repo
        .try_find_reference("refs/remotes/origin/HEAD")
        .map_err(|error| err_chain(&error))?
        .and_then(|r| {
            let target = r.target().try_name()?.as_bstr().to_str().ok()?.to_owned();
            target.strip_prefix("refs/remotes/origin/").map(str::to_owned)
        });

    let mut branches = Vec::new();
    // Remote-tracking refs a local branch follows, with the commit that branch is at.
    let mut followed = Vec::new();
    for reference in repo
        .references()
        .map_err(|error| err_chain(&error))?
        .local_branches()
        .map_err(|error| err_chain(&error))?
    {
        let mut reference = reference.map_err(|error| err_chain(&error))?;
        let name = reference.name().shorten().to_string();
        let commit = CommitId(reference.peel_to_id().map_err(|error| err_chain(&error))?.to_string());
        if let Some(tracking) = tracking_ref(repo, reference.name()) {
            followed.push((tracking, commit.clone()));
        }
        branches.push(BranchInfo {
            current: current.as_ref().is_some_and(|c| c.as_ref() == reference.name()),
            default: default.as_deref() == Some(name.as_str()),
            remote: false,
            name,
            commit,
        });
    }
    branches.sort_by(|a, b| a.name.cmp(&b.name));

    let mut remote = Vec::new();
    for reference in repo
        .references()
        .map_err(|error| err_chain(&error))?
        .remote_branches()
        .map_err(|error| err_chain(&error))?
    {
        let mut reference = reference.map_err(|error| err_chain(&error))?;
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
            remote: true,
            name,
            commit,
        });
    }
    remote.sort_by(|a, b| a.name.cmp(&b.name));
    branches.extend(remote);
    Ok(branches)
}
