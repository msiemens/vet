//! The diagnostics of the changeset on view, compared with the base by the backend and pushed, so
//! the client shows them without asking (SPEC §6.6, A13). Runs while the review screen shows the
//! changeset (D81).

use std::sync::Arc;

use futures::stream::StreamExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use vet_app::session::FileStatus;
use vet_core::content::RepoPath;
use vet_protocol::{Event, Response, Unavailable, UnavailableKind};

use crate::Open;
use crate::analysis::{BOTH_SIDES, TARGET_SIDE, Turns, changed_files, in_turns, retrying};
use crate::open::Booted;

/// Files whose target side is read at once, as the window did.
const TARGET_AT_ONCE: usize = 4;
/// Files whose comparison with the base runs at once, which is slower and needs the servers.
const COMPARED_AT_ONCE: usize = 2;

/// The kind and slot of the run that analyses the files for problems.
pub(crate) const KIND: &str = "problems";

/// Starts the run that analyses the changeset's files of `revision` for problems. It replaces an
/// older run; the supervisor stops it when the review screen goes away or the project closes.
pub(crate) fn start(open: &Arc<Open>, revision: u32) {
    open.supervisor
        .replace(KIND, open.supervisor.track(KIND, run(open.clone(), revision)));
}

/// Analyses the changed files with text, the open ones first: first each file's target side, so the
/// panel fills at once with the origins pending, then the comparison with the base, which needs the
/// servers. Pushes each answer as it comes; stops when nothing is left, the review screen goes away
/// or the revision moves.
async fn run(open: Arc<Open>, revision: u32) {
    let booted = match open.booted().await {
        Ok(booted) => booted.clone(),
        Err(_) => return,
    };
    let (opened, rest) = changed_files(&open, &booted, |file| {
        file.status != FileStatus::Deleted && file.target_lines > 0
    })
    .await;
    for compared in [false, true] {
        let at_once = Arc::new(Semaphore::new(if compared { COMPARED_AT_ONCE } else { TARGET_AT_ONCE }));
        // The files the viewer has open first, then the rest; each batch a few at a time, which are
        // turns taken in order: a file waiting for its server gives its turn away.
        for batch in [&opened, &rest] {
            in_turns(batch.iter().cloned(), at_once.clone())
                .for_each_concurrent(None, |(path, permit)| {
                    let (open, booted, at_once) = (open.clone(), booted.clone(), at_once.clone());
                    async move {
                        if !open.screens.shown() || booted.session.revision().await != revision {
                            return;
                        }
                        push(&open, &booted, revision, &path, compared, &at_once, permit).await;
                    }
                })
                .await;
            if !open.screens.shown() || booted.session.revision().await != revision {
                return;
            }
        }
    }
}

/// Asks the backend for one file's diagnostics and pushes them; `compared` asks for the comparison
/// with the base side. `permit` is the turn of `permits` the file took.
async fn push(
    open: &Open,
    booted: &Booted,
    revision: u32,
    path: &str,
    compared: bool,
    permits: &Arc<Semaphore>,
    permit: OwnedSemaphorePermit,
) {
    let turns = Turns::new(
        permits.clone(),
        booted.semantics.watch_runtimes(),
        booted.semantics.language_of(&RepoPath::from(path)).cloned(),
        if compared { BOTH_SIDES } else { TARGET_SIDE },
    );
    let event = match retrying(&turns, permit, || {
        let (booted, path) = (booted, path.to_owned());
        async move { booted.diagnostics(revision, &path, compared).await }
    })
    .await
    {
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
            tracing::warn!(%path, %error, "analysing a file's diagnostics failed");
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
