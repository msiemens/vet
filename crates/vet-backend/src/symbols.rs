//! The changed symbols of the changeset on view, analysed by the backend in the background and
//! pushed, so the client shows them without asking and the work is not scheduled per window
//! (SPEC §7.3, A13). It runs while the review screen shows the changeset (D81).

use std::sync::Arc;

use vet_app::session::View;
use vet_core::content::RepoPath;
use vet_protocol::{DerivedInputs, Event, FileSymbols, Response, Unavailable, UnavailableKind};

use crate::Open;
use crate::error::Error;
use crate::open::Booted;

/// Starts (or restarts) the task that analyses the changeset's changed symbols. A newer run
/// replaces the older; the review screen going away or the project closing stops it.
pub(crate) fn refresh_symbols_in_background(open: Arc<Open>) {
    let task = tokio::spawn(run(open.clone()));
    open.keep_symbols(task.abort_handle());
}

/// The changed files of the changeset on view with text to analyse, the files the viewer has open
/// first, then the rest in the order the changeset lists them.
async fn changed_files(open: &Open, booted: &Booted) -> Vec<String> {
    let Ok(manifest) = booted.session.manifest(View::Changeset).await else {
        return Vec::new();
    };
    let paths: Vec<String> = manifest
        .files
        .iter()
        .filter(|file| !file.hunks.is_empty())
        .map(|file| file.path.to_string_lossy())
        .collect();
    open_first(paths, &open.open_files())
}

/// `paths`, with those in `first` moved to the front in `first`'s order; the rest keep their order.
fn open_first(paths: Vec<String>, first: &[String]) -> Vec<String> {
    let mut ordered: Vec<String> = Vec::with_capacity(paths.len());
    for open in first {
        if paths.contains(open) && !ordered.contains(open) {
            ordered.push(open.clone());
        }
    }
    for path in paths {
        if !ordered.contains(&path) {
            ordered.push(path);
        }
    }
    ordered
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
    let files = changed_files(&open, &booted).await;
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

#[cfg(test)]
mod tests {
    use super::open_first;

    #[test]
    fn the_open_files_come_first_in_their_order_and_the_rest_keep_theirs() {
        let paths = vec![
            "a.rs".to_owned(),
            "b.rs".to_owned(),
            "c.rs".to_owned(),
            "d.rs".to_owned(),
        ];
        let ordered = open_first(paths, &["c.rs".to_owned(), "a.rs".to_owned()]);
        assert_eq!(ordered, ["c.rs", "a.rs", "b.rs", "d.rs"]);
    }

    #[test]
    fn an_open_file_not_in_the_changeset_is_ignored() {
        let paths = vec!["a.rs".to_owned(), "b.rs".to_owned()];
        let ordered = open_first(paths, &["gone.rs".to_owned(), "b.rs".to_owned()]);
        assert_eq!(ordered, ["b.rs", "a.rs"]);
    }
}
