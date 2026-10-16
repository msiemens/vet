//! What the background work that follows the state of a project should do. Each kind of such work
//! has a function from the state to a key: none while it should not run, otherwise what its result
//! depends on. A run is started when the key it is wanted for differs from the one it last ran
//! for, and stopped when no key is wanted. A run that ended is not started again for the same key,
//! so only a change of the state or a rerun starts it again.
//!
//! This is the decision only; `reconcile` carries it out.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::sync::{MutexGuard, OwnedMutexGuard, watch};
use vet_app::ports::{Clock, Watcher};
use vet_app::settle::{SettleFlush, Settling};
use vet_app::supervise::{Supervisor, Trigger};

use crate::mutex::lock;

/// What asked for a sync.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Reason {
    /// The pull request was opened.
    Opened,
    /// The credentials of its host changed.
    Accounts,
    /// The viewer asked.
    Requested,
    /// A review or drafts were posted to it.
    Posted,
    /// A fetch ran.
    Fetched,
}

/// How a sync went.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum SyncOutcome {
    /// The platform was asked and the threads are in step with it. `told` says whether the
    /// subscribers were told that threads changed.
    Synced { told: bool },
    /// The changeset on view is no pull request.
    NotPullRequest,
    /// Another changeset was on view by the time the platform answered, so nothing was synced.
    SourceChanged,
    /// The platform could not be asked or failed, or the project could not be scanned or closed.
    /// The message is fit to show.
    Failed(String),
}

/// The syncs of one project: at most one runs and one waits, whoever asked. They are never aborted
/// for a newer one and end by themselves when the project closes.
pub(crate) type Syncs = Trigger<Vec<Reason>, SyncOutcome>;

/// Who asked for a fetch: what its follow-ups still do when no ref moved. A fetch several asked
/// for is the strongest's.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Asked {
    /// The timer: a fetch that moved a ref is announced and followed; either way the comments of
    /// the pull request on view are synced, which change without a push.
    Timer,
    /// The viewer, through `Request::Fetch`: the fetch is reported whenever a remote was reached.
    Viewer,
}

/// The fetches of one project, one at a time. A request that comes while a fetch is on its way
/// joins it, and the fetch is the strongest asker's; the fetch is not a `Trigger` since a later
/// request is answered by a run that began before it, the refs being what is wanted. Once the
/// fetch has told who asked (`Sealing::asked`) a request waits for it and asks again, so no
/// asker's follow-ups are missed. A fetch writes review state, so it is never aborted for a newer
/// request and ends by itself when the project closes.
#[derive(Clone)]
pub(crate) struct Fetches(Arc<Fetching>);

struct Fetching {
    supervisor: Supervisor,
    kind: &'static str,
    run: Box<dyn Fn(Sealing) -> BoxFuture<'static, ()> + Send + Sync>,
    current: Mutex<Option<Current>>,
    /// Counts the fetches that ended, so the countdown to the next one starts over.
    ended: watch::Sender<u32>,
}

struct Current {
    asked: Asked,
    sealed: bool,
    done: watch::Sender<bool>,
}

/// How a request stands with the fetch.
enum Began {
    /// Joined or started a fetch, which answers it when it is done.
    Answered(watch::Receiver<bool>),
    /// A fetch is past taking askers: wait for it, then ask again.
    Later(watch::Receiver<bool>),
}

/// Held by the run of a fetch to learn who asked.
pub(crate) struct Sealing(Arc<Fetching>);

impl Sealing {
    /// The strongest of those who asked. A request after this gets a fetch of its own.
    pub(crate) fn asked(&self) -> Asked {
        let mut slot = lock(&self.0.current);
        let current = slot.as_mut().expect("a fetch that runs is current");
        current.sealed = true;
        current.asked
    }
}

/// Clears the current fetch when its run is over, however it ends; waiting requests are answered
/// only by a run that finished.
struct Over {
    fetching: Arc<Fetching>,
    finished: bool,
}

impl Drop for Over {
    fn drop(&mut self) {
        let current = lock(&self.fetching.current).take();
        if self.finished
            && let Some(current) = current
        {
            current.done.send_replace(true);
            self.fetching.ended.send_modify(|ended| *ended = ended.wrapping_add(1));
        }
    }
}

impl Fetches {
    /// Fetches whose runs are `run`, each counted under `kind`.
    pub(crate) fn new(
        supervisor: Supervisor,
        kind: &'static str,
        run: impl Fn(Sealing) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Fetching {
            supervisor,
            kind,
            run: Box::new(run),
            current: Mutex::new(None),
            ended: watch::channel(0).0,
        }))
    }

    /// Fetches for `asked`: joins the fetch on its way or starts one, and answers when it is done.
    /// False if the project closed before that.
    pub(crate) async fn fetch(&self, asked: Asked) -> bool {
        loop {
            match self.begin(asked) {
                Began::Answered(mut done) => return done.wait_for(|done| *done).await.is_ok(),
                Began::Later(mut done) => {
                    if done.wait_for(|done| *done).await.is_err() {
                        return false;
                    }
                }
            }
        }
    }

    fn begin(&self, asked: Asked) -> Began {
        let done = {
            let mut slot = lock(&self.0.current);
            match slot.as_mut() {
                Some(current) if current.sealed => return Began::Later(current.done.subscribe()),
                Some(current) => {
                    current.asked = current.asked.max(asked);
                    return Began::Answered(current.done.subscribe());
                }
                None => {
                    let (done, receiver) = watch::channel(false);
                    *slot = Some(Current {
                        asked,
                        sealed: false,
                        done,
                    });
                    receiver
                }
            }
        };
        self.spawn();
        Began::Answered(done)
    }

    fn spawn(&self) {
        let fetching = self.0.clone();
        let over = Over {
            fetching: fetching.clone(),
            finished: false,
        };
        let work = fetching
            .supervisor
            .track(fetching.kind, (fetching.run)(Sealing(fetching.clone())));
        fetching.supervisor.cooperate(fetching.kind, async move {
            let mut over = over;
            work.await;
            over.finished = true;
        });
    }

    /// Fetches now and then once `interval` has passed since the last fetch ended, whoever asked
    /// for it, until the project closes.
    pub(crate) async fn every(self, interval: Duration) {
        let mut ended = self.0.ended.subscribe();
        while self.fetch(Asked::Timer).await {
            ended.borrow_and_update();
            while tokio::time::timeout(interval, ended.changed()).await.is_ok() {}
        }
    }
}

/// The inputs that are counters: each change of one is a change of the state the work depends on.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Inputs {
    /// Bumped when a connection shows or leaves the review screen, so the loop looks again.
    screens: u32,
    /// Bumped when the trust that decides what analysis may execute changed.
    policy: u32,
    /// Bumped when a language server was replaced.
    servers: u32,
    /// Bumped by the viewer asking for the problems again.
    problems: u32,
    /// Bumped by an action on a run, after which the checks are read again.
    checks: u32,
    /// Bumped when the credentials of the host of the pull request on view changed.
    accounts: u32,
    /// Bumped each time a copy of the working tree's content ahead of a review is asked for.
    copies: u32,
    /// Whether that copy is still wanted: a tree that settles again withdraws it, since it would
    /// be a copy of an older tree.
    copying: bool,
}

impl Inputs {
    fn ask_for_copy(&mut self) {
        self.copies = self.copies.wrapping_add(1);
        self.copying = true;
    }
}

/// What a run of a keyed kind was started for.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: u32,
    policy: u32,
    servers: u32,
    /// Which asking for a rerun the run answers; zero for a kind that cannot be asked for one.
    rerun: u32,
    /// Which change of the accounts the run answers; only the checks depend on it.
    accounts: u32,
}

/// The key each keyed kind last ran for, none while it was not wanted.
#[derive(Default)]
struct Ran {
    retire: Option<Key>,
    servers: Option<Key>,
    symbols: Option<Key>,
    problems: Option<Key>,
    checks: Option<Key>,
    copy: Option<Key>,
}

/// What to do with the runs of one kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Change {
    /// The run, if one is alive, is the one wanted, or it ended for the key wanted.
    Keep,
    /// Start a run for this revision, replacing an older one.
    Start(u32),
    /// Nothing is wanted: end the run.
    Stop,
}

/// What the project is in, as far as the keyed work depends on it.
#[derive(Clone, Copy, Default)]
pub(crate) struct State {
    /// The revision on view; none until the project has scanned.
    pub(crate) revision: Option<u32>,
    /// Whether a connection shows the review screen.
    pub(crate) shown: bool,
    /// Whether the changeset on view is a pull request.
    pub(crate) pull_request: bool,
}

/// What `Reconciliation::decide` found.
pub(crate) struct Decision {
    pub(crate) retire: Change,
    pub(crate) servers: Change,
    pub(crate) symbols: Change,
    pub(crate) problems: Change,
    pub(crate) checks: Change,
    pub(crate) copy: Change,
    /// Whether a started checks run answers an action on a run, so it reads once more later.
    pub(crate) checks_after_action: bool,
}

/// What the settle loop, which lives as long as the project, shows of its work: the state of a
/// project is not at rest while any of it is set. It is read by those who wait for quiet and
/// written by the loop.
#[derive(Default)]
pub(crate) struct Settle {
    /// Edits of the working tree wait to settle, as the subscribers were last told.
    edits: Arc<AtomicBool>,
    /// A ref write waits for its window before the branch on view is looked at again.
    refs: Arc<AtomicBool>,
    /// The loop handles a result right now.
    busy: Arc<AtomicBool>,
    /// A client's request to reconcile the working tree now, taken by the loop.
    flush: Arc<SettleFlush>,
}

impl Settle {
    /// Whether edits of the working tree wait to settle into a snapshot.
    pub(crate) fn edits_waiting(&self) -> bool {
        self.edits.load(Ordering::SeqCst)
    }

    pub(crate) fn set_edits_waiting(&self, waiting: bool) {
        self.edits.store(waiting, Ordering::SeqCst);
    }

    /// Asks the loop to look at the working tree as it is now.
    pub(crate) fn request_flush(&self) {
        self.flush.request();
    }

    /// Whether the loop has work: edits or a ref write waiting to settle, a result it is handling,
    /// or a reconcile asked for that it has not made.
    pub(crate) fn busy(&self) -> bool {
        self.edits_waiting()
            || self.refs.load(Ordering::SeqCst)
            || self.busy.load(Ordering::SeqCst)
            || self.flush.requested()
    }

    /// `settling`, reporting to this state and taking its flush requests.
    pub(crate) fn report<W, C>(&self, settling: Settling<W, C>) -> Settling<W, C>
    where
        W: Watcher,
        C: Clock,
    {
        settling
            .refs_waiting(self.refs.clone())
            .reporting_busy(self.busy.clone())
            .flushable(self.flush.clone())
    }
}

/// What the supervisor keeps running for the state of a project. The mechanism that holds the runs
/// is `vet_app::supervise::Supervisor`.
#[derive(Default)]
pub(crate) struct Policy {
    inputs: watch::Sender<Inputs>,
    ran: tokio::sync::Mutex<Ran>,
    starts: Starts,
    settle: Settle,
}

impl Policy {
    pub(crate) fn settle(&self) -> &Settle {
        &self.settle
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Inputs> {
        self.inputs.subscribe()
    }

    /// A connection showed or left the review screen. A review screen that comes up asks for the
    /// copy ahead, since the queue copied nothing.
    pub(crate) fn screen_changed(&self, shown: bool) {
        self.inputs.send_modify(|inputs| {
            inputs.screens = inputs.screens.wrapping_add(1);
            if shown {
                inputs.ask_for_copy();
            }
        });
    }

    /// A new snapshot of the working tree settled: a copy of the tree before it is of no use.
    pub(crate) fn tree_settled(&self) {
        self.inputs.send_modify(|inputs| inputs.copying = false);
    }

    /// The settled tree stayed as it is for the quiet period, so its content may be copied.
    pub(crate) fn tree_quiet(&self) {
        self.inputs.send_modify(Inputs::ask_for_copy);
    }

    /// What analysis may execute changed.
    pub(crate) fn policy_changed(&self) {
        self.inputs
            .send_modify(|inputs| inputs.policy = inputs.policy.wrapping_add(1));
    }

    /// A language server was replaced, so what the analysis found of it may differ.
    pub(crate) fn servers_replaced(&self) {
        self.inputs
            .send_modify(|inputs| inputs.servers = inputs.servers.wrapping_add(1));
    }

    /// The viewer asked for the problems of the changeset on view again.
    pub(crate) fn rerun_problems(&self) {
        self.inputs
            .send_modify(|inputs| inputs.problems = inputs.problems.wrapping_add(1));
    }

    /// An action on a run changed the checks of the changeset on view.
    pub(crate) fn rerun_checks(&self) {
        self.inputs
            .send_modify(|inputs| inputs.checks = inputs.checks.wrapping_add(1));
    }

    /// The credentials the checks are read with changed.
    pub(crate) fn accounts_changed(&self) {
        self.inputs
            .send_modify(|inputs| inputs.accounts = inputs.accounts.wrapping_add(1));
    }

    /// The turns for starting servers, of which one is taken at a time.
    pub(crate) fn starts(&self) -> &Starts {
        &self.starts
    }

    /// Takes the turn to decide: whoever holds it starts and stops what it decided before the next
    /// decision is made, so two reconciliations never both start the same run.
    pub(crate) async fn reconciling(&self) -> Reconciliation<'_> {
        Reconciliation {
            ran: self.ran.lock().await,
            inputs: *self.inputs.borrow(),
        }
    }
}

/// The server starts of a project queue up: a start waits for the one before it to end, and of
/// several that wait only the latest runs.
#[derive(Default)]
pub(crate) struct Starts {
    latest: AtomicU32,
    running: Arc<tokio::sync::Mutex<()>>,
}

impl Starts {
    /// Queues a start and returns its ticket.
    pub(crate) fn queue(&self) -> u32 {
        self.latest.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    }

    /// Drops the starts that wait.
    pub(crate) fn withdraw(&self) {
        self.latest.fetch_add(1, Ordering::SeqCst);
    }

    /// Waits until no start runs. None if a later start was queued or the waiting ones were
    /// withdrawn meanwhile, otherwise the turn to start, held until dropped.
    pub(crate) async fn turn(&self, ticket: u32) -> Option<OwnedMutexGuard<()>> {
        let turn = self.running.clone().lock_owned().await;
        (self.latest.load(Ordering::SeqCst) == ticket).then_some(turn)
    }
}

/// One turn of deciding.
pub(crate) struct Reconciliation<'a> {
    ran: MutexGuard<'a, Ran>,
    inputs: Inputs,
}

impl Reconciliation<'_> {
    /// What to do with each keyed kind in `state`. The servers and the symbols and problems analysis
    /// are for the review screen: they depend on the revision on view, the policy the servers run
    /// under and replaced servers. Retiring the servers that run under a policy not allowed any more
    /// is for every screen, since a question on any of them may have started one. The checks are
    /// for the pull request on view, on any screen. The copy ahead is for the review screen, when
    /// the quiet tree or the screen coming up asked for it, until the next tree settles.
    pub(crate) fn decide(&mut self, state: State) -> Decision {
        let key = |revision| Key {
            revision,
            policy: self.inputs.policy,
            servers: 0,
            rerun: 0,
            accounts: 0,
        };
        let retire = state.revision.map(key);
        let servers = state.revision.filter(|_| state.shown).map(|revision| Key {
            servers: self.inputs.servers,
            ..key(revision)
        });
        let problems = servers.map(|key| Key {
            rerun: self.inputs.problems,
            ..key
        });
        let checks = state.revision.filter(|_| state.pull_request).map(|revision| Key {
            policy: 0,
            rerun: self.inputs.checks,
            accounts: self.inputs.accounts,
            ..key(revision)
        });
        let copy = state
            .revision
            .filter(|_| state.shown && self.inputs.copying)
            .map(|_| Key {
                rerun: self.inputs.copies,
                ..key(0)
            });
        let checks_after_action = matches!(
            (self.ran.checks, checks),
            (Some(ran), Some(wanted)) if ran.revision == wanted.revision && ran.rerun != wanted.rerun
        );
        Decision {
            retire: change(&mut self.ran.retire, retire),
            servers: change(&mut self.ran.servers, servers),
            symbols: change(&mut self.ran.symbols, servers),
            problems: change(&mut self.ran.problems, problems),
            checks: change(&mut self.ran.checks, checks),
            copy: change(&mut self.ran.copy, copy),
            checks_after_action,
        }
    }
}

/// Remembers `wanted` as the key `ran` last ran for, and says what that asks of the run.
fn change(ran: &mut Option<Key>, wanted: Option<Key>) -> Change {
    if *ran == wanted {
        return Change::Keep;
    }
    *ran = wanted;
    match wanted {
        Some(key) => Change::Start(key.revision),
        None => Change::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_view(revision: u32, shown: bool, pull_request: bool) -> State {
        State {
            revision: Some(revision),
            shown,
            pull_request,
        }
    }

    #[tokio::test]
    async fn a_run_is_started_once_per_key_and_again_when_the_key_or_a_rerun_changes() {
        let policy = Policy::default();
        let decide = async |revision: Option<u32>| {
            let state = State {
                revision,
                shown: revision.is_some(),
                pull_request: false,
            };
            let decision = policy.reconciling().await.decide(state);
            (decision.symbols, decision.problems)
        };

        assert_eq!(decide(Some(3)).await, (Change::Start(3), Change::Start(3)));
        assert_eq!(decide(Some(3)).await, (Change::Keep, Change::Keep));
        assert_eq!(decide(Some(4)).await, (Change::Start(4), Change::Start(4)));

        policy.rerun_problems();
        assert_eq!(decide(Some(4)).await, (Change::Keep, Change::Start(4)));
        policy.policy_changed();
        assert_eq!(decide(Some(4)).await, (Change::Start(4), Change::Start(4)));

        // Nothing is wanted while the review screen is away, and what comes back runs again.
        assert_eq!(decide(None).await, (Change::Stop, Change::Stop));
        assert_eq!(decide(None).await, (Change::Keep, Change::Keep));
        assert_eq!(decide(Some(4)).await, (Change::Start(4), Change::Start(4)));
    }

    #[tokio::test]
    async fn retire_servers_and_checks_each_follow_their_own_inputs() {
        let policy = Policy::default();
        let decide = async |state: State| {
            let decision = policy.reconciling().await.decide(state);
            (decision.retire, decision.servers, decision.checks)
        };
        let (start, keep) = (Change::Start(1), Change::Keep);

        // Retire follows the revision on any screen, servers need the screen, checks a pull request.
        assert_eq!(decide(on_view(1, false, false)).await, (start, keep, keep));
        assert_eq!(decide(on_view(1, true, true)).await, (keep, start, start));
        assert_eq!(decide(on_view(1, false, true)).await, (keep, Change::Stop, keep));

        // Trust moves retire and servers, not the checks; replaced servers move only the servers.
        assert_eq!(decide(on_view(1, true, true)).await, (keep, start, keep));
        policy.policy_changed();
        assert_eq!(decide(on_view(1, true, true)).await, (start, start, keep));
        policy.servers_replaced();
        assert_eq!(decide(on_view(1, true, true)).await, (keep, start, keep));
        policy.rerun_checks();
        assert_eq!(decide(on_view(1, true, true)).await, (keep, keep, start));

        // An accounts change moves the checks alone, and they are read as plainly as for a revision.
        policy.accounts_changed();
        assert_eq!(decide(on_view(1, true, true)).await, (keep, keep, start));

        // A working tree wants no checks.
        assert_eq!(decide(on_view(1, true, false)).await.2, Change::Stop);
    }

    #[tokio::test]
    async fn the_checks_read_once_more_only_for_a_rerun_not_for_a_new_revision() {
        let policy = Policy::default();
        let decide = async |revision| {
            let decision = policy.reconciling().await.decide(on_view(revision, false, true));
            (decision.checks, decision.checks_after_action)
        };

        assert_eq!(decide(1).await, (Change::Start(1), false));
        policy.rerun_checks();
        assert_eq!(decide(1).await, (Change::Start(1), true));
        assert_eq!(decide(2).await, (Change::Start(2), false));
        policy.accounts_changed();
        assert_eq!(decide(2).await, (Change::Start(2), false));
    }

    #[tokio::test]
    async fn the_copy_is_asked_for_by_a_quiet_tree_or_a_screen_coming_up_and_only_while_shown() {
        let policy = Policy::default();
        let decide = async |shown: bool| {
            let state = State {
                revision: Some(1),
                shown,
                pull_request: false,
            };
            policy.reconciling().await.decide(state).copy
        };

        // The queue is on screen: a quiet tree asks, and nothing is copied.
        policy.tree_quiet();
        assert_eq!(decide(false).await, Change::Keep);
        // The screen that comes up copies, and what was asked for before is the same copy.
        policy.screen_changed(true);
        assert_eq!(decide(true).await, Change::Start(0));
        assert_eq!(decide(true).await, Change::Keep);

        // A tree that settles ends the copy of the one before; quiet again copies the new one.
        policy.tree_settled();
        assert_eq!(decide(true).await, Change::Stop);
        policy.tree_quiet();
        assert_eq!(decide(true).await, Change::Start(0));

        // Leaving the screen ends the copy, and a screen leaving asks for none.
        policy.screen_changed(false);
        assert_eq!(decide(false).await, Change::Stop);
        assert_eq!(decide(false).await, Change::Keep);
    }

    #[tokio::test]
    async fn of_several_waiting_starts_only_the_latest_takes_its_turn() {
        let starts = Starts::default();
        let (first, second, third) = (starts.queue(), starts.queue(), starts.queue());

        assert!(starts.turn(first).await.is_none());
        assert!(starts.turn(second).await.is_none());
        let turn = starts.turn(third).await.expect("the latest start runs");

        let fourth = starts.queue();
        let waiting = starts.turn(fourth);
        tokio::pin!(waiting);
        assert!(
            futures::poll!(&mut waiting).is_pending(),
            "a start ran while another did"
        );
        drop(turn);
        assert!(waiting.await.is_some());

        let fifth = starts.queue();
        starts.withdraw();
        assert!(starts.turn(fifth).await.is_none());
    }
}
