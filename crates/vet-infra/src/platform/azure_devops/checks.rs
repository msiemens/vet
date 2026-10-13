//! The checks of a commit on Azure DevOps: the statuses services post to a pull request's
//! iterations, and the evaluations of the pull request's policies, builds among them.
//!
//! A status belongs to an iteration, which has the commit the source branch was at when it was
//! pushed. The policies are evaluated for the pull request as it is now, so they count for its
//! head only. A build's log is found through the build the evaluation names: the log of the step
//! of its timeline that failed.

use azure_devops_rust_api::build::models::build::{Reason as BuildReason, Status as BuildStatus};
use azure_devops_rust_api::build::models::timeline_record::{Result as StepResult, State as StepState};
use azure_devops_rust_api::build::models::update_stage_parameters::State as StageState;
use azure_devops_rust_api::build::models::{TimelineRecord, UpdateStageParameters};
use azure_devops_rust_api::git::models::{
    GitPullRequest, GitPullRequestIteration, GitPullRequestQuery, GitPullRequestQueryInput, GitPullRequestStatus,
    git_pull_request_query_input::Type as QueryType, git_status::State as StatusState,
};
use azure_devops_rust_api::policy::models::{PolicyEvaluationRecord, policy_evaluation_record::Status as PolicyStatus};
use futures::future::{join_all, try_join_all};
use vet_app::platform::{
    Check, CheckLog, CiState, CommitChecks, CredentialProvider, LogFailure, Phase, PipelineAction, PlatformError,
    RemoteKey, Run, Step,
};
use vet_core::num::sat_u32;

use super::wire::newest_per_context;
use super::{AzureDevOpsClient, Call, Repository, address, web_address};

/// What the log of a failed step marks its errors with.
const ERROR_MARK: &str = "##[error]";

/// The kinds of record of a build's timeline that matter: a stage of a pipeline and a task, the
/// step that ran.
const STAGE: &str = "Stage";
const TASK: &str = "Task";

/// How many logs of a stage are asked for at once.
const LOGS_AT_ONCE: usize = 8;

/// A build as the tool lists it: the run, and its timeline.
struct BuildRun {
    run: Run,
    timeline: Timeline,
}

/// The records of a build's timeline: its stages, and under them phases, jobs and tasks.
struct Timeline(Vec<TimelineRecord>);

impl Timeline {
    fn stages(&self) -> Vec<&TimelineRecord> {
        let mut stages: Vec<_> = self.0.iter().filter(|r| r.type_.as_deref() == Some(STAGE)).collect();
        stages.sort_by_key(|r| r.order);
        stages
    }

    /// The tasks inside `stage`, in the order they run.
    fn tasks_of(&self, stage: &TimelineRecord) -> Vec<&TimelineRecord> {
        let Some(stage_id) = stage.id.as_deref() else {
            return Vec::new();
        };
        // The orders on the way down from the stage: its phase, job, then the task itself.
        let path = |record: &TimelineRecord| -> Option<Vec<Option<i32>>> {
            let mut orders = vec![record.order];
            let mut parent = record.parent_id.as_deref();
            while let Some(id) = parent {
                if id == stage_id {
                    orders.reverse();
                    return Some(orders);
                }
                let up = self.0.iter().find(|r| r.id.as_deref() == Some(id))?;
                orders.push(up.order);
                parent = up.parent_id.as_deref();
            }
            None
        };
        let mut tasks: Vec<_> = self
            .0
            .iter()
            .filter(|r| r.type_.as_deref() == Some(TASK))
            .filter_map(|r| Some((path(r)?, r)))
            .collect();
        tasks.sort_by(|a, b| a.0.cmp(&b.0));
        tasks.into_iter().map(|(_, r)| r).collect()
    }

    fn stage(&self, identifier: &str) -> Option<&TimelineRecord> {
        self.stages()
            .into_iter()
            .find(|r| r.identifier.as_deref() == Some(identifier))
    }
}

/// How a stage or task stands: its state for the merge, and how far it has come.
fn outcome(record: &TimelineRecord) -> (CiState, Phase) {
    match (&record.state, &record.result) {
        (Some(StepState::Completed), Some(StepResult::Succeeded | StepResult::SucceededWithIssues)) => {
            (CiState::Success, Phase::Finished)
        }
        (Some(StepState::Completed), Some(StepResult::Failed)) => (CiState::Failure, Phase::Finished),
        (Some(StepState::Completed), Some(StepResult::Canceled | StepResult::Abandoned)) => {
            (CiState::Neutral, Phase::Cancelled)
        }
        (Some(StepState::Completed), Some(StepResult::Skipped)) => (CiState::Skipped, Phase::Finished),
        (Some(StepState::Completed), None) => (CiState::Neutral, Phase::Finished),
        (Some(StepState::InProgress), _) => (CiState::Pending, Phase::Running),
        _ => (CiState::Pending, Phase::Queued),
    }
}

/// Why a build ran, in words.
fn reason_words(reason: &BuildReason) -> String {
    match reason {
        BuildReason::PullRequest => "pull request".to_owned(),
        BuildReason::IndividualCi | BuildReason::BatchedCi => "push".to_owned(),
        BuildReason::Schedule | BuildReason::ScheduleForced => "schedule".to_owned(),
        BuildReason::Manual | BuildReason::UserCreated => "manual".to_owned(),
        other => other.to_string(),
    }
}

/// The check that stands for a stage of the run: its job.
fn stage_check(build: i32, run: &Run, stage: &TimelineRecord, tasks: &[&TimelineRecord], url: &str) -> Option<Check> {
    let (state, phase) = outcome(stage);
    Some(Check {
        id: format!("stage:{build}:{}", stage.identifier.as_deref()?),
        name: stage.name.clone()?,
        state,
        phase,
        run: Some(run.clone()),
        url: Some(url.to_owned()),
        started: stage.start_time.map(|t| t.unix_timestamp()),
        finished: stage.finish_time.map(|t| t.unix_timestamp()),
        log: tasks.iter().any(|t| t.log.is_some()),
    })
}

fn stage_id(check: &str) -> Option<(i32, &str)> {
    let (build, identifier) = check.strip_prefix("stage:")?.split_once(':')?;
    Some((build.parse().ok()?, identifier))
}

/// The build and policy evaluation a run names.
fn run_ids(run: &str) -> Option<(i32, &str)> {
    let (build, evaluation) = run.split_once(':')?;
    Some((build.parse().ok()?, evaluation))
}

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
        phase: if state == CiState::Pending {
            Phase::Running
        } else {
            Phase::Finished
        },
        run: None,
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
        phase: match evaluation.status.as_ref()? {
            PolicyStatus::Queued => Phase::Queued,
            PolicyStatus::Running => Phase::Running,
            _ => Phase::Finished,
        },
        run: None,
        url: Some(build.map_or_else(|| url.to_owned(), build_url)),
        started: evaluation.started_date.map(|d| d.unix_timestamp()),
        finished: evaluation.completed_date.map(|d| d.unix_timestamp()),
        log: build.is_some(),
    })
}

/// Whether a policy that holds the merge back has failed. One still queued or running has not
/// failed yet, and a policy that is not blocking only advises.
pub(super) fn blocked_by(evaluations: &[PolicyEvaluationRecord]) -> bool {
    evaluations.iter().any(|evaluation| {
        let blocking = evaluation
            .configuration
            .as_ref()
            .is_some_and(|c| c.is_blocking != Some(false));
        blocking && matches!(evaluation.status, Some(PolicyStatus::Rejected | PolicyStatus::Broken))
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
        line: sat_u32(line),
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
        let host = &remote.account();
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
            self.state().checks.insert(key, checks.clone(), (self.now)());
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
        let answer = Self::or_nothing(self.settle(&call.remote.account(), answer))?;
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
            self.state().pull_requests_of_commits.insert(key, number, (self.now)());
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
        let host = &call.remote.account();
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
        let url = format!("{}{number}", web_address(call.remote));
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
        let build_url = |build: i64| format!("{}?buildId={build}", address(call.remote, &["_build", "results"]));
        let policies: Vec<(&PolicyEvaluationRecord, Check)> = evaluations
            .iter()
            .filter_map(|e| Some((e, policy_check(e, build_url, &url)?)))
            .collect();
        // A build whose timeline is not readable stands for itself, as one check.
        let timelines = try_join_all(policies.iter().map(|(evaluation, policy)| async move {
            let Some(build) = build_of(evaluation).and_then(|b| i32::try_from(b).ok()) else {
                return Ok(None);
            };
            let id = evaluation.evaluation_id.as_deref().unwrap_or_default();
            match self.build_run(call, build, id, &policy.name).await {
                Ok(built) => Ok(Some(built)),
                // The build stands for itself when its timeline cannot be had or understood.
                Err(PlatformError::Forbidden(_) | PlatformError::NotFound(_) | PlatformError::Unexpected(_)) => {
                    Ok(None)
                }
                Err(other) => Err(other),
            }
        }))
        .await?;
        for ((evaluation, policy), built) in policies.into_iter().zip(timelines) {
            let Some(built) = built else {
                checks.checks.push(policy);
                continue;
            };
            let advisory = evaluation
                .configuration
                .as_ref()
                .is_some_and(|c| c.is_blocking == Some(false));
            let build = build_of(evaluation)
                .and_then(|b| i32::try_from(b).ok())
                .unwrap_or_default();
            let stages = built.timeline.stages();
            if stages.is_empty() {
                checks.checks.push(Check {
                    run: Some(built.run.clone()),
                    ..policy
                });
                continue;
            }
            for stage in stages {
                let tasks = built.timeline.tasks_of(stage);
                let Some(mut check) =
                    stage_check(build, &built.run, stage, &tasks, policy.url.as_deref().unwrap_or(&url))
                else {
                    continue;
                };
                // What a policy that does not hold the merge back reports does not fail the commit.
                if advisory && check.state == CiState::Failure {
                    check.state = CiState::Neutral;
                }
                checks.checks.push(check);
            }
        }
        Ok(checks)
    }

    /// The build `build`, which the policy evaluation `evaluation` queued, with its timeline.
    /// `fallback` names the run when its pipeline has no name.
    async fn build_run(
        &self,
        call: &Call<'_>,
        build: i32,
        evaluation: &str,
        fallback: &str,
    ) -> Result<BuildRun, PlatformError> {
        let Repository {
            organization, project, ..
        } = call.repository;
        let host = &call.remote.account();
        let builds = call.access.builds(&self.endpoints().api)?;
        let (info, timeline) = futures::future::try_join(
            async {
                let found = builds.builds_client().get(organization, project, build).await;
                self.settle(host, found)
            },
            async {
                let found = builds.timeline_client().get(organization, project, build, "").await;
                self.settle(host, found)
            },
        )
        .await?;
        let name = info
            .definition
            .name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| fallback.to_owned());
        let reason = reason_words(&info.reason);
        let attempt = timeline
            .records
            .iter()
            .filter(|r| r.type_.as_deref() == Some(STAGE))
            .filter_map(|r| r.attempt)
            .max()
            .and_then(|a| u32::try_from(a).ok())
            .unwrap_or(1);
        Ok(BuildRun {
            run: Run {
                id: format!("{build}:{evaluation}"),
                name,
                detail: match info.build_number.filter(|n| !n.is_empty()) {
                    Some(number) => format!("#{number} · {reason}"),
                    None => reason,
                },
                attempt,
            },
            timeline: Timeline(timeline.records),
        })
    }

    /// The timeline of build `build`.
    async fn timeline(&self, call: &Call<'_>, build: i32) -> Result<Timeline, PlatformError> {
        let Repository {
            organization, project, ..
        } = call.repository;
        let host = &call.remote.account();
        let found = call
            .access
            .builds(&self.endpoints().api)?
            .timeline_client()
            .get(organization, project, build, "")
            .await;
        Ok(Timeline(self.settle(host, found)?.records))
    }

    /// The tasks of the stage that `check` names.
    pub(super) async fn stage_tasks(&self, remote: &RemoteKey, check: &str) -> Result<Vec<Step>, PlatformError> {
        let (build, identifier) = stage_id(check)
            .ok_or_else(|| PlatformError::NotFound(format!("{} has no stage {check}", remote.account())))?;
        let call = self.call(remote).await?;
        let timeline = self.timeline(&call, build).await?;
        let stage = timeline.stage(identifier).ok_or_else(|| {
            PlatformError::NotFound(format!(
                "{} has no stage {identifier} in build {build}",
                remote.account()
            ))
        })?;
        Ok(timeline
            .tasks_of(stage)
            .into_iter()
            .filter_map(|task| {
                let (state, phase) = outcome(task);
                Some(Step {
                    name: task.name.clone()?,
                    state,
                    phase,
                    started: task.start_time.map(|t| t.unix_timestamp()),
                    finished: task.finish_time.map(|t| t.unix_timestamp()),
                })
            })
            .collect())
    }

    /// Asks Azure Pipelines to retry or stop a run. What is kept of the commit's checks is dropped,
    /// since they change.
    pub(super) async fn change_run(&self, remote: &RemoteKey, action: &PipelineAction) -> Result<(), PlatformError> {
        let call = self.call(remote).await?;
        let Repository {
            organization, project, ..
        } = call.repository;
        let host = &remote.account();
        let not_a_run = |run: &str| PlatformError::NotFound(format!("{host} has no run {run}"));
        let builds = call.access.builds(&self.endpoints().api)?;
        let retry = |build: i32, identifier: String| {
            let stages = builds.stages_client();
            async move {
                let body = UpdateStageParameters {
                    state: Some(StageState::Retry),
                    force_retry_all_jobs: None,
                };
                let sent = stages.update(organization, body, build, identifier, project).await;
                self.settle(host, sent)
            }
        };
        match action {
            PipelineAction::RerunJob { check } => {
                let (build, identifier) = stage_id(check).ok_or_else(|| not_a_run(check))?;
                retry(build, identifier.to_owned()).await?;
            }
            PipelineAction::RerunFailed { run } => {
                let (build, _) = run_ids(run).ok_or_else(|| not_a_run(run))?;
                let timeline = self.timeline(&call, build).await?;
                let failed: Vec<String> = timeline
                    .stages()
                    .into_iter()
                    .filter(|s| outcome(s).0 == CiState::Failure)
                    .filter_map(|s| s.identifier.clone())
                    .collect();
                for identifier in failed {
                    retry(build, identifier).await?;
                }
            }
            PipelineAction::RerunAll { run } => {
                let (_, evaluation) = run_ids(run).ok_or_else(|| not_a_run(run))?;
                let sent = call
                    .access
                    .policies(&self.endpoints().api)?
                    .evaluations_client()
                    .requeue_policy_evaluation(organization, project, evaluation)
                    .await;
                self.settle(host, sent)?;
            }
            PipelineAction::Cancel { run } => {
                let (build, _) = run_ids(run).ok_or_else(|| not_a_run(run))?;
                let client = builds.builds_client();
                let found = client.get(organization, project, build).await;
                let mut found = self.settle(host, found)?;
                found.status = Some(BuildStatus::Cancelling);
                let sent = client.update_build(organization, found, project, build).await;
                self.settle(host, sent)?;
            }
        }
        self.state().checks.retain(|(kept, _), _| kept != remote);
        Ok(())
    }

    /// The log of the stage that `check` names: what its tasks printed, one after the other. Where
    /// the stage failed, it shows the first error line and the task it is of.
    async fn stage_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        let (build, identifier) = stage_id(check)
            .ok_or_else(|| PlatformError::NotFound(format!("{} has no log for {check}", remote.account())))?;
        let call = self.call(remote).await?;
        let Repository {
            organization, project, ..
        } = call.repository;
        let host = &remote.account();
        let timeline = self.timeline(&call, build).await?;
        let stage = timeline
            .stage(identifier)
            .ok_or_else(|| PlatformError::NotFound(format!("{host} has no stage {identifier} in build {build}")))?;
        let failed = outcome(stage).0 == CiState::Failure;
        let tasks: Vec<(String, i32)> = timeline
            .tasks_of(stage)
            .into_iter()
            .filter_map(|t| Some((t.name.clone()?, t.log.as_ref()?.id)))
            .collect();
        if tasks.is_empty() {
            return Err(PlatformError::NotFound(format!(
                "{host} has no log of stage {identifier}"
            )));
        }
        let builds = call.access.builds(&self.endpoints().api)?;
        let mut texts = Vec::new();
        for chunk in tasks.chunks(LOGS_AT_ONCE) {
            let replies = join_all(chunk.iter().map(|(_, log)| async {
                let reply = builds
                    .builds_client()
                    .get_build_log(organization, project, build, *log)
                    .send()
                    .await;
                self.settle(host, reply).map(|r| log_text(r.into_raw_response().body()))
            }))
            .await;
            for reply in replies {
                texts.push(reply?);
            }
        }
        let mut text = String::new();
        let mut lines = 0;
        let mut failure = None;
        for ((name, _), part) in tasks.iter().zip(&texts) {
            let part = part.trim_end_matches('\n');
            if part.is_empty() {
                continue;
            }
            if failed
                && failure.is_none()
                && let Some(at) = part.lines().position(|l| l.contains(ERROR_MARK))
            {
                failure = Some(LogFailure {
                    line: sat_u32(lines + at),
                    step: Some(name.clone()),
                });
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(part);
            lines += part.lines().count();
        }
        Ok(CheckLog { text, failure })
    }

    /// The log of the step of build `check` that failed, with the line of its first error.
    pub(super) async fn build_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        if check.starts_with("stage:") {
            return self.stage_log(remote, check).await;
        }
        let build = build_number(check)
            .ok_or_else(|| PlatformError::NotFound(format!("{} has no log for {check}", remote.account())))?;
        let call = self.call(remote).await?;
        let Repository {
            organization, project, ..
        } = call.repository;
        let host = &remote.account();
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
