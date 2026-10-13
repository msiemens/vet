//! The checks a platform reports for a commit: CI runs and the statuses services set.

use super::model::CiState;

/// How far a check or step has come, which its [`CiState`] does not say: a pending check may
/// still wait for a runner, and a neutral one may have been cancelled.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Queued,
    Running,
    Finished,
    Cancelled,
}

/// The run (a GitHub workflow run, an Azure DevOps pipeline run) that a check is a job of.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Run {
    /// Names the run to the platform, for [`super::PipelineAction`].
    pub id: String,
    pub name: String,
    /// What distinguishes the run from others of its pipeline: its number and why it ran.
    pub detail: String,
    /// 1 for the first time the run was executed, higher after a re-run.
    pub attempt: u32,
}

/// One check of a commit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Check {
    /// Names the check to the platform, for [`super::Platform::check_log`].
    pub id: String,
    pub name: String,
    pub state: CiState,
    pub phase: Phase,
    /// The run this check is a job of; none for a status a service set.
    pub run: Option<Run>,
    /// Where the platform shows it.
    pub url: Option<String>,
    /// Unix seconds.
    pub started: Option<i64>,
    /// Unix seconds; absent while it runs.
    pub finished: Option<i64>,
    /// The platform can give the log of this check.
    pub log: bool,
}

/// One step of a job, or one task of a stage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    pub name: String,
    pub state: CiState,
    pub phase: Phase,
    /// Unix seconds.
    pub started: Option<i64>,
    /// Unix seconds; absent while it runs.
    pub finished: Option<i64>,
}

/// What the viewer may ask of a platform's runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PipelineAction {
    /// Runs the failed jobs of the run again.
    RerunFailed {
        run: String,
    },
    /// Runs every job of a finished run again.
    RerunAll {
        run: String,
    },
    /// Runs one job again ([`Check::id`]).
    RerunJob {
        check: String,
    },
    Cancel {
        run: String,
    },
}

/// A run with what its jobs say of it together.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RunSummary {
    pub run: Run,
    pub state: CiState,
    pub phase: Phase,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommitChecks {
    pub commit: String,
    pub checks: Vec<Check>,
    /// Where the platform lists the commit's checks.
    pub url: Option<String>,
}

impl CommitChecks {
    /// The combined state: running if any check runs, else failing if any failed, else waiting for
    /// approval if any check has not run for that, else passing if any passed. A failure shows
    /// before a missing approval, because it is settled; a pass does not hide a check that has not
    /// run. None for a commit without checks.
    pub fn state(&self) -> Option<CiState> {
        combined(self.checks.iter().map(|c| c.state))
    }

    /// The runs of the checks in the order their first job comes, each with the state and phase
    /// its jobs make together. A run is running while any job is, and queued while all wait; a
    /// run with some job started and the rest waiting is running.
    pub fn runs(&self) -> Vec<RunSummary> {
        let mut found: Vec<(Run, Vec<&Check>)> = Vec::new();
        for check in &self.checks {
            let Some(run) = &check.run else { continue };
            match found.iter_mut().find(|(r, _)| r.id == run.id) {
                Some((_, jobs)) => jobs.push(check),
                None => found.push((run.clone(), vec![check])),
            }
        }
        found
            .into_iter()
            .map(|(run, jobs)| {
                let any = |wanted: Phase| jobs.iter().any(|j| j.phase == wanted);
                let phase =
                    if any(Phase::Running) || (any(Phase::Queued) && jobs.iter().any(|j| j.phase != Phase::Queued)) {
                        Phase::Running
                    } else if any(Phase::Queued) {
                        Phase::Queued
                    } else if jobs
                        .iter()
                        .all(|j| matches!(j.phase, Phase::Cancelled | Phase::Finished))
                        && any(Phase::Cancelled)
                    {
                        Phase::Cancelled
                    } else {
                        Phase::Finished
                    };
                let state = combined(jobs.iter().map(|j| j.state)).unwrap_or(CiState::Skipped);
                RunSummary { run, state, phase }
            })
            .collect()
    }

    /// The checks will not change: there are some and none runs or waits for approval, which can
    /// be given any time. A commit's checks may not have been created yet when it was pushed a
    /// moment ago, so none is not final.
    pub fn is_final(&self) -> bool {
        !self.checks.is_empty()
            && self
                .checks
                .iter()
                .all(|c| !matches!(c.state, CiState::Pending | CiState::NeedsApproval))
    }
}

/// What `states` make together: running if any runs, else failing if any failed, else waiting for
/// approval if any has not run for that, else passing if any passed. None for no state at all.
fn combined(states: impl Iterator<Item = CiState> + Clone) -> Option<CiState> {
    let any = |wanted: CiState| states.clone().any(|s| s == wanted);
    if states.clone().next().is_none() {
        None
    } else if any(CiState::Pending) {
        Some(CiState::Pending)
    } else if any(CiState::Failure) {
        Some(CiState::Failure)
    } else if any(CiState::NeedsApproval) {
        Some(CiState::NeedsApproval)
    } else if any(CiState::Success) {
        Some(CiState::Success)
    } else if any(CiState::Neutral) {
        Some(CiState::Neutral)
    } else {
        Some(CiState::Skipped)
    }
}

/// Where a check's log shows what failed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LogFailure {
    /// 0-based line of the log.
    pub line: u32,
    /// The step that failed, if the platform says which.
    pub step: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckLog {
    pub text: String,
    pub failure: Option<LogFailure>,
}

#[cfg(test)]
mod tests {
    use CiState::*;

    use super::*;

    fn check(state: CiState) -> Check {
        Check {
            id: "1".to_owned(),
            name: "build".to_owned(),
            state,
            phase: Phase::Finished,
            run: None,
            url: None,
            started: None,
            finished: None,
            log: false,
        }
    }

    fn job(run: &str, phase: Phase, state: CiState) -> Check {
        Check {
            phase,
            run: Some(Run {
                id: run.to_owned(),
                name: format!("workflow {run}"),
                detail: String::new(),
                attempt: 1,
            }),
            ..check(state)
        }
    }

    fn with_jobs(jobs: Vec<Check>) -> CommitChecks {
        CommitChecks {
            commit: "abc".to_owned(),
            checks: jobs,
            url: None,
        }
    }

    fn commit(states: &[CiState]) -> CommitChecks {
        CommitChecks {
            commit: "abc".to_owned(),
            checks: states.iter().copied().map(check).collect(),
            url: None,
        }
    }

    #[test]
    fn a_running_check_outweighs_a_failed_one_and_a_failure_outweighs_success() {
        assert_eq!(commit(&[]).state(), None);
        assert_eq!(commit(&[Success, Failure, Pending]).state(), Some(Pending));
        assert_eq!(commit(&[Success, Failure, Skipped]).state(), Some(Failure));
        assert_eq!(commit(&[Skipped, Success, Neutral]).state(), Some(Success));
        assert_eq!(commit(&[Success, NeedsApproval, Skipped]).state(), Some(NeedsApproval));
        assert_eq!(commit(&[Success, NeedsApproval, Failure]).state(), Some(Failure));
        assert_eq!(commit(&[NeedsApproval, Pending]).state(), Some(Pending));
        assert_eq!(commit(&[Skipped, Neutral]).state(), Some(Neutral));
        assert_eq!(commit(&[Skipped]).state(), Some(Skipped));
    }

    #[test]
    fn jobs_are_grouped_by_run_in_the_order_the_first_job_of_each_comes() {
        let checks = with_jobs(vec![
            job("b", Phase::Finished, CiState::Success),
            job("a", Phase::Finished, CiState::Failure),
            check(CiState::Success),
            job("b", Phase::Finished, CiState::Success),
        ]);
        let runs = checks.runs();
        assert_eq!(runs.iter().map(|r| r.run.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(runs[0].state, CiState::Success);
        assert_eq!(runs[1].state, CiState::Failure);
    }

    #[test]
    fn a_run_waits_only_while_no_job_has_started_and_is_cancelled_when_its_stopped_jobs_were() {
        let phase = |jobs: Vec<Check>| with_jobs(jobs).runs()[0].phase;
        let queued = || job("a", Phase::Queued, CiState::Pending);
        assert_eq!(phase(vec![queued(), queued()]), Phase::Queued);
        assert_eq!(
            phase(vec![queued(), job("a", Phase::Finished, CiState::Success)]),
            Phase::Running
        );
        assert_eq!(
            phase(vec![queued(), job("a", Phase::Running, CiState::Pending)]),
            Phase::Running
        );
        assert_eq!(
            phase(vec![
                job("a", Phase::Cancelled, CiState::Neutral),
                job("a", Phase::Finished, CiState::Success)
            ]),
            Phase::Cancelled
        );
        assert_eq!(
            phase(vec![job("a", Phase::Finished, CiState::Failure)]),
            Phase::Finished
        );
    }

    #[test]
    fn checks_are_final_once_there_are_some_and_none_runs() {
        assert!(!commit(&[]).is_final());
        assert!(!commit(&[Success, Pending]).is_final());
        assert!(commit(&[Success, Failure]).is_final());
        assert!(!commit(&[Success, NeedsApproval]).is_final());
    }
}
