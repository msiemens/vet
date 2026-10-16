//! The checks of the commits on view, refreshed by the backend and pushed, so the client shows
//! them without asking and the platform is asked on the account's terms rather than the window's.

use std::sync::Arc;
use std::time::Duration;

use vet_app::changeset::Source;
use vet_app::platform::{CiState, Platform, PlatformError, Standing, commits_checks};
use vet_app::progress::TaskKind;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::Event;

use crate::{Live, Open};

/// How many of the newest versions' tips have their checks asked for, as the review shows them.
const MAX_CHECKED_VERSIONS: usize = 20;

/// The shortest pause after a rate-limited checks refresh, however soon the platform says.
const MIN_PACE: Duration = Duration::from_millis(50);

/// The kind and slot of the run that keeps the checks of the changeset on view fresh.
pub(crate) const KIND: &str = "checks";

/// Starts the run that keeps the checks of the pull request on view fresh, replacing an older one.
/// The supervisor keeps it for a pull request in the queue or on the review screen, both of which
/// show its checks. After an action on a run the platform shows what it made of the action a
/// moment later, so the checks are read once more `checks_settle` after the first read.
pub(crate) fn start(open: &Arc<Open>, after_action: bool) {
    let settle = after_action.then(|| open.checks_settle());
    let pace = open.checks_pace();
    open.supervisor
        .replace(KIND, open.supervisor.track(KIND, run(open.clone(), pace, settle)));
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

/// What the footer calls a failed checks refresh.
const TITLE: &str = "Checks refresh";

/// Tells the footer how a read of the checks went: a failure stays shown until a read succeeds. A
/// rate limit is waited out, so it says nothing.
fn show(open: &Open, error: Option<&PlatformError>) {
    match error {
        None => open.tasks.start(TaskKind::Checks, TITLE).finish(),
        Some(PlatformError::RateLimited { .. }) => {}
        Some(error) => open.tasks.start(TaskKind::Checks, TITLE).fail(error.to_string()),
    }
}

/// The checks of the changeset on view, asked of the platform and pushed, again while some run. A
/// rate limit the account's health knows is waited out and then resumed; another failure of the
/// account stops the run; so does nothing being left to wait for, once an after-action `settle`
/// read has been made.
async fn run(open: Arc<Open>, pace: Duration, mut settle: Option<Duration>) {
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
        let Ok(hosting) = open.hosting_for(&remote).await else {
            return;
        };
        let (found, error) = commits_checks(&hosting, &remote, &commits).await;
        show(&open, error.as_ref());
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
                tokio::time::sleep(until.unwrap_or(pace).max(MIN_PACE)).await;
                continue;
            }
            Some(Standing::Failing(_)) => return,
            _ => {}
        }
        if !found.iter().any(|c| c.state() == Some(CiState::Pending)) {
            // Nothing is running. After an action, the platform may not have shown its result yet:
            // forget what it last gave of these checks and read once more, then stop if nothing runs.
            if let Some(wait) = settle.take() {
                hosting.forget_checks(&remote);
                tokio::time::sleep(wait).await;
                continue;
            }
            return;
        }
        // A run is going on; it keeps the round going, the settle read is not needed.
        settle = None;
        tokio::time::sleep(pace).await;
    }
}
