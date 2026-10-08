//! The checks of a commit on Azure DevOps: the statuses services post to a pull request's
//! iterations, and the evaluations of the pull request's policies, builds among them.
//!
//! A status belongs to an iteration, which has the commit the source branch was at when it was
//! pushed. The policies are evaluated for the pull request as it is now, so they count for its
//! head only. A build's log is found through the build the evaluation names: the log of the step
//! of its timeline that failed.

use azure_devops_rust_api::build::models::timeline_record::Result as StepResult;
use azure_devops_rust_api::git::models::{
    GitPullRequest, GitPullRequestIteration, GitPullRequestQuery, GitPullRequestQueryInput, GitPullRequestStatus,
    git_pull_request_query_input::Type as QueryType, git_status::State as StatusState,
};
use azure_devops_rust_api::policy::models::{PolicyEvaluationRecord, policy_evaluation_record::Status as PolicyStatus};
use vet_app::platform::{
    Check, CheckLog, CiState, CommitChecks, CredentialProvider, LogFailure, PlatformError, RemoteKey,
};

use super::wire::newest_per_context;
use super::{AzureDevOpsClient, Call, Repository, address, web_address};

/// What the log of a failed step marks its errors with.
const ERROR_MARK: &str = "##[error]";

fn status_check(status: &GitPullRequestStatus, url: &str) -> Option<Check> {
    let git = &status.git_status;
    let context = git.context.as_ref()?;
    let name = context.name.clone()?;
    let state = match git.state.as_ref()? {
        StatusState::Succeeded => CiState::Success,
        StatusState::Failed | StatusState::Error => CiState::Failure,
        StatusState::Pending | StatusState::NotSet => CiState::Pending,
        StatusState::NotApplicable => CiState::Skipped,
    };
    Some(Check {
        id: format!("status:{}", git.id?),
        name: match context.genre.as_deref().filter(|g| !g.is_empty()) {
            Some(genre) => format!("{genre}/{name}"),
            None => name,
        },
        state,
        url: git.target_url.clone().or_else(|| Some(url.to_owned())),
        started: git.creation_date.map(|d| d.unix_timestamp()),
        finished: (state != CiState::Pending)
            .then(|| git.updated_date.or(git.creation_date).map(|d| d.unix_timestamp()))
            .flatten(),
        log: false,
    })
}

/// The build a policy evaluation was made for, if it is a build policy.
fn build_of(evaluation: &PolicyEvaluationRecord) -> Option<i64> {
    evaluation
        .context
        .as_ref()?
        .get("buildId")?
        .as_i64()
        .filter(|id| *id > 0)
}

fn policy_check(evaluation: &PolicyEvaluationRecord, build_url: impl Fn(i64) -> String, url: &str) -> Option<Check> {
    let configuration = evaluation.configuration.as_ref()?;
    let blocking = configuration.is_blocking != Some(false);
    let state = match evaluation.status.as_ref()? {
        PolicyStatus::Approved => CiState::Success,
        // A policy that does not hold the merge back is advice.
        PolicyStatus::Rejected | PolicyStatus::Broken if !blocking => CiState::Neutral,
        PolicyStatus::Rejected | PolicyStatus::Broken => CiState::Failure,
        PolicyStatus::Queued | PolicyStatus::Running => CiState::Pending,
        PolicyStatus::NotApplicable => return None,
    };
    let name = configuration
        .settings
        .as_ref()
        .and_then(|s| s.get("displayName")?.as_str())
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            let reference = &configuration
                .versioned_policy_configuration_ref
                .policy_configuration_ref;
            Some(reference.type_.as_ref()?.display_name.clone())
        })
        .unwrap_or_else(|| "Policy".to_owned());
    let build = build_of(evaluation);
    Some(Check {
        id: match build {
            Some(build) => format!("build:{build}"),
            None => format!("policy:{}", evaluation.evaluation_id.as_deref()?),
        },
        name,
        state,
        url: Some(build.map_or_else(|| url.to_owned(), build_url)),
        started: evaluation.started_date.map(|d| d.unix_timestamp()),
        finished: evaluation.completed_date.map(|d| d.unix_timestamp()),
        log: build.is_some(),
    })
}

/// The id of the build a check names, if it is one.
fn build_number(check: &str) -> Option<i32> {
    check.strip_prefix("build:")?.parse().ok()
}

/// Where the log first shows an error: its line and the step it is of.
fn failure_in(text: &str, step: Option<String>) -> LogFailure {
    let line = text.lines().position(|l| l.contains(ERROR_MARK)).unwrap_or(0);
    LogFailure {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        step,
    }
}

/// A log as text: the API may send it as a JSON string or as plain text.
fn log_text(body: &[u8]) -> String {
    serde_json::from_slice::<String>(body).unwrap_or_else(|_| String::from_utf8_lossy(body).into_owned())
}

impl<C: CredentialProvider + Sync> AzureDevOpsClient<C> {
    /// The checks of `commit` in the pull request that has it as an iteration's source. A commit no
    /// pull request has has none.
    pub(super) async fn commit_checks(&self, remote: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
        let key = (remote.clone(), commit.to_owned());
        if let Some(known) = self.state().checks.get(&key) {
            return Ok(known.clone());
        }
        let call = self.call(remote).await?;
        let Some(number) = self.pull_request_with(&call, commit).await? else {
            return Ok(CommitChecks {
                commit: commit.to_owned(),
                checks: Vec::new(),
                url: None,
            });
        };
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let host = remote.host.as_str();
        let (pr, iterations) = futures::future::try_join(
            async {
                let found = call
                    .git
                    .pull_requests_client()
                    .get_pull_request(organization, name, number, project)
                    .await;
                self.settle(host, found)
            },
            async {
                let list = call
                    .git
                    .pull_request_iterations_client()
                    .list(organization, name, number, project)
                    .await;
                self.settle(host, list).map(|l| l.value)
            },
        )
        .await?;
        let checks = self.pull_request_checks(&call, &pr, &iterations, commit).await?;
        if checks.is_final() {
            self.state().checks.insert(key, checks.clone());
        }
        Ok(checks)
    }

    /// The number of a pull request that has `commit`, the one asked about before if there is one.
    async fn pull_request_with(&self, call: &Call<'_>, commit: &str) -> Result<Option<i32>, PlatformError> {
        let key = (call.remote.clone(), commit.to_owned());
        if let Some(number) = self.state().pull_requests_of_commits.get(&key) {
            return Ok(Some(*number));
        }
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let query = GitPullRequestQuery {
            queries: vec![GitPullRequestQueryInput {
                items: vec![commit.to_owned()],
                type_: Some(QueryType::Commit),
            }],
            results: Vec::new(),
        };
        let answer = call
            .git
            .pull_request_query_client()
            .get(organization, query, name, project)
            .await;
        let answer = Self::or_nothing(self.settle(&call.remote.host, answer))?;
        // The answer is a list of maps from a commit to the pull requests that have it.
        let found = answer
            .results
            .iter()
            .filter_map(|r| r.get(commit)?.as_array())
            .flatten()
            .filter_map(|pr| {
                let active = pr.get("status")?.as_str() == Some("active");
                Some((active, i32::try_from(pr.get("pullRequestId")?.as_i64()?).ok()?))
            })
            .max_by_key(|(active, _)| *active)
            .map(|(_, number)| number);
        if let Some(number) = found {
            self.state().pull_requests_of_commits.insert(key, number);
        }
        Ok(found)
    }

    /// The checks `commit` has in `pr`: the statuses of the iterations it is the source of, and for
    /// the head also the policies.
    pub(super) async fn pull_request_checks(
        &self,
        call: &Call<'_>,
        pr: &GitPullRequest,
        iterations: &[GitPullRequestIteration],
        commit: &str,
    ) -> Result<CommitChecks, PlatformError> {
        let Repository {
            organization,
            project,
            name,
        } = call.repository;
        let host = call.remote.host.as_str();
        let number = pr.pull_request_id;
        let source = |i: &GitPullRequestIteration| i.source_ref_commit.as_ref().and_then(|c| c.commit_id.clone());
        let of_commit: Vec<i32> = iterations
            .iter()
            .filter(|i| source(i).as_deref() == Some(commit))
            .filter_map(|i| i.id)
            .collect();
        let head = iterations
            .iter()
            .max_by_key(|i| i.id)
            .and_then(source)
            .or_else(|| pr.last_merge_source_commit.as_ref()?.commit_id.clone());
        let is_head = head.as_deref() == Some(commit);
        let url = format!("{}{number}", web_address(call.remote, &call.repository));
        let mut checks = CommitChecks {
            commit: commit.to_owned(),
            checks: Vec::new(),
            url: Some(url.clone()),
        };
        if of_commit.is_empty() && !is_head {
            return Ok(checks);
        }

        let statuses = async {
            let list = call
                .git
                .pull_request_statuses_client()
                .list(organization, name, number, project)
                .await;
            Self::or_nothing(self.settle(host, list).map(|l| l.value))
        };
        let evaluations = async {
            let Some(project_id) = pr.repository.project.id.as_deref().filter(|_| is_head) else {
                return Ok(Vec::new());
            };
            let artifact = format!("vstfs:///CodeReview/CodeReviewId/{project_id}/{number}");
            let list = call
                .access
                .policies(&self.endpoints().api)?
                .evaluations_client()
                .list(organization, project, artifact)
                .await;
            Self::or_nothing(self.settle(host, list).map(|l| l.value))
        };
        let (statuses, evaluations) = futures::future::try_join(statuses, evaluations).await?;

        let applicable = statuses
            .iter()
            .filter(|s| s.iteration_id.map_or(is_head, |i| of_commit.contains(&i)));
        checks.checks.extend(
            newest_per_context(applicable)
                .into_iter()
                .filter_map(|s| status_check(s, &url)),
        );
        let build_url = |build: i64| {
            format!(
                "{}?buildId={build}",
                address(call.remote, &call.repository, &["_build", "results"])
            )
        };
        checks
            .checks
            .extend(evaluations.iter().filter_map(|e| policy_check(e, build_url, &url)));
        Ok(checks)
    }

    /// The log of the step of build `check` that failed, with the line of its first error.
    pub(super) async fn build_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        let build = build_number(check)
            .ok_or_else(|| PlatformError::NotFound(format!("{} has no log for {check}", remote.host)))?;
        let call = self.call(remote).await?;
        let Repository {
            organization, project, ..
        } = call.repository;
        let host = remote.host.as_str();
        let builds = call.access.builds(&self.endpoints().api)?;
        let timeline = builds.timeline_client().get(organization, project, build, "").await;
        let timeline = self.settle(host, timeline)?;
        let step = timeline
            .records
            .iter()
            .filter(|r| r.result == Some(StepResult::Failed) && r.log.is_some())
            // A task is the step that ran; the stage and the job above it only hold it.
            .min_by_key(|r| (r.type_.as_deref() != Some("Task"), r.order))
            .ok_or_else(|| {
                PlatformError::NotFound(format!("{host} has no log of a step that failed in build {build}"))
            })?;
        let log = step.log.as_ref().map(|l| l.id).expect("filtered");
        let reply = builds
            .builds_client()
            .get_build_log(organization, project, build, log)
            .send()
            .await;
        let reply = self.settle(host, reply)?;
        let text = log_text(reply.into_raw_response().body());
        let failure = failure_in(&text, step.name.clone());
        Ok(CheckLog {
            text,
            failure: Some(failure),
        })
    }
}
