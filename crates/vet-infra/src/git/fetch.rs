//! Fetching a repository's remotes, read-only: objects are written and
//! remote-tracking refs updated, nothing else. HEAD, local branches, tags, the
//! index and the worktree are never touched, whatever the remote's refspecs say.
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
use vet_app::ports::{FetchAuth, GitError, RemoteInfo};

use super::err;

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
        let remote = repo.find_remote(name.as_bstr()).map_err(err)?;
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
    config.set_value(&Credentials::TERMINAL_PROMPT, "false").map_err(err)?;
    // Without a terminal `ssh` cannot ask for a passphrase or a host key; it should fail, not wait.
    let ssh_chosen = config.string(Core::SSH_COMMAND).is_some() || std::env::var_os("GIT_SSH").is_some();
    if !ssh_chosen {
        config
            .set_value(&Core::SSH_COMMAND, "ssh -o BatchMode=yes")
            .map_err(err)?;
        config.set_value(&Ssh::VARIANT, "ssh").map_err(err)?;
    }
    config.commit().map_err(err)?;
    Ok(repo)
}

/// Fetches `remote` as its configured refspecs say. Returns how many refs were updated.
pub(super) fn fetch(repo: gix::Repository, name: &str, auth: Option<FetchAuth>) -> Result<usize, GitError> {
    // The provider reqwest's TLS needs; installed once, later calls find it in place.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut repo = headless(repo)?;
    // Azure DevOps takes the Entra token of the `az` CLI as a bearer token, not as a password.
    let password = match auth {
        Some(a) if a.kind == PlatformKind::AzureDevOps && a.credential.source == CredentialSource::Cli => {
            let header = format!("Authorization: Bearer {}", a.credential.token.expose());
            repo.config_snapshot_mut()
                .set_value(&Http::EXTRA_HEADER, header.as_str())
                .map_err(err)?;
            None
        }
        other => other,
    };

    let mut remote = repo.find_remote(name).map_err(err)?.with_fetch_tags(Tags::None);
    let specs: Vec<_> = remote
        .refspecs(Direction::Fetch)
        .iter()
        .filter(|s| stays_in_remotes(s))
        .map(|s| s.to_ref().to_bstring())
        .collect();
    if specs.is_empty() {
        return Err(GitError(format!(
            "{name} has no refspec that fetches into refs/remotes"
        )));
    }
    remote.replace_refspecs(specs, Direction::Fetch).map_err(err)?;

    let connection = remote.connect(Direction::Fetch).map_err(err)?;
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
        .map_err(err)?;
    let outcome = prepared
        .receive(gix::progress::Discard, &AtomicBool::new(false))
        .map_err(err)?;
    let (Status::Change { update_refs, .. } | Status::NoPackReceived { update_refs, .. }) = outcome.status;
    let changed = update_refs
        .updates
        .iter()
        .filter(|u| u.edit_index.is_some() && u.mode != Mode::NoChangeNeeded)
        .count();
    Ok(changed)
}
