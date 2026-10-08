//! The checks of a commit and the logs of its Actions jobs, over the REST API.
//!
//! A commit's checks are its check runs and the commit statuses services set (the legacy API many
//! tools still use). What is learnt of a commit whose checks are all done is kept for good.
//! A job's log is where Actions writes what each step printed; where the job failed is found
//! from the job's steps and the log's own `##[error]` lines (see [`process_log`]).

use futures::future::try_join;
use octocrab::map_github_error;
use serde::Deserialize;
use vet_app::platform::{
    Check, CheckLog, CiState, CommitChecks, CredentialProvider, LogFailure, PlatformError, RemoteKey,
};

use super::wire::parse_timestamp;
use super::{Bucket, GitHubClient, repository};

/// The app whose check runs have a log to fetch.
const ACTIONS_APP: &str = "github-actions";

/// How many commits' complete checks are kept.
const KEPT_COMMITS: usize = 500;

#[derive(Deserialize)]
struct CheckRuns {
    check_runs: Vec<Run>,
}

#[derive(Deserialize)]
struct Run {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    html_url: Option<String>,
    app: Option<App>,
}

#[derive(Deserialize)]
struct App {
    slug: String,
}

#[derive(Deserialize)]
struct CombinedStatus {
    statuses: Vec<Status>,
}

#[derive(Deserialize)]
struct Status {
    id: u64,
    context: String,
    state: String,
    target_url: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct Job {
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    name: String,
    conclusion: Option<String>,
    started_at: Option<String>,
}

/// The state of a check run: running until it completed, then by its conclusion. A cancelled run
/// decided nothing about the commit.
fn run_state(status: &str, conclusion: Option<&str>) -> CiState {
    if status != "completed" {
        return CiState::Pending;
    }
    match conclusion {
        Some("success") => CiState::Success,
        Some("failure" | "timed_out" | "startup_failure" | "action_required") => CiState::Failure,
        Some("skipped") => CiState::Skipped,
        _ => CiState::Neutral,
    }
}

fn status_state(state: &str) -> CiState {
    match state {
        "success" => CiState::Success,
        "failure" | "error" => CiState::Failure,
        _ => CiState::Pending,
    }
}

fn run_check(run: Run) -> Check {
    Check {
        id: run.id.to_string(),
        state: run_state(&run.status, run.conclusion.as_deref()),
        started: run.started_at.as_deref().and_then(parse_instant),
        finished: run.completed_at.as_deref().and_then(parse_instant),
        url: run.html_url,
        log: run.app.is_some_and(|a| a.slug == ACTIONS_APP),
        name: run.name,
    }
}

fn status_check(status: Status) -> Check {
    let state = status_state(&status.state);
    let finished = (state != CiState::Pending)
        .then(|| status.updated_at.as_deref().and_then(parse_instant))
        .flatten();
    Check {
        id: format!("status-{}", status.id),
        started: status.created_at.as_deref().and_then(parse_instant),
        finished,
        url: status.target_url.filter(|u| !u.is_empty()),
        log: false,
        name: status.context,
        state,
    }
}

/// The checks of a commit from the answers of the two APIs: runs first, then statuses.
fn commit_checks(remote: &RemoteKey, commit: &str, runs: CheckRuns, statuses: CombinedStatus) -> CommitChecks {
    let mut checks: Vec<Check> = runs.check_runs.into_iter().map(run_check).collect();
    checks.extend(statuses.statuses.into_iter().map(status_check));
    CommitChecks {
        commit: commit.to_owned(),
        checks,
        url: Some(format!(
            "https://{}/{}/commit/{commit}/checks",
            remote.host, remote.path
        )),
    }
}

impl<C: CredentialProvider + Sync> GitHubClient<C> {
    pub(super) async fn commit_checks(&self, remote: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
        let (owner, name) = repository(remote)?;
        if commit.is_empty() || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(PlatformError::Unexpected(format!("{commit:?} is not a commit id")));
        }
        let key = (remote.clone(), commit.to_owned());
        if let Some(kept) = self.state().checks.get(&key) {
            return Ok(kept.clone());
        }
        let host = remote.host.as_str();
        let clients = self.clients(host).await?;
        let page = [("per_page", "100")];
        let runs = async {
            let found: octocrab::Result<CheckRuns> = clients
                .rest
                .get(
                    format!("/repos/{owner}/{name}/commits/{commit}/check-runs"),
                    Some(&page),
                )
                .await;
            self.settle(host, &clients.rest, Bucket::Core, found).await
        };
        let statuses = async {
            let found: octocrab::Result<CombinedStatus> = clients
                .rest
                .get(format!("/repos/{owner}/{name}/commits/{commit}/status"), Some(&page))
                .await;
            self.settle(host, &clients.rest, Bucket::Core, found).await
        };
        let (runs, statuses) = try_join(runs, statuses).await?;
        let checks = commit_checks(remote, commit, runs, statuses);
        if checks.is_final() {
            let mut state = self.state();
            if state.checks.len() >= KEPT_COMMITS {
                state.checks.clear();
            }
            state.checks.insert(key, checks.clone());
        }
        Ok(checks)
    }

    pub(super) async fn job_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        let (owner, name) = repository(remote)?;
        let host = remote.host.as_str();
        let job_id: u64 = check
            .parse()
            .map_err(|_| PlatformError::Unexpected(format!("{check:?} is not a job of {host}")))?;
        let clients = self.clients(host).await?;
        let rest = &clients.rest;

        let job: octocrab::Result<Job> = rest
            .get(format!("/repos/{owner}/{name}/actions/jobs/{job_id}"), None::<&()>)
            .await;
        let job = self.settle(host, rest, Bucket::Core, job).await?;

        // The log is behind a redirect to a signed address on another host, which the token must
        // not be sent to: the client follows none, and the address is fetched without it.
        let answer = rest
            ._get(format!("/repos/{owner}/{name}/actions/jobs/{job_id}/logs"))
            .await;
        let answer = self.settle(host, rest, Bucket::Core, answer).await?;
        let body = if answer.status().is_redirection() {
            let location = answer
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .ok_or_else(|| PlatformError::Unexpected(format!("{host} did not say where the log is")))?
                .to_owned();
            self.plain_get(host, &location).await?
        } else {
            let answered = self
                .settle(host, rest, Bucket::Core, map_github_error(answer).await)
                .await?;
            self.settle(host, rest, Bucket::Core, rest.body_to_string(answered).await)
                .await?
        };
        let failed = job.steps.iter().find(|s| s.conclusion.as_deref() == Some("failure"));
        Ok(process_log(
            &body,
            failed.map(|s| (s.name.as_str(), s.started_at.as_deref().and_then(parse_instant))),
        ))
    }

    /// The text at `address`, fetched with no credential.
    async fn plain_get(&self, host: &str, address: &str) -> Result<String, PlatformError> {
        let http = self
            .http
            .get_or_init(|| crate::http::http_client(super::TIMEOUT))
            .as_ref()
            .map_err(|why| PlatformError::Unreachable(why.clone()))?;
        let gone = |e: reqwest::Error| PlatformError::Unreachable(format!("cannot reach the log of {host}: {e}"));
        let response = http.get(address).send().await.map_err(gone)?;
        if !response.status().is_success() {
            return Err(PlatformError::Unexpected(format!(
                "the log of {host} is not available ({})",
                response.status()
            )));
        }
        response.text().await.map_err(gone)
    }
}

/// Unix seconds of a time as GitHub writes it: UTC or with an offset, with or without a fraction
/// of a second (`2024-05-01T12:00:00Z`, `2024-05-01T12:00:00.1234567Z`, `2024-05-01T05:00:00-07:00`).
pub(super) fn parse_instant(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let end = time.find(['Z', '+', '-'])?;
    let (clock, zone) = time.split_at(end);
    let clock = clock.split_once('.').map_or(clock, |(whole, _)| whole);
    let offset = match zone {
        "Z" => 0,
        _ => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (hours, minutes) = zone[1..].split_once(':')?;
            sign * (hours.parse::<i64>().ok()? * 3600 + minutes.parse::<i64>().ok()? * 60)
        }
    };
    Some(parse_timestamp(&format!("{date}T{clock}Z"))? - offset)
}

/// `line` without the colour codes tools print (`ESC [ … m`), which an editor shows as garbage.
fn without_colours(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for skipped in chars.by_ref() {
                if skipped.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    plain
}

/// The log's lines without the time Actions puts before each and the colour codes in them, and
/// where the job failed.
///
/// Actions marks a failure with a `##[error]` line, which ends the step that failed. With the
/// failed step from the job's API (its name and when it started) the failure is the first error
/// line from that time on, and the step is named. Without a step, or when the log has no error
/// line after it started, the first error line of the log is taken, and the step is named only if
/// it is known to contain the line. A step that failed without an error line (it was cancelled) is
/// found by its start time.
pub(super) fn process_log(raw: &str, failed_step: Option<(&str, Option<i64>)>) -> CheckLog {
    let mut texts = Vec::new();
    let mut times = Vec::new();
    for line in raw.trim_start_matches('\u{feff}').lines() {
        let (stamp, text) = line.split_once(' ').unwrap_or((line, ""));
        match parse_instant(stamp) {
            Some(at) => {
                times.push(Some(at));
                texts.push(without_colours(text));
            }
            None => {
                times.push(None);
                texts.push(without_colours(line));
            }
        }
    }
    let is_error = |i: usize| texts[i].starts_with("##[error]");
    let started = failed_step.and_then(|(_, at)| at);
    let in_step = (0..texts.len()).find(|&i| is_error(i) && started.is_some_and(|s| times[i].is_some_and(|t| t >= s)));
    let failure = if let Some(line) = in_step {
        Some((line, failed_step.map(|(name, _)| name)))
    } else if let Some(line) = (0..texts.len()).find(|&i| is_error(i)) {
        Some((line, None))
    } else {
        // Times in the job's steps are whole seconds, so the step before it may have started in
        // the same second: its header is the last one in that second.
        let header = |s: i64| {
            (0..texts.len())
                .rev()
                .find(|&i| texts[i].starts_with("##[group]") && times[i] == Some(s))
        };
        started
            .and_then(|s| header(s).or_else(|| times.iter().position(|t| t.is_some_and(|t| t >= s))))
            .zip(failed_step.map(|(name, _)| name))
            .map(|(line, name)| (line, Some(name)))
    };
    CheckLog {
        text: texts.join("\n"),
        failure: failure.map(|(line, step)| LogFailure {
            line: u32::try_from(line).unwrap_or(u32::MAX),
            step: step.map(str::to_owned),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_read_with_fractions_and_offsets() {
        assert_eq!(parse_instant("2024-05-01T12:00:00Z"), Some(1_714_564_800));
        assert_eq!(parse_instant("2024-05-01T12:00:00.1234567Z"), Some(1_714_564_800));
        assert_eq!(parse_instant("2024-05-01T05:00:00.000-07:00"), Some(1_714_564_800));
        assert_eq!(parse_instant("2024-05-01T14:00:00+02:00"), Some(1_714_564_800));
        assert_eq!(parse_instant("not a time"), None);
    }

    const LOG: &str = "\
2024-05-01T12:00:00.0000000Z ##[group]Run cargo build
2024-05-01T12:00:01.0000000Z Compiling app
2024-05-01T12:00:09.0000000Z ##[error]Process completed with exit code 1.
2024-05-01T12:00:10.0000000Z ##[group]Run cargo test
2024-05-01T12:00:12.0000000Z test a ... FAILED
2024-05-01T12:00:13.0000000Z ##[error]Process completed with exit code 101.
2024-05-01T12:00:14.0000000Z Post job cleanup.
";

    fn at(clock: &str) -> Option<i64> {
        parse_instant(&format!("2024-05-01T{clock}Z"))
    }

    #[test]
    fn the_failure_is_the_first_error_line_from_the_time_the_failed_step_started() {
        let log = process_log(LOG, Some(("Test", at("12:00:10"))));
        assert_eq!(
            log.failure,
            Some(LogFailure {
                line: 5,
                step: Some("Test".to_owned())
            })
        );
    }

    #[test]
    fn the_timestamps_are_cut_from_the_text_and_no_line_is_lost() {
        let log = process_log(LOG, None);
        let lines: Vec<&str> = log.text.lines().collect();
        assert_eq!(lines.len(), 7);
        assert_eq!(lines[1], "Compiling app");
    }

    #[test]
    fn without_a_failed_step_the_first_error_line_of_the_log_is_the_failure() {
        let log = process_log(LOG, None);
        assert_eq!(log.failure, Some(LogFailure { line: 2, step: None }));
    }

    #[test]
    fn a_step_that_was_cut_short_without_an_error_line_is_found_by_when_it_started() {
        let log = process_log(
            "2024-05-01T12:00:00Z start\n2024-05-01T12:00:05Z still going\n",
            Some(("Build", at("12:00:05"))),
        );
        assert_eq!(
            log.failure,
            Some(LogFailure {
                line: 1,
                step: Some("Build".to_owned())
            })
        );
    }

    #[test]
    fn a_step_that_failed_without_an_error_line_starts_at_the_last_header_of_the_second_it_began() {
        let log = "\
2024-05-01T12:00:05.1000000Z ##[group]Run first
2024-05-01T12:00:05.2000000Z fine
2024-05-01T12:00:05.9000000Z ##[group]Run second
2024-05-01T12:00:06.5000000Z boom
2024-05-01T12:00:09.0000000Z ##[group]Run third
";
        let found = process_log(log, Some(("second", at("12:00:05"))));
        assert_eq!(
            found.failure,
            Some(LogFailure {
                line: 2,
                step: Some("second".to_owned())
            })
        );
    }

    #[test]
    fn colour_codes_are_cut_from_the_text() {
        let log = process_log("2024-05-01T12:00:00Z \u{1b}[31;1mFAILED\u{1b}[0m test\n", None);
        assert_eq!(log.text, "FAILED test");
    }

    #[test]
    fn a_log_with_nothing_wrong_in_it_points_nowhere() {
        assert_eq!(process_log("2024-05-01T12:00:00Z all fine\n", None).failure, None);
    }

    #[test]
    fn a_line_without_a_time_stays_as_it_is() {
        let log = process_log("plain\n2024-05-01T12:00:00Z ##[error]boom\n", None);
        assert_eq!(log.text, "plain\n##[error]boom");
        assert_eq!(log.failure.map(|f| f.line), Some(1));
    }
}
