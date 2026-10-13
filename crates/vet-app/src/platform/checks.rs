//! The checks a platform reports for a commit: CI runs and the statuses services set.

use super::model::CiState;

/// One check of a commit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Check {
    /// Names the check to the platform, for [`super::Platform::check_log`].
    pub id: String,
    pub name: String,
    pub state: CiState,
    /// Where the platform shows it.
    pub url: Option<String>,
    /// Unix seconds.
    pub started: Option<i64>,
    /// Unix seconds; absent while it runs.
    pub finished: Option<i64>,
    /// The platform can give the log of this check.
    pub log: bool,
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
        let any = |wanted: CiState| self.checks.iter().any(|c| c.state == wanted);
        if self.checks.is_empty() {
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
            url: None,
            started: None,
            finished: None,
            log: false,
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
    fn checks_are_final_once_there_are_some_and_none_runs() {
        assert!(!commit(&[]).is_final());
        assert!(!commit(&[Success, Pending]).is_final());
        assert!(commit(&[Success, Failure]).is_final());
        assert!(!commit(&[Success, NeedsApproval]).is_final());
    }
}
