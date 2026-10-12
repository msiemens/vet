//! What the accounts say about `gh`, with a fake `gh` script run by the real process runner.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use vet_app::platform::{
    Account, AccountState, Accounts, Credential, CredentialProvider, CredentialSource, CredentialValidator, Owner,
    PlatformKind, ValidationError,
};
use vet_app::ports::{Command, ProcessRunner};
use vet_infra::exec::{Process, TokioRunner};

/// Runs the script at `gh` wherever the code asks for `gh`.
struct FakeGh {
    script: PathBuf,
}

impl ProcessRunner for FakeGh {
    type Process = Process;

    fn spawn(&self, command: &Command) -> Result<Process, String> {
        let program = if command.program == "gh" {
            self.script.to_string_lossy().into_owned()
        } else {
            command.program.clone()
        };
        TokioRunner.spawn(&Command {
            program,
            args: command.args.clone(),
            cwd: command.cwd.clone(),
            env: command.env.clone(),
        })
    }
}

struct Everyone;

impl CredentialValidator for Everyone {
    async fn user(&self, _: &str, _: PlatformKind, _: &Credential) -> Result<Owner, ValidationError> {
        Ok(Owner::named("octocat"))
    }
}

const ERROR_STATUS: &str = r#"{"hosts":{"github.com":[{"active":true,"host":"github.com","login":"octocat","state":"error","tokenSource":"default","error":"HTTP 401: Bad credentials (https://api.github.com/)"}]}}"#;
const GOOD_STATUS: &str =
    r#"{"hosts":{"github.com":[{"active":true,"host":"github.com","login":"octocat","state":"success"}]}}"#;

/// A `gh` that answers `auth status` and `auth token` with the given shell snippets.
fn gh(dir: &Path, status: &str, token: &str) -> FakeGh {
    let script = dir.join("gh");
    let body = format!("#!/bin/sh\ncase \"$2\" in\nstatus) {status};;\ntoken) {token};;\nesac\n");
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    FakeGh { script }
}

fn accounts(runner: FakeGh) -> Accounts<FakeGh, Everyone> {
    Accounts::new(runner, Everyone, std::env::temp_dir(), |_| {})
}

fn states(rows: &[Account]) -> Vec<(&str, CredentialSource, &AccountState)> {
    rows.iter().map(|a| (a.host.as_str(), a.source, &a.state)).collect()
}

#[tokio::test]
async fn a_host_gh_reports_in_error_shows_gh_s_own_error() {
    let dir = tempfile::tempdir().unwrap();
    let runner = gh(
        dir.path(),
        &format!("echo '{ERROR_STATUS}'"),
        "echo 'no oauth token found for github.com' >&2; exit 1",
    );
    let accounts = accounts(runner);
    let rows = accounts.accounts(0).await;
    assert_eq!(
        states(&rows),
        [(
            "github.com",
            CredentialSource::Cli,
            &AccountState::Unusable("HTTP 401: Bad credentials (https://api.github.com/)".into())
        )]
    );
    assert!(!rows[0].in_use);
}

#[tokio::test]
async fn a_failed_token_command_for_a_host_gh_lists_shows_its_first_stderr_line() {
    let dir = tempfile::tempdir().unwrap();
    let runner = gh(
        dir.path(),
        &format!("echo '{GOOD_STATUS}'"),
        "printf 'could not read the token\\nsecond line\\n' >&2; exit 1",
    );
    let accounts = accounts(runner);
    let rows = accounts.accounts(0).await;
    assert_eq!(
        states(&rows),
        [(
            "github.com",
            CredentialSource::Cli,
            &AccountState::Unusable("could not read the token".into())
        )]
    );
}

#[tokio::test]
async fn a_signed_in_gh_is_a_normal_account() {
    let dir = tempfile::tempdir().unwrap();
    let runner = gh(dir.path(), &format!("echo '{GOOD_STATUS}'"), "echo gho_fake");
    let accounts = accounts(runner);
    let rows = accounts.accounts(0).await;
    assert_eq!(
        states(&rows),
        [("github.com", CredentialSource::Cli, &AccountState::SignedIn)]
    );
    assert_eq!(rows[0].user.as_deref(), Some("octocat"));
    assert!(rows[0].in_use);
    assert_eq!(accounts.cli_unusable("github.com"), None);
}

#[tokio::test]
async fn a_gh_that_does_not_list_the_host_is_just_not_signed_in() {
    let dir = tempfile::tempdir().unwrap();
    let runner = gh(
        dir.path(),
        "echo '{\"hosts\":{}}'",
        "echo 'no oauth token found for github.com' >&2; exit 1",
    );
    let accounts = accounts(runner);
    assert!(accounts.accounts(0).await.is_empty());
}

#[tokio::test]
async fn gh_not_being_installed_shows_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = accounts(FakeGh {
        script: dir.path().join("no-such-gh"),
    });
    assert!(accounts.accounts(0).await.is_empty());
    assert!(accounts.credential("github.com", 0).await.is_none());
}

#[tokio::test]
async fn the_credential_of_an_unusable_gh_is_missing_for_the_reason_gh_gave() {
    let dir = tempfile::tempdir().unwrap();
    let runner = gh(
        dir.path(),
        &format!("echo '{ERROR_STATUS}'"),
        "echo 'no oauth token found for github.com' >&2; exit 1",
    );
    let accounts = accounts(runner);
    assert!(accounts.credential("github.com", 0).await.is_none());
    assert_eq!(
        accounts.cli_unusable("github.com").as_deref(),
        Some("HTTP 401: Bad credentials (https://api.github.com/)")
    );
}

#[tokio::test]
async fn a_failed_gh_call_is_logged_without_its_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let runner = gh(
        dir.path(),
        &format!("echo '{GOOD_STATUS}'"),
        "echo gho_LEAKEDTOKEN; echo 'token store is locked' >&2; exit 1",
    );
    let capture = vet_log::capture::Capture::default();
    let _logging = capture.install(vet_log::filter(Some("debug")).0);
    accounts(runner).accounts(0).await;
    let log = capture.text();
    assert!(
        log.contains("gh auth token --hostname github.com")
            && log.contains("exit_code=Some(1)")
            && log.contains("token store is locked"),
        "{log}"
    );
    assert!(!log.contains("LEAKEDTOKEN"), "{log}");
}
