//! The changed symbols of the changeset on view, analysed by the backend in the background and
//! pushed, so the client shows them without asking and the work is not scheduled per window
//! (SPEC §7.3, A13). It runs while the review screen shows the changeset (D81).

use std::sync::Arc;

use vet_core::content::RepoPath;
use vet_protocol::{DerivedInputs, Event, FileSymbols, Response, Unavailable, UnavailableKind};

use crate::Open;
use crate::analysis::changed_files;
use crate::error::Error;
use crate::open::Booted;

/// Starts (or restarts) the task that analyses the changeset's changed symbols. A newer run
/// replaces the older; the review screen going away or the project closing stops it.
pub(crate) fn refresh_symbols_in_background(open: Arc<Open>) {
    let task = tokio::spawn(run(open.clone()));
    open.keep_symbols(task.abort_handle());
}

/// Analyses each changed file's symbols, the open ones first, and pushes each answer as it comes.
/// Stops when nothing is left, the review screen goes away or the revision moves (a newer run
/// analyses the new revision).
async fn run(open: Arc<Open>) {
    let booted = match open.booted().await {
        Ok(booted) => booted.clone(),
        Err(_) => return,
    };
    let revision = booted.session.revision().await;
    let files = changed_files(&open, &booted, |file| !file.hunks.is_empty()).await;
    let total = files.len() as u32;
    for (done, path) in files.into_iter().enumerate() {
        if !open.screens.shown() || booted.session.revision().await != revision {
            return;
        }
        let symbols = match booted.file_symbols(revision, &path).await {
            Ok(Response::FileSymbols(symbols)) => symbols,
            Ok(_) => continue,
            Err(error) => {
                tracing::warn!(path, %error, "analysing a file's symbols failed");
                failed(revision, &path, &booted, &error)
            }
        };
        open.subscribers.broadcast(Event::FileSymbols {
            project: open.id,
            symbols,
            done: done as u32 + 1,
            total,
        });
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
