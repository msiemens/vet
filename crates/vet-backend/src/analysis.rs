//! Which changed files the background analysis (symbols, problems) reads, in what order, and how a
//! failure that can pass is asked again once what it waits for is over.

#[cfg(debug_assertions)]
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::Mutex;
use std::time::Duration;

use futures::stream::{self, Stream, StreamExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

#[cfg(debug_assertions)]
use crate::mutex::lock;
use vet_app::analysis::open_first;
#[cfg(debug_assertions)]
use vet_app::ports::StoreError;
use vet_app::semantics::ServerRuntimes;
use vet_app::session::{FileView, SessionError, View};
use vet_core::review::Side;
use vet_protocol::{Response, Unavailable, UnavailableKind};
use vet_semantic::pool::Phase;
use vet_semantic::ports::BackendId;

use crate::Open;
use crate::error::Error;
use crate::open::Booted;

/// How many times one file's analysis is asked again while it fails for a reason that can pass,
/// before the failure is pushed as final.
const TRIES: usize = 3;
/// How long between two tries.
const RETRY_PAUSE: Duration = Duration::from_millis(500);

/// How long one wait for a file's server to finish working lasts at most; after it the file is
/// asked again whatever the server does.
const READY_WAIT: Duration = Duration::from_secs(10);

/// Both sides of the revision on view: what a comparison with the base asks of the servers.
pub(crate) const BOTH_SIDES: &[Side] = &[Side::Base, Side::Target];
/// The target side alone: what reading a file's own problems asks of the servers.
pub(crate) const TARGET_SIDE: &[Side] = &[Side::Target];

/// What a file waits for before it is asked again, from the weakest to the strongest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Again {
    /// Nothing can change the answer: a server that failed to start or stays stopped (the pool
    /// answers that failure for a while without trying again, and a restart by hand restarts the
    /// analysis), or a lasting reason — no server for its language, a project the analysis may
    /// not run in, the server does not offer the operation, no text to analyse (binary, too large,
    /// not a changed file), a snapshot that cannot be rebuilt.
    Never,
    /// A query that failed (the server may have been restarted): a pause is honest.
    Pause,
    /// A server at work on its own: asking again helps once it is at rest.
    AtRest,
}

/// What a reason the semantic layer gave for a file waits for.
fn again_for(reason: &Unavailable) -> Again {
    match reason.kind {
        UnavailableKind::Incomplete => Again::AtRest,
        UnavailableKind::QueryFailed => Again::Pause,
        _ => Again::Never,
    }
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

/// What the outcome of one file's analysis waits for before the file is asked again: the whole
/// answer failed, the comparison with the base could not be made, or a server was at work while it
/// answered.
fn again(outcome: &Result<Response, Error>) -> Again {
    match outcome {
        Err(error) if error_can_pass(error) => Again::Pause,
        Err(_) => Again::Never,
        Ok(Response::FileSymbols(symbols)) => symbols.gap.as_ref().map_or(Again::Never, again_for),
        Ok(Response::Diagnostics {
            unavailable,
            origin_unavailable,
            incomplete,
            ..
        }) => {
            if incomplete.is_some() {
                return Again::AtRest;
            }
            [unavailable, origin_unavailable]
                .into_iter()
                .flatten()
                .map(again_for)
                .max()
                .unwrap_or(Again::Never)
        }
        Ok(_) => Again::Never,
    }
}

/// The turns of one batch of files to analyse, and what one file waits for outside them: its
/// server. A file asks while it holds a turn; while its server starts, and while it waits to ask
/// again, it holds none, so files of other languages and files the cache can answer go on.
pub(crate) struct Turns {
    permits: Arc<Semaphore>,
    runtimes: watch::Receiver<ServerRuntimes>,
    /// The language of the file; none when no server serves it.
    language: Option<BackendId>,
    /// The sides of the revision on view the analysis of the file asks its server about.
    sides: &'static [Side],
    ready_wait: Duration,
}

impl Turns {
    pub(crate) fn new(
        permits: Arc<Semaphore>,
        runtimes: watch::Receiver<ServerRuntimes>,
        language: Option<BackendId>,
        sides: &'static [Side],
    ) -> Self {
        Self {
            permits,
            runtimes,
            language,
            sides,
            ready_wait: READY_WAIT,
        }
    }

    #[cfg(test)]
    fn waiting_at_most(mut self, ready_wait: Duration) -> Self {
        self.ready_wait = ready_wait;
        self
    }

    /// Whether the file's server has a runtime on one of its sides in a phase `is` accepts.
    fn phase_is(&self, runtimes: &ServerRuntimes, is: fn(&Phase) -> bool) -> bool {
        runtimes.runtimes.iter().any(|runtime| {
            Some(&runtime.language) == self.language.as_ref()
                && self.sides.contains(&runtime.side)
                && is(&runtime.phase)
        })
    }

    /// Resolves when the file's server is being started; never without a language, nor when the
    /// runtimes are no longer published.
    async fn starting(&self) {
        if self.language.is_some() {
            let mut runtimes = self.runtimes.clone();
            if runtimes
                .wait_for(|now| self.phase_is(now, |phase| matches!(phase, Phase::Starting)))
                .await
                .is_ok()
            {
                return;
            }
        }
        std::future::pending::<()>().await;
    }

    /// Waits until the file's server is neither starting nor indexing, at most the readiness wait.
    async fn at_rest(&self) {
        let mut runtimes = self.runtimes.clone();
        let _ = tokio::time::timeout(
            self.ready_wait,
            runtimes
                .wait_for(|now| !self.phase_is(now, |phase| matches!(phase, Phase::Starting | Phase::Indexing { .. }))),
        )
        .await;
    }

    /// `question`, asked under `permit`; the permit is given back as soon as the file's server
    /// shows it is being started, and the question is awaited without it.
    async fn asked<Fut: Future<Output = Result<Response, Error>>>(
        &self,
        permit: OwnedSemaphorePermit,
        question: Fut,
    ) -> Result<Response, Error> {
        let mut question = std::pin::pin!(question);
        let early = {
            let _permit = permit;
            tokio::select! {
                biased;
                answer = &mut question => Some(answer),
                () = self.starting() => None,
            }
        };
        match early {
            Some(answer) => answer,
            None => question.await,
        }
    }

    /// The next turn, after those who wait for one.
    async fn turn(&self) -> OwnedSemaphorePermit {
        take_turn(self.permits.clone()).await
    }
}

async fn take_turn(permits: Arc<Semaphore>) -> OwnedSemaphorePermit {
    permits
        .acquire_owned()
        .await
        .expect("the turns of a run are never closed")
}

/// `items`, each with the turn it took from `permits`, taken in order: the next item is yielded
/// when a turn is free.
pub(crate) fn in_turns<T>(
    items: impl IntoIterator<Item = T>,
    permits: Arc<Semaphore>,
) -> impl Stream<Item = (T, OwnedSemaphorePermit)> {
    stream::iter(items).then(move |item| {
        let permits = permits.clone();
        async move { (item, take_turn(permits).await) }
    })
}

/// `analyse`, asked up to [`TRIES`] times while it fails for a reason that can pass; the last
/// answer is returned. `first` is the turn the file took to begin; each try asks under a turn and
/// waits for what it needs outside one: a server at work is waited for until it is at rest, a
/// failed query pauses. Callers check the review screen and the revision before each file; a newer
/// run, a hidden screen or a closed project aborts the task, and with it a wait between two tries.
pub(crate) async fn retrying<F, Fut>(
    turns: &Turns,
    first: OwnedSemaphorePermit,
    mut analyse: F,
) -> Result<Response, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Response, Error>>,
{
    let mut permit = Some(first);
    for tried in 1..=TRIES {
        let turn = match permit.take() {
            Some(permit) => permit,
            None => turns.turn().await,
        };
        let answer = turns.asked(turn, analyse()).await;
        match again(&answer) {
            _ if tried == TRIES => return answer,
            Again::Never => return answer,
            Again::AtRest => turns.at_rest().await,
            Again::Pause => tokio::time::sleep(RETRY_PAUSE).await,
        }
    }
    unreachable!("the last try returns")
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
    let mut failures = lock(&open.analysis_failures);
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;
    use tokio::time::Instant;
    use vet_app::semantics::ServerRuntime;
    use vet_protocol::{DerivedInputs, FileSymbols, Response, UnavailableKind};

    use super::{
        Again, Arc, BOTH_SIDES, BackendId, Duration, Error, Phase, RETRY_PAUSE, Semaphore, ServerRuntimes, Side, Turns,
        Unavailable, again, retrying, watch,
    };

    /// A reason of the semantic layer of `kind`, as the answer carries it.
    fn reason(kind: UnavailableKind) -> Unavailable {
        Unavailable {
            language: None,
            kind,
            label: String::new(),
            message: String::new(),
        }
    }

    /// The answer for a file's diagnostics, with what the semantic layer said about them.
    fn diagnostics(
        unavailable: Option<UnavailableKind>,
        origin_unavailable: Option<UnavailableKind>,
        incomplete: bool,
    ) -> Result<Response, Error> {
        Ok(Response::Diagnostics {
            revision: 1,
            path: "src/lib.rs".into(),
            diagnostics: Vec::new(),
            count: 0,
            unavailable: unavailable.map(reason),
            origin_unavailable: origin_unavailable.map(reason),
            incomplete: incomplete.then(|| reason(UnavailableKind::Incomplete)),
        })
    }

    fn diagnostics_incomplete() -> Result<Response, Error> {
        diagnostics(None, None, true)
    }

    /// What the servers do when the rust server is `phase` on the target side.
    fn runtimes(phase: Phase) -> ServerRuntimes {
        ServerRuntimes {
            serial: 1,
            runtimes: vec![ServerRuntime {
                language: BackendId("rust".into()),
                side: Side::Target,
                phase,
            }],
            features: Vec::new(),
        }
    }

    /// The turns of a batch of one at a time, for a rust file; the permit is the one the file took.
    fn turns(runtimes: ServerRuntimes) -> (Turns, tokio::sync::OwnedSemaphorePermit, watch::Sender<ServerRuntimes>) {
        let permits = Arc::new(Semaphore::new(1));
        let first = permits.clone().try_acquire_owned().unwrap();
        let (sender, receiver) = watch::channel(runtimes);
        (
            Turns::new(permits, receiver, Some(BackendId("rust".into())), BOTH_SIDES),
            first,
            sender,
        )
    }

    #[test]
    fn each_answer_is_asked_again_after_what_it_waits_for() {
        use vet_app::ports::StoreError;
        use vet_app::session::SessionError;

        use UnavailableKind::{
            AnalysisLimited, Incomplete, NotOffered, QueryFailed, ServerFailed, ServerMissing, SnapshotUnavailable,
            Unsupported,
        };

        for kind in [
            ServerFailed,
            Unsupported,
            ServerMissing,
            NotOffered,
            AnalysisLimited,
            SnapshotUnavailable,
        ] {
            assert_eq!(again(&Ok(symbols_with_gap(kind))), Again::Never, "{kind:?}");
            assert_eq!(again(&diagnostics(Some(kind), None, false)), Again::Never, "{kind:?}");
        }
        assert_eq!(again(&Ok(symbols_with_gap(Incomplete))), Again::AtRest);
        assert_eq!(again(&Ok(symbols_with_gap(QueryFailed))), Again::Pause);
        assert_eq!(again(&diagnostics(None, None, false)), Again::Never);
        assert_eq!(again(&diagnostics_incomplete()), Again::AtRest);
        assert_eq!(again(&diagnostics(Some(QueryFailed), None, false)), Again::Pause);
        assert_eq!(again(&diagnostics(None, Some(Incomplete), false)), Again::AtRest);
        // The strongest wait of the two reasons wins.
        assert_eq!(
            again(&diagnostics(Some(QueryFailed), Some(Incomplete), false)),
            Again::AtRest
        );
        assert_eq!(
            again(&Err(Error::Store(StoreError("the store was busy".into())))),
            Again::Pause
        );
        assert_eq!(
            again(&Err(Error::Session(SessionError::NotFound("src/lib.rs".into())))),
            Again::Never
        );
    }

    /// The answer for a file whose symbols could not be determined, for `kind`.
    fn symbols_with_gap(kind: UnavailableKind) -> Response {
        Response::FileSymbols(FileSymbols {
            revision: 1,
            path: "src/lib.rs".into(),
            symbols: Vec::new(),
            context: Vec::new(),
            gap: Some(reason(kind)),
            inputs: DerivedInputs {
                analysis: String::new(),
                bounds: None,
                primitives: Vec::new(),
                hunks: Vec::new(),
                config: Vec::new(),
            },
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_server_is_answered_at_once() {
        let (turns, first, _runtimes) = turns(runtimes(Phase::Ready));
        let calls = AtomicUsize::new(0);
        let started = Instant::now();
        let answer = retrying(&turns, first, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(symbols_with_gap(UnavailableKind::ServerFailed))
        })
        .await;
        assert!(answer.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() < RETRY_PAUSE);
    }

    fn indexing() -> Phase {
        Phase::Indexing {
            done_of_total: None,
            detail: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_incomplete_answer_is_asked_again_as_soon_as_its_server_is_at_rest() {
        let (turns, first, runtimes_sender) = turns(runtimes(indexing()));
        let permits = turns.permits.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let calls = calls.clone();
            async move {
                retrying(&turns, first, || {
                    let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                    async move {
                        if first {
                            diagnostics_incomplete()
                        } else {
                            diagnostics(None, None, false)
                        }
                    }
                })
                .await
            }
        });

        tokio::time::sleep(RETRY_PAUSE / 10).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "asked again while the server was at work"
        );
        assert_eq!(permits.available_permits(), 1, "the wait held the turn");

        let ready = Instant::now();
        runtimes_sender.send_replace(runtimes(Phase::Ready));
        assert!(task.await.unwrap().is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(ready.elapsed() < RETRY_PAUSE / 2, "waited a pause, not for the server");
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_waiting_on_a_starting_server_gives_its_turn_back() {
        let (turns, first, _runtimes) = turns(runtimes(Phase::Starting));
        let permits = turns.permits.clone();
        let answer = Arc::new(Notify::new());
        let task = tokio::spawn({
            let answer = answer.clone();
            async move {
                retrying(&turns, first, || {
                    let answer = answer.clone();
                    async move {
                        answer.notified().await;
                        diagnostics(None, None, false)
                    }
                })
                .await
            }
        });

        tokio::time::sleep(RETRY_PAUSE / 5).await;
        assert_eq!(permits.available_permits(), 1, "the start held the turn");
        answer.notify_one();
        assert!(task.await.unwrap().is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_stays_at_work_is_waited_for_at_most_ready_wait_each_try() {
        let (turns, first, _runtimes) = turns(runtimes(indexing()));
        let turns = turns.waiting_at_most(Duration::from_millis(100));
        let calls = AtomicUsize::new(0);
        let started = Instant::now();
        let answer = retrying(&turns, first, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            diagnostics_incomplete()
        })
        .await;
        assert!(answer.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(200) && took < Duration::from_secs(2),
            "{took:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_store_failure_pauses_without_holding_a_turn() {
        use vet_app::ports::StoreError;

        let (turns, first, _runtimes) = turns(runtimes(Phase::Ready));
        let permits = turns.permits.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let calls = calls.clone();
            async move {
                retrying(&turns, first, || {
                    let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                    async move {
                        if first {
                            Err(Error::Store(StoreError("the store was busy".into())))
                        } else {
                            diagnostics(None, None, false)
                        }
                    }
                })
                .await
            }
        });

        tokio::time::sleep(RETRY_PAUSE / 5).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(permits.available_permits(), 1, "the pause held the turn");
        assert!(task.await.unwrap().is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
