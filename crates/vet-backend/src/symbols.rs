//! The changed symbols of the changeset on view, analysed by the backend in the background and
//! pushed, so the client shows them without asking and the work is not scheduled per window
//! (SPEC §7.3, A13). It runs while the review screen shows the changeset (D81).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use futures::stream::{self, StreamExt};
use vet_core::content::RepoPath;
use vet_protocol::{DerivedInputs, Event, FileSymbols, Response, Unavailable, UnavailableKind};

use crate::Open;
use crate::analysis::{changed_files, injected, retrying};
use crate::error::Error;
use crate::open::Booted;

/// Files analysed at once, as the window did.
const AT_ONCE: usize = 4;

/// The kind and slot of the run that analyses the changed symbols.
pub(crate) const KIND: &str = "symbols";

/// Starts the run that analyses the changed symbols of `revision`. It replaces an older run; the
/// supervisor stops it when the review screen goes away or the project closes.
pub(crate) fn start(open: &Arc<Open>, revision: u32) {
    open.supervisor
        .replace(KIND, open.supervisor.track(KIND, run(open.clone(), revision)));
}

/// Analyses each changed file's symbols, the open ones first, a few at a time, and pushes each
/// answer as it comes. Stops when nothing is left, the review screen goes away or the revision
/// moves (a newer run analyses the new revision).
async fn run(open: Arc<Open>, revision: u32) {
    let booted = match open.booted().await {
        Ok(booted) => booted.clone(),
        Err(_) => return,
    };
    let (opened, rest) = changed_files(&open, &booted, |file| !file.hunks.is_empty()).await;
    let total = (opened.len() + rest.len()) as u32;
    let done = Arc::new(AtomicU32::new(0));
    // The files the viewer has open first, then the rest; each batch a few at a time.
    for batch in [opened, rest] {
        stream::iter(batch)
            .for_each_concurrent(AT_ONCE, |path| {
                let (open, booted, done) = (open.clone(), booted.clone(), done.clone());
                async move {
                    if !open.screens.shown() || booted.session.revision().await != revision {
                        return;
                    }
                    let symbols = match retrying(|| {
                        let (open, booted, path) = (open.clone(), booted.clone(), path.clone());
                        async move {
                            match injected(&open, &path) {
                                Some(injected) => Err(injected.error()),
                                None => booted.file_symbols(revision, &path).await,
                            }
                        }
                    })
                    .await
                    {
                        Ok(Response::FileSymbols(symbols)) => symbols,
                        Ok(_) => return,
                        Err(error) => {
                            tracing::warn!(%path, %error, "analysing a file's symbols failed");
                            failed(revision, &path, &booted, &error)
                        }
                    };
                    let analysed = done.fetch_add(1, Ordering::Relaxed) + 1;
                    open.subscribers.broadcast(Event::FileSymbols {
                        project: open.id,
                        symbols,
                        done: analysed,
                        total,
                    });
                }
            })
            .await;
        if !open.screens.shown() || booted.session.revision().await != revision {
            return;
        }
    }
}

/// What to push for a file whose analysis failed outright: unknown, with the reason, so the client
/// shows it as unavailable rather than as a file with no symbols.
fn failed(revision: u32, path: &str, booted: &Booted, error: &Error) -> FileSymbols {
    FileSymbols {
        revision,
        path: path.to_owned(),
        symbols: Vec::new(),
        context: Vec::new(),
        gap: Some(Unavailable {
            kind: UnavailableKind::QueryFailed,
            label: "Symbols unavailable".to_owned(),
            message: error.to_string(),
            language: booted.language_of(&RepoPath::from(path)),
        }),
        inputs: DerivedInputs {
            analysis: String::new(),
            bounds: None,
            primitives: Vec::new(),
            hunks: Vec::new(),
            config: Vec::new(),
        },
    }
}
