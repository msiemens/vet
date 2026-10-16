//! What the background work that follows the state of a project should do. Each kind of such work
//! has a function from the state to a key: none while it should not run, otherwise what its result
//! depends on. A run is started when the key it is wanted for differs from the one it last ran
//! for, and stopped when no key is wanted. A run that ended is not started again for the same key,
//! so only a change of the state or a rerun starts it again.
//!
//! This is the decision only; `reconcile` carries it out.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::{MutexGuard, OwnedMutexGuard, watch};

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
    /// Whether a started checks run answers an action on a run, so it reads once more later.
    pub(crate) checks_after_action: bool,
}

/// What the supervisor keeps running for the state of a project. The mechanism that holds the runs
/// is `vet_app::supervise::Supervisor`.
#[derive(Default)]
pub(crate) struct Policy {
    inputs: watch::Sender<Inputs>,
    ran: tokio::sync::Mutex<Ran>,
    starts: Starts,
}

impl Policy {
    pub(crate) fn subscribe(&self) -> watch::Receiver<Inputs> {
        self.inputs.subscribe()
    }

    /// A connection showed or left the review screen.
    pub(crate) fn screen_changed(&self) {
        self.inputs
            .send_modify(|inputs| inputs.screens = inputs.screens.wrapping_add(1));
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
    /// for the pull request on view, on any screen.
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
