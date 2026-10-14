//! Which changed files the background analysis (symbols, problems) reads, in what order, and how a
//! failure that can pass is asked again.

#[cfg(debug_assertions)]
use std::collections::{HashMap, VecDeque};
#[cfg(debug_assertions)]
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

#[cfg(debug_assertions)]
use vet_app::ports::StoreError;
use vet_app::session::{FileView, SessionError, View};
use vet_protocol::{Response, Unavailable, UnavailableKind};

use crate::Open;
use crate::error::Error;
use crate::open::Booted;

/// How many times one file's analysis is asked again while it fails for a reason that can pass,
/// before the failure is pushed as final.
const TRIES: usize = 3;
/// How long between two tries.
const RETRY_PAUSE: Duration = Duration::from_millis(500);

/// Whether a reason the semantic layer gave for a file can pass if it is asked again: a server that
/// is starting or indexing (or was restarted), or a query that failed. A file that is unavailable
/// for a lasting reason — no server for its language, a project the analysis may not run in, the
/// server does not offer the operation, no text to analyse (binary, too large, not a changed file),
/// a snapshot that cannot be rebuilt — is not asked again.
fn can_pass(reason: &Unavailable) -> bool {
    matches!(
        reason.kind,
        UnavailableKind::Incomplete | UnavailableKind::QueryFailed | UnavailableKind::ServerFailed
    )
}

/// Whether a failure of the analysis itself can pass: a store read (the review database, its
/// content or the servers'). A file that is not there to analyse, or a revision that moved on,
/// cannot.
fn error_can_pass(error: &Error) -> bool {
    match error {
        Error::Store(_) => true,
        Error::Session(SessionError::Store(_)) | Error::Servers(vet_app::servers::ServersError::Store(_)) => true,
        Error::Context { source, .. } => error_can_pass(source),
        _ => false,
    }
}

/// Whether the answer to one file's analysis is a failure that can pass: the whole answer failed,
/// the comparison with the base could not be made, or a server was at work while it answered.
fn answer_can_pass(answer: &Response) -> bool {
    match answer {
        Response::FileSymbols(symbols) => symbols.gap.as_ref().is_some_and(can_pass),
        Response::Diagnostics {
            unavailable,
            origin_unavailable,
            incomplete,
            ..
        } => {
            unavailable.as_ref().is_some_and(can_pass)
                || origin_unavailable.as_ref().is_some_and(can_pass)
                || incomplete.is_some()
        }
        _ => false,
    }
}

/// `analyse`, asked up to [`TRIES`] times while it fails for a reason that can pass; the last
/// answer is returned. Callers check the review screen and the revision before each file; a newer
/// run, a hidden screen or a closed project aborts the task, and with it a pause between two tries.
pub(crate) async fn retrying<F, Fut>(mut analyse: F) -> Result<Response, Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Response, Error>>,
{
    for _ in 1..TRIES {
        match analyse().await {
            Ok(answer) if !answer_can_pass(&answer) => return Ok(answer),
            Err(error) if !error_can_pass(&error) => return Err(error),
            Ok(_) | Err(_) => tokio::time::sleep(RETRY_PAUSE).await,
        }
    }
    analyse().await
}

/// A failure a test asks the symbols analysis of a file to give, so the retry can be exercised
/// without a broken server. Only a debug build has it.
#[cfg(debug_assertions)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Injected {
    /// The store was busy: a failure that can pass.
    Store,
    /// The file is not there to analyse: a lasting reason.
    Gone,
}

#[cfg(debug_assertions)]
impl Injected {
    /// The failure the analysis of the file gives.
    pub(crate) fn error(self) -> Error {
        match self {
            Injected::Store => Error::Store(StoreError("the store was busy (injected)".into())),
            Injected::Gone => Error::Session(SessionError::NotFound("the file is gone (injected)".into())),
        }
    }
}

#[cfg(not(debug_assertions))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Injected {}

#[cfg(not(debug_assertions))]
impl Injected {
    pub(crate) fn error(self) -> Error {
        match self {}
    }
}

/// The next failure a test asked the symbols analysis of `path` to give, taken from its script. A
/// debug build only; a release build has no script and answers nothing.
#[cfg(debug_assertions)]
pub(crate) fn injected(open: &Open, path: &str) -> Option<Injected> {
    let mut failures = open.analysis_failures.lock().unwrap_or_else(PoisonError::into_inner);
    let script = failures.get_mut(path)?;
    let next = script.pop_front();
    if script.is_empty() {
        failures.remove(path);
    }
    next
}

#[cfg(not(debug_assertions))]
pub(crate) fn injected(_: &Open, _: &str) -> Option<Injected> {
    None
}

/// For tests: the files whose symbols analysis fails before it runs for real, and what it fails
/// with, in order. A debug build only.
#[cfg(debug_assertions)]
pub(crate) type Failures = Mutex<HashMap<String, VecDeque<Injected>>>;

/// Which changed files the changeset on view that `keep` accepts, as two batches: the files the
/// viewer has open, in the order the window shows them, then the rest in the changeset's order.
/// The open files are read first.
pub(crate) async fn changed_files(
    open: &Open,
    booted: &Booted,
    keep: fn(&FileView) -> bool,
) -> (Vec<String>, Vec<String>) {
    let Ok(manifest) = booted.session.manifest(View::Changeset).await else {
        return (Vec::new(), Vec::new());
    };
    let paths: Vec<String> = manifest
        .files
        .iter()
        .filter(|file| keep(file))
        .map(|file| file.path.to_string_lossy())
        .collect();
    open_first(paths, &open.open_files())
}

/// `paths` as the open ones and the rest: those in `first` (each once, in `first`'s order) that the
/// changeset has, then the others in their order.
fn open_first(paths: Vec<String>, first: &[String]) -> (Vec<String>, Vec<String>) {
    let mut opened: Vec<String> = Vec::new();
    for path in first {
        if paths.contains(path) && !opened.contains(path) {
            opened.push(path.clone());
        }
    }
    let rest = paths.into_iter().filter(|path| !opened.contains(path)).collect();
    (opened, rest)
}

#[cfg(test)]
mod tests {
    use vet_protocol::UnavailableKind;

    use super::{Error, Unavailable, can_pass, error_can_pass, open_first};

    /// A reason of the semantic layer of `kind`, as the answer carries it.
    fn reason(kind: UnavailableKind) -> Unavailable {
        Unavailable {
            language: None,
            kind,
            label: String::new(),
            message: String::new(),
        }
    }

    #[test]
    fn a_reason_that_can_pass_is_asked_again_and_a_lasting_one_is_not() {
        for kind in [
            UnavailableKind::Incomplete,
            UnavailableKind::QueryFailed,
            UnavailableKind::ServerFailed,
        ] {
            assert!(can_pass(&reason(kind)), "{kind:?} can pass");
        }
        for kind in [
            UnavailableKind::Unsupported,
            UnavailableKind::ServerMissing,
            UnavailableKind::NotOffered,
            UnavailableKind::AnalysisLimited,
            UnavailableKind::SnapshotUnavailable,
        ] {
            assert!(!can_pass(&reason(kind)), "{kind:?} cannot pass");
        }
    }

    #[test]
    fn a_store_failure_can_pass_and_a_missing_file_cannot() {
        use vet_app::ports::StoreError;
        use vet_app::session::SessionError;

        assert!(error_can_pass(&Error::Store(StoreError("the store was busy".into()))));
        assert!(!error_can_pass(&Error::Session(SessionError::NotFound(
            "src/lib.rs".into()
        ))));
    }

    #[test]
    fn the_open_files_come_first_in_their_order_and_the_rest_keep_theirs() {
        let paths = vec![
            "a.rs".to_owned(),
            "b.rs".to_owned(),
            "c.rs".to_owned(),
            "d.rs".to_owned(),
        ];
        let (opened, rest) = open_first(paths, &["c.rs".to_owned(), "a.rs".to_owned()]);
        assert_eq!(opened, ["c.rs", "a.rs"]);
        assert_eq!(rest, ["b.rs", "d.rs"]);
    }

    #[test]
    fn an_open_file_not_in_the_changeset_is_ignored() {
        let paths = vec!["a.rs".to_owned(), "b.rs".to_owned()];
        let (opened, rest) = open_first(paths, &["gone.rs".to_owned(), "b.rs".to_owned()]);
        assert_eq!(opened, ["b.rs"]);
        assert_eq!(rest, ["a.rs"]);
    }
}
