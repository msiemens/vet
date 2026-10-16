//! Fetching a repository's remotes, read-only: objects are written and
//! remote-tracking refs updated, nothing else. HEAD, local branches, tags, the
//! index and the worktree are never touched, whatever the remote's refspecs say.
//! The refs of fetched pull requests, below `refs/vet/pr/`, are the only ones deleted.
//!
//! HTTPS authenticates with the platform credential for the remote's host when
//! one is given, else with git's credential helpers, else anonymously. SSH runs
//! the system `ssh`. Nothing is ever prompted for: Vet has no terminal to ask on.

use std::sync::atomic::AtomicBool;

use gix::bstr::ByteSlice;
use gix::config::tree::{Core, Http, Ssh, gitoxide::Credentials};
use gix::credentials::helper::{Action, NextAction};
use gix::credentials::protocol::Outcome;
use gix::refspec::instruction::Fetch;
use gix::remote::Direction;
use gix::remote::fetch::refs::update::Mode;
use gix::remote::fetch::{Status, Tags};
use gix::sec::identity::Account;
use vet_app::platform::{CredentialSource, PlatformKind};
use vet_app::ports::{FetchAuth, GitError, RefTip, RefUpdate, RemoteInfo};

use super::{chained, redact};
use crate::http::install_tls_provider;

/// A user name the platforms accept in front of a token.
fn token_user(kind: PlatformKind) -> &'static str {
    match kind {
        PlatformKind::GitHub => "x-access-token",
        PlatformKind::AzureDevOps => "vet",
    }
}

/// `host[:port]` of an `https` URL, lower case, spelled as an account's host is.
fn https_host(url: &gix::Url) -> Option<String> {
    if url.scheme != gix::url::Scheme::Https {
        return None;
    }
    let host = url.host()?.to_ascii_lowercase();
    Some(match url.port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

pub(super) fn remotes(repo: &gix::Repository) -> Result<Vec<RemoteInfo>, GitError> {
    let mut found = Vec::new();
    for name in repo.remote_names() {
        let remote = repo.find_remote(name.as_bstr()).map_err(chained)?;
        let Some(url) = remote.url(Direction::Fetch) else {
            continue;
        };
        found.push(RemoteInfo {
            name: name.to_str_lossy().into_owned(),
            url: url.to_bstring().to_str_lossy().into_owned(),
            https_host: https_host(url),
        });
    }
    Ok(found)
}

/// Whether fetching with `spec` writes only below `refs/remotes/`. Exclusions
/// write nothing; a spec without a destination would only record `FETCH_HEAD`.
fn stays_in_remotes(spec: &gix::refspec::RefSpec) -> bool {
    match spec.to_ref().instruction() {
        gix::refspec::Instruction::Fetch(Fetch::AndUpdate { dst, .. }) => dst.starts_with(b"refs/remotes/"),
        gix::refspec::Instruction::Fetch(Fetch::Exclude { .. }) => true,
        _ => false,
    }
}

/// The repository with the settings a fetch without a terminal needs.
fn headless(mut repo: gix::Repository) -> Result<gix::Repository, GitError> {
    let mut config = repo.config_snapshot_mut();
    config
        .set_value(&Credentials::TERMINAL_PROMPT, "false")
        .map_err(chained)?;
    // Without a terminal `ssh` cannot ask for a passphrase or a host key; it should fail, not wait.
    let ssh_chosen = config.string(Core::SSH_COMMAND).is_some() || std::env::var_os("GIT_SSH").is_some();
    if !ssh_chosen {
        config
            .set_value(&Core::SSH_COMMAND, "ssh -o BatchMode=yes")
            .map_err(chained)?;
        config.set_value(&Ssh::VARIANT, "ssh").map_err(chained)?;
    }
    config.commit().map_err(chained)?;
    Ok(repo)
}

/// The error of a fetch from `remote`. When `ssh` was refused, gix reports credentials that were
/// "not accepted"; Vet passes none to ssh, so the message says what ssh itself said and what to do.
fn fetch_err(remote: &str, url: Option<&gix::Url>, error: &(dyn std::error::Error + 'static)) -> GitError {
    match url.and_then(|url| ssh_denied(url, error)) {
        Some(message) => GitError::Remote(format!("{remote}: {message}")),
        None => GitError::Remote(redact(error)),
    }
}

/// The message for an `ssh` remote that refused to let `ssh` in, from the line `ssh` printed.
fn ssh_denied(url: &gix::Url, error: &(dyn std::error::Error + 'static)) -> Option<String> {
    if url.scheme != gix::url::Scheme::Ssh {
        return None;
    }
    let mut source = Some(error);
    let refusal = loop {
        let current = source?;
        if let Some(io) = current.downcast_ref::<std::io::Error>()
            && io.kind() == std::io::ErrorKind::PermissionDenied
        {
            break io.to_string();
        }
        source = current.source();
    };
    let host = url.host().unwrap_or("the remote");
    // `ssh` prefixes its line with `user@host: `.
    let line = refusal.trim();
    let line = match line.split_once(": ") {
        Some((who, rest)) if !who.contains(' ') => rest,
        _ => line,
    };
    let stop = if line.ends_with('.') { "" } else { "." };
    Some(format!(
        "SSH authentication to {host} failed: {line}{stop} If your key has a passphrase, add it to an agent \
         (ssh-add) or use an agent such as 1Password's; Vet runs ssh without prompting."
    ))
}

/// What a fetch asks the remote for.
enum Specs {
    /// The remote's configured refspecs, the ones that stay below `refs/remotes/`.
    Configured,
    /// Exactly these, each forced.
    Refs(Vec<RefUpdate>),
}

/// Fetches `remote` as its configured refspecs say. Returns how many refs were updated.
pub(super) fn fetch(repo: gix::Repository, name: &str, auth: Option<FetchAuth>) -> Result<usize, GitError> {
    run(repo, name, auth, Specs::Configured)
}

/// Fetches the refs `updates` name. The refs below `refs/vet/` they are kept under move even if
/// that is not a fast-forward.
pub(super) fn fetch_refs(
    repo: gix::Repository,
    name: &str,
    updates: Vec<RefUpdate>,
    auth: Option<FetchAuth>,
) -> Result<(), GitError> {
    if let Some(outside) = updates.iter().find(|u| !u.destination.starts_with("refs/vet/")) {
        return Err(GitError::failed(format!(
            "{} is not below refs/vet/, where fetched refs are kept",
            outside.destination
        )));
    }
    run(repo, name, auth, Specs::Refs(updates)).map(drop)
}

/// Where the refs of fetched pull requests are kept: the only refs a fetch may delete.
const DELETABLE: &str = "refs/vet/pr/";

/// The refs below `prefix`, each with its commit; a symbolic ref has none and is left out.
pub(super) fn refs_below(repo: &gix::Repository, prefix: &str) -> Result<Vec<RefTip>, GitError> {
    let platform = repo.references().map_err(chained)?;
    let mut tips = Vec::new();
    for reference in platform.prefixed(prefix).map_err(chained)? {
        let reference = reference.map_err(chained)?;
        if let Some(id) = reference.target().try_id() {
            tips.push(RefTip {
                name: reference.name().as_bstr().to_str_lossy().into_owned(),
                commit: id.to_string(),
            });
        }
    }
    Ok(tips)
}

/// Whether `descendant` is `ancestor` or has it in its history; false when either is not in the
/// repository or is no commit id.
pub(super) fn descends_from(repo: &gix::Repository, descendant: &str, ancestor: &str) -> Result<bool, GitError> {
    let (Ok(descendant), Ok(ancestor)) = (
        gix::ObjectId::from_hex(descendant.as_bytes()),
        gix::ObjectId::from_hex(ancestor.as_bytes()),
    ) else {
        return Ok(false);
    };
    if !repo.has_object(descendant) || !repo.has_object(ancestor) {
        return Ok(false);
    }
    match repo.merge_base(descendant, ancestor) {
        Ok(base) => Ok(base.detach() == ancestor),
        Err(_) => Ok(false),
    }
}

/// Deletes the refs `names` that exist, all or none if one is outside [`DELETABLE`].
pub(super) fn delete_refs(repo: &gix::Repository, names: &[String]) -> Result<(), GitError> {
    if let Some(outside) = names.iter().find(|n| !n.starts_with(DELETABLE) || n.contains("..")) {
        return Err(GitError::failed(format!(
            "{outside} is not below {DELETABLE}, so Vet does not delete it"
        )));
    }
    for name in names {
        if let Some(reference) = repo.try_find_reference(name.as_str()).map_err(chained)? {
            reference.delete().map_err(chained)?;
        }
    }
    Ok(())
}

fn run(repo: gix::Repository, name: &str, auth: Option<FetchAuth>, specs: Specs) -> Result<usize, GitError> {
    install_tls_provider();

    let mut repo = headless(repo)?;
    // Azure DevOps takes the Entra token of the `az` CLI as a bearer token, not as a password.
    let password = match auth {
        Some(a) if a.kind == PlatformKind::AzureDevOps && a.credential.source == CredentialSource::Cli => {
            let header = format!("Authorization: Bearer {}", a.credential.token.expose());
            repo.config_snapshot_mut()
                .set_value(&Http::EXTRA_HEADER, header.as_str())
                .map_err(chained)?;
            None
        }
        other => other,
    };

    let mut remote = repo.find_remote(name).map_err(chained)?.with_fetch_tags(Tags::None);
    let specs: Vec<gix::bstr::BString> = match specs {
        Specs::Configured => remote
            .refspecs(Direction::Fetch)
            .iter()
            .filter(|s| stays_in_remotes(s))
            .map(|s| s.to_ref().to_bstring())
            .collect(),
        Specs::Refs(updates) => updates
            .iter()
            .map(|u| format!("+{}:{}", u.source, u.destination).into())
            .collect(),
    };
    if specs.is_empty() {
        return Err(GitError::failed(format!(
            "{name} has no refspec that fetches into refs/remotes"
        )));
    }
    remote.replace_refspecs(specs, Direction::Fetch).map_err(chained)?;

    let url = remote.url(Direction::Fetch).cloned();
    let connection = remote
        .connect(Direction::Fetch)
        .map_err(|e| fetch_err(name, url.as_ref(), &e))?;
    let mut configured = connection.configured_credentials_for_current_url();
    let mut offered = false;
    let connection = connection.with_credentials(move |action| {
        if let (Action::Get(context), Some(auth)) = (&action, &password) {
            let wanted = context.url.as_ref().and_then(|u| gix::url::parse(u.as_bstr()).ok());
            if wanted.as_ref().and_then(https_host).as_deref() == Some(auth.host.as_str()) {
                offered = true;
                return Ok(Some(Outcome {
                    identity: Account {
                        username: token_user(auth.kind).to_owned(),
                        password: auth.credential.token.expose().to_owned(),
                        oauth_refresh_token: None,
                    },
                    next: NextAction::from(context.clone()),
                }));
            }
        }
        // Whether our token was taken or refused is not for the user's credential helpers to store or erase.
        if offered && !matches!(action, Action::Get(_)) {
            return Ok(None);
        }
        configured(action)
    });

    let prepared = connection
        .prepare_fetch(gix::progress::Discard, Default::default())
        .map_err(|e| fetch_err(name, url.as_ref(), &e))?;
    let outcome = prepared
        .receive(gix::progress::Discard, &AtomicBool::new(false))
        .map_err(|e| fetch_err(name, url.as_ref(), &e))?;
    let (Status::Change { update_refs, .. } | Status::NoPackReceived { update_refs, .. }) = outcome.status;
    let changed = update_refs
        .updates
        .iter()
        .filter(|u| u.edit_index.is_some() && u.mode != Mode::NoChangeNeeded)
        .count();
    Ok(changed)
}

#[cfg(test)]
mod tests;
