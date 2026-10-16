//! What the background work that follows the state of a project should do. Each kind of such work
//! has a function from the state to a key: none while it should not run, otherwise what its result
//! depends on. A run is started when the key it is wanted for differs from the one it last ran
//! for, and stopped when no key is wanted. A run that ended is not started again for the same key,
//! so only a change of the state or a rerun starts it again.
//!
//! This is the decision only; `reconcile` carries it out.

use tokio::sync::{MutexGuard, watch};

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
}

/// What a run of a keyed kind was started for.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: u32,
    policy: u32,
    servers: u32,
    /// Which asking for a rerun the run answers; zero for a kind that cannot be asked for one.
    rerun: u32,
}

/// The key each keyed kind last ran for, none while it was not wanted.
#[derive(Default)]
struct Ran {
    symbols: Option<Key>,
    problems: Option<Key>,
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

/// What `Reconciliation::decide` found.
pub(crate) struct Decision {
    pub(crate) symbols: Change,
    pub(crate) problems: Change,
}

/// What the supervisor keeps running for the state of a project: the analysis of the changeset on
/// view. The mechanism that holds the runs is `vet_app::supervise::Supervisor`.
#[derive(Default)]
pub(crate) struct Policy {
    inputs: watch::Sender<Inputs>,
    ran: tokio::sync::Mutex<Ran>,
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

    /// Takes the turn to decide: whoever holds it starts and stops what it decided before the next
    /// decision is made, so two reconciliations never both start the same run.
    pub(crate) async fn reconciling(&self) -> Reconciliation<'_> {
        Reconciliation {
            ran: self.ran.lock().await,
            inputs: *self.inputs.borrow(),
        }
    }
}

/// One turn of deciding.
pub(crate) struct Reconciliation<'a> {
    ran: MutexGuard<'a, Ran>,
    inputs: Inputs,
}

impl Reconciliation<'_> {
    /// What to do with the symbols and the problems analysis, which are for the review screen: both
    /// depend on the revision on view and the policy the servers run under. `revision` is the one
    /// on view while the review screen is shown and the project scanned, none otherwise.
    pub(crate) fn decide(&mut self, revision: Option<u32>) -> Decision {
        let symbols = revision.map(|revision| Key {
            revision,
            policy: self.inputs.policy,
            servers: self.inputs.servers,
            rerun: 0,
        });
        let problems = symbols.map(|key| Key {
            rerun: self.inputs.problems,
            ..key
        });
        Decision {
            symbols: change(&mut self.ran.symbols, symbols),
            problems: change(&mut self.ran.problems, problems),
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

    #[tokio::test]
    async fn a_run_is_started_once_per_key_and_again_when_the_key_or_a_rerun_changes() {
        let policy = Policy::default();
        let decide = async |revision: Option<u32>| {
            let decision = policy.reconciling().await.decide(revision);
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
}
