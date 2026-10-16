//! Carries out what the supervisor's policy decides: starts and stops the runs that follow the
//! state of a project. It happens when an input changes, in a loop that lives as long as the
//! project, and after every request, so the work a request calls for has been decided before its
//! reply goes out.

use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use tokio::sync::watch;
use vet_app::changeset::Source;
use vet_app::supervise::Supervisor;

use crate::mutex::lock;
use crate::supervisor::{Change, Inputs, Policy, State};
use crate::{Open, checks, copy, problems, servers, symbols};

/// Starts the loop that reconciles the project once its first scan is done, and again whenever the
/// revision or an input changes. `entry` is how it reaches the project without keeping it open.
pub(crate) fn attach(
    supervisor: &Supervisor,
    policy: &Policy,
    entry: Arc<OnceLock<Weak<Open>>>,
    revision: watch::Receiver<u32>,
) {
    supervisor.start("supervising", keep(entry, revision, policy.subscribe()));
}

/// Starts, replaces and stops the keyed work so it matches the state now. Cheap, and never holds
/// its turn while a run goes on.
pub(crate) async fn reconcile(open: &Arc<Open>) {
    let mut turn = open.policy.reconciling().await;
    let state = match open.booted_now() {
        Some(booted) => State {
            revision: Some(booted.session.revision().await),
            shown: open.screens.shown(),
            pull_request: matches!(booted.session.source().await, Source::PullRequest { .. }),
        },
        None => State::default(),
    };
    let decision = turn.decide(state);
    match decision.retire {
        Change::Start(_) => servers::retire(open),
        Change::Stop => open.supervisor.stop(servers::RETIRE),
        Change::Keep => {}
    }
    match decision.servers {
        Change::Start(_) => servers::start(open),
        Change::Stop => open.policy.starts().withdraw(),
        Change::Keep => {}
    }
    match decision.checks {
        Change::Start(_) => checks::start(open, decision.checks_after_action),
        Change::Stop => open.supervisor.stop(checks::KIND),
        Change::Keep => {}
    }
    match decision.copy {
        Change::Start(_) => copy::start(open),
        Change::Stop => open.supervisor.stop(copy::KIND),
        Change::Keep => {}
    }
    match decision.symbols {
        Change::Start(revision) => symbols::start(open, revision),
        Change::Stop => open.supervisor.stop(symbols::KIND),
        Change::Keep => {}
    }
    match decision.problems {
        Change::Start(revision) => problems::start(open, revision),
        Change::Stop => open.supervisor.stop(problems::KIND),
        Change::Keep => {}
    }
}

/// Whether the project has no work: what its state calls for is started first, so no change of an
/// input falls between its being made and the loop reacting to it, then no run of any kind is
/// alive or waiting and nothing of the settle loop is pending.
pub(crate) async fn idle(open: &Arc<Open>) -> bool {
    reconcile(open).await;
    open.quiet()
}

/// Ends with the project.
async fn keep(
    entry: Arc<OnceLock<Weak<Open>>>,
    mut revision: watch::Receiver<u32>,
    mut inputs: watch::Receiver<Inputs>,
) {
    // The loop starts while the boot that attached it finishes, and the state it missed before the
    // project was booted is read once the boot is done.
    loop {
        match entry.get().and_then(Weak::upgrade) {
            Some(open) => {
                let _ = open.booted().await;
                break;
            }
            None => tokio::time::sleep(Duration::from_millis(1)).await,
        }
    }
    loop {
        if let Some(open) = entry.get().and_then(Weak::upgrade) {
            reconcile(&open).await;
        }
        tokio::select! {
            changed = revision.changed() => if changed.is_err() { return },
            changed = inputs.changed() => if changed.is_err() { return },
        }
    }
}

tokio::task_local! {
    /// The projects the request being answered looked up.
    static TOUCHED: Mutex<Vec<Arc<Open>>>;
}

/// Notes that the request being answered is about `open`, if one is being answered.
pub(crate) fn touched(open: &Arc<Open>) {
    let _ = TOUCHED.try_with(|touched| {
        let mut touched = lock(touched);
        if !touched.iter().any(|known| Arc::ptr_eq(known, open)) {
            touched.push(open.clone());
        }
    });
}

/// Answers a request with `answer`, then reconciles the projects it looked up, so the background
/// work its changes call for is decided before the reply goes out.
pub(crate) async fn reconciling<T>(answer: impl Future<Output = T>) -> T {
    TOUCHED
        .scope(Mutex::default(), async {
            let answered = answer.await;
            let looked_up = TOUCHED.with(|touched| std::mem::take(&mut *lock(touched)));
            for open in looked_up {
                reconcile(&open).await;
            }
            answered
        })
        .await
}
