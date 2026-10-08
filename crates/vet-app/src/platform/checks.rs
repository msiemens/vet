//! The checks a platform reports for a commit: CI runs and the statuses services set.

use super::model::{CiState, PlatformError};
use super::port::Platform;
use super::remote::RemoteKey;

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
    /// The combined state: running if any check runs, else failing if any failed, else passing if
    /// any passed. None for a commit without checks.
    pub fn state(&self) -> Option<CiState> {
        let any = |wanted: CiState| self.checks.iter().any(|c| c.state == wanted);
        if self.checks.is_empty() {
            None
        } else if any(CiState::Pending) {
            Some(CiState::Pending)
        } else if any(CiState::Failure) {
            Some(CiState::Failure)
        } else if any(CiState::Success) {
            Some(CiState::Success)
        } else if any(CiState::Neutral) {
            Some(CiState::Neutral)
        } else {
            Some(CiState::Skipped)
        }
    }

    /// The checks will not change: there are some and none runs. A commit's checks may not have
    /// been created yet when it was pushed a moment ago, so none is not final.
    pub fn is_final(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.state != CiState::Pending)
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

/// The checks of each of `commits` in `remote`, asked for together, in the order of `commits`. A
/// commit the platform cannot answer for is left out, and the first reason is returned beside the
/// rest, so one commit that was garbage collected does not hide the others.
pub async fn commits_checks<P: Platform>(
    platform: &P,
    remote: &RemoteKey,
    commits: &[String],
) -> (Vec<CommitChecks>, Option<PlatformError>) {
    let answers = futures::future::join_all(commits.iter().map(|commit| platform.checks(remote, commit))).await;
    let mut found = Vec::new();
    let mut error = None;
    for answer in answers {
        match answer {
            Ok(checks) => found.push(checks),
            Err(e) => error = error.or(Some(e)),
        }
    }
    (found, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{PlatformTraits, PullRequest, PullRequestDetail, PullRequestQuery, Viewer};

    /// Knows the checks of some commits and fails for the others.
    struct Known(Vec<CommitChecks>);

    impl Platform for Known {
        fn traits(&self) -> PlatformTraits {
            PlatformTraits::github()
        }

        async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
            unreachable!("only checks are asked for")
        }

        async fn list_pull_requests(
            &self,
            _: &RemoteKey,
            _: &PullRequestQuery,
        ) -> Result<Vec<PullRequest>, PlatformError> {
            unreachable!("only checks are asked for")
        }

        async fn pull_request(&self, _: &RemoteKey, _: u64) -> Result<PullRequestDetail, PlatformError> {
            unreachable!("only checks are asked for")
        }

        async fn checks(&self, _: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
            self.0
                .iter()
                .find(|c| c.commit == commit)
                .cloned()
                .ok_or_else(|| PlatformError::NotFound(format!("no commit {commit}")))
        }

        async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
            unreachable!("only checks are asked for")
        }
    }

    fn remote() -> RemoteKey {
        RemoteKey {
            host: "github.com".to_owned(),
            path: "acme/app".to_owned(),
        }
    }

    #[tokio::test]
    async fn a_commit_the_platform_does_not_know_does_not_hide_the_others() {
        let platform = Known(vec![
            commit_of("b", &[CiState::Success]),
            commit_of("d", &[CiState::Failure]),
        ]);
        let asked: Vec<String> = ["a", "b", "c", "d"].iter().map(|c| (*c).to_owned()).collect();
        let (found, error) = commits_checks(&platform, &remote(), &asked).await;
        assert_eq!(found.iter().map(|c| c.commit.as_str()).collect::<Vec<_>>(), ["b", "d"]);
        assert_eq!(error, Some(PlatformError::NotFound("no commit a".to_owned())));
    }

    fn commit_of(id: &str, states: &[CiState]) -> CommitChecks {
        CommitChecks {
            commit: id.to_owned(),
            ..commit(states)
        }
    }

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
        use CiState::*;
        assert_eq!(commit(&[]).state(), None);
        assert_eq!(commit(&[Success, Failure, Pending]).state(), Some(Pending));
        assert_eq!(commit(&[Success, Failure, Skipped]).state(), Some(Failure));
        assert_eq!(commit(&[Skipped, Success, Neutral]).state(), Some(Success));
        assert_eq!(commit(&[Skipped, Neutral]).state(), Some(Neutral));
        assert_eq!(commit(&[Skipped]).state(), Some(Skipped));
    }

    #[test]
    fn checks_are_final_once_there_are_some_and_none_runs() {
        use CiState::*;
        assert!(!commit(&[]).is_final());
        assert!(!commit(&[Success, Pending]).is_final());
        assert!(commit(&[Success, Failure]).is_final());
    }
}
