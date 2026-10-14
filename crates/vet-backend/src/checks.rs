//! The checks of the commits on view, refreshed by the backend and pushed, so the client shows
//! them without asking and the platform is asked on the account's terms rather than the window's.

use std::sync::Arc;
use std::time::Duration;

use vet_app::changeset::Source;
use vet_app::platform::{CiState, PlatformError, Standing, commits_checks};
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::Event;

use crate::{Live, Open};

/// How many of the newest versions' tips have their checks asked for, as the review shows them.
const MAX_CHECKED_VERSIONS: usize = 20;

/// Starts (or restarts) the task that keeps the checks of the changeset on view fresh, if the
/// review screen is shown. A newer run replaces the older; the project closing stops it.
pub(crate) fn refresh_checks_in_background(open: Arc<Open>) {
    if !open.screens.shown() {
        return;
    }
    let pace = open.checks_pace();
    let task = tokio::spawn(run(open.clone(), pace));
    open.keep_checking(task.abort_handle());
}

/// The commits the review view shows the checks of: the changeset's commits and the newest
/// versions' tips, as the commit list, the timeline and the head show them.
async fn watched_commits(session: &Live) -> Vec<String> {
    let mut commits: Vec<String> = Vec::new();
    let mut keep = |id: String| {
        if !commits.contains(&id) {
            commits.push(id);
        }
    };
    if let Ok(list) = session.commits().await {
        for unit in list.commits {
            keep(unit.info.id.0);
        }
    }
    if let Ok(versions) = session.versions().await {
        for version in versions.iter().rev().take(MAX_CHECKED_VERSIONS) {
            if let Some(tip) = &version.tip {
                keep(tip.id.0.clone());
            }
        }
    }
    commits
}

/// The checks of the changeset on view, asked of the platform and pushed, again while some run. A
/// rate limit the account's health knows is waited out and then resumed; another failure of the
/// account stops the run; so does nothing being left to wait for.
async fn run(open: Arc<Open>, pace: Duration) {
    let booted = match open.booted().await {
        Ok(booted) => booted.clone(),
        Err(_) => return,
    };
    loop {
        let Source::PullRequest { remote, .. } = booted.session.source().await else {
            return;
        };
        let commits = watched_commits(&booted.session).await;
        if commits.is_empty() {
            return;
        }
        let hosting = open.hosting(remote.kind());
        let (found, error) = commits_checks(&hosting, &remote, &commits).await;
        let (checks, error, limited_until) = mapping::checks_parts(&found, error.as_ref());
        open.subscribers.broadcast(Event::ChecksChanged {
            project: open.id,
            checks,
            error,
            limited_until,
        });
        match open.platforms.standing(&remote.account()) {
            Some(Standing::Failing(PlatformError::RateLimited { retry_at, .. })) => {
                let until = retry_at.map(|t| Duration::from_secs((t - unix_now()).max(0) as u64));
                tokio::time::sleep(until.unwrap_or(pace).max(Duration::from_millis(50))).await;
                continue;
            }
            Some(Standing::Failing(_)) => return,
            _ => {}
        }
        if !found.iter().any(|c| c.state() == Some(CiState::Pending)) {
            return;
        }
        tokio::time::sleep(pace).await;
    }
}
