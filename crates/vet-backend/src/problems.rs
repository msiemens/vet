//! The diagnostics of the changeset on view, compared with the base by the backend and pushed, so
//! the client shows them without asking (SPEC §7.3, A13). Runs while the review screen shows the
//! changeset (D81).

use std::sync::Arc;

use vet_app::session::FileStatus;
use vet_core::content::RepoPath;
use vet_protocol::{Event, Response, Unavailable, UnavailableKind};

use crate::Open;
use crate::analysis::changed_files;
use crate::open::Booted;

/// Starts (or restarts) the task that analyses the changeset's files for problems. A newer run
/// replaces the older; the review screen going away or the project closing stops it.
pub(crate) fn refresh_problems_in_background(open: Arc<Open>) {
    let task = tokio::spawn(run(open.clone()));
    open.keep_problems(task.abort_handle());
}

/// Analyses the changed files with text, the open ones first: first each file's target side, so the
/// panel fills at once with the origins pending, then the comparison with the base, which needs the
/// servers. Pushes each answer as it comes; stops when nothing is left, the review screen goes away
/// or the revision moves.
async fn run(open: Arc<Open>) {
    let booted = match open.booted().await {
        Ok(booted) => booted.clone(),
        Err(_) => return,
    };
    let revision = booted.session.revision().await;
    let files = changed_files(&open, &booted, |file| {
        file.status != FileStatus::Deleted && file.target_lines > 0
    })
    .await;
    for compared in [false, true] {
        for path in &files {
            if !open.screens.shown() || booted.session.revision().await != revision {
                return;
            }
            push(&open, &booted, revision, path, compared).await;
        }
    }
}

/// Asks the backend for one file's diagnostics and pushes them; `compared` asks for the comparison
/// with the base side.
async fn push(open: &Open, booted: &Booted, revision: u32, path: &str, compared: bool) {
    let event = match booted.diagnostics(revision, path, compared).await {
        Ok(Response::Diagnostics {
            revision,
            path,
            diagnostics,
            count,
            unavailable,
            origin_unavailable,
            incomplete,
        }) => Event::FileDiagnostics {
            project: open.id,
            revision,
            path,
            diagnostics,
            count,
            unavailable,
            origin_unavailable,
            incomplete,
            compared,
        },
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(path, %error, "analysing a file's diagnostics failed");
            Event::FileDiagnostics {
                project: open.id,
                revision,
                path: path.to_owned(),
                diagnostics: Vec::new(),
                count: 0,
                unavailable: Some(Unavailable {
                    kind: UnavailableKind::QueryFailed,
                    label: "Problems unavailable".to_owned(),
                    message: error.to_string(),
                    language: booted.language_of(&RepoPath::from(path)),
                }),
                origin_unavailable: None,
                incomplete: None,
                compared,
            }
        }
    };
    open.subscribers.broadcast(event);
}
