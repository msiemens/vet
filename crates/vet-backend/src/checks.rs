//! The checks of the commits on view, refreshed by the backend and pushed, so the client shows
//! them without asking and the platform is asked on the account's terms rather than the window's.

use std::sync::Arc;
use std::time::Duration;

use vet_app::changeset::Source;
use vet_app::platform::{CiState, Platform, PlatformError, Standing, commits_checks};
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::Event;

use crate::{Live, Open};

/// How many of the newest versions' tips have their checks asked for, as the review shows them.
const MAX_CHECKED_VERSIONS: usize = 20;

/// The shortest pause after a rate-limited checks refresh, however soon the platform says.
const MIN_PACE: Duration = Duration::from_millis(50);

/// Starts (or restarts) the task that keeps the checks of the changeset on view fresh. It runs
/// while a pull request is on view — in the queue or on the review screen, both of which show its
/// checks — and stops when another changeset comes on view or the project closes. A newer run
/// replaces the older.
pub(crate) fn refresh_checks_in_background(open: Arc<Open>) {
    refresh(open, None);
}

/// The same, after an action on a run: the platform shows what it made of the action a moment
/// later, so the checks are read once more `checks_settle` after the first read.
pub(crate) fn refresh_checks_after_action(open: Arc<Open>) {
    let settle = open.checks_settle();
    refresh(open, Some(settle));
}

fn refresh(open: Arc<Open>, settle: Option<Duration>) {
    let pace = open.checks_pace();
    let task = tokio::spawn(crate::background::tracked(
        &open,
        "checks",
        run(open.clone(), pace, settle),
    ));
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
