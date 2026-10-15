//! The credentials Vet uses against hosting platforms (design §4.7). Per host and
//! app-wide, not per session. Held in memory only: a CLI token is asked of the CLI
//! again when it expires, a personal access token is kept for as long as the shell's
//! keychain says so (the shell hands it over, and takes it back on `remove`). A CLI
//! read is one flight per host however many callers ask while it reads, and a read
//! that gave no token is trusted to give none for a moment.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::cli;
use super::credentials::{
    Credential, CredentialProvider, CredentialSource, CredentialValidator, Owner, Token, ValidationError,
};
use super::host::{PlatformKind, normalize_host};
use crate::mutex::lock;
use crate::ports::{Command, ProcessEvent, ProcessRunner, RunningProcess};
use vet_engine::{Computed, Engine, Kind, Limits, Retention};

/// How long a CLI gets to answer; `az` can take a couple of seconds.
const CLI_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a CLI token that does not say when it expires is trusted, in seconds.
const CLI_TOKEN_LIFETIME: i64 = 300;

/// A CLI token that says when it expires is read again this many seconds before.
const EXPIRY_MARGIN: i64 = 120;

/// How long a CLI read that gave no token is trusted to still give none, in seconds.
/// Short, so that signing in from a terminal (`gh auth login`) or repairing a broken
/// sign-in is seen soon after; whatever many requests in those seconds ask, the CLI
/// is spawned once. Vet's own account changes forget it at once.
const ABSENT_LIFETIME: i64 = 30;

/// Hosts whose CLI is looked at even before anything asks for them, so that a signed-in
/// `gh` shows up in the accounts, besides the GitHub Enterprise hosts `gh` says it is signed
/// in to. An `az` token is of no organization in particular, so it shows up with the
/// organizations that are asked for.
const DEFAULT_HOSTS: [&str; 1] = ["github.com"];

/// One read of the CLI of the host it names, the key of the flight that is asked of:
/// however many callers ask while a host's CLI is being read, they join it and share
/// its answer, instead of each spawning the CLI (R6). A host has one CLI.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct CliCall(String);

impl Kind for CliCall {
    type Value = CliRead;

    /// A CLI read cannot fail: [`CliRead`] is all it answers.
    type Error = Infallible;

    /// Nothing is kept of the flight itself: [`Accounts`] keeps what it looked up.
    fn retention(&self) -> Retention {
        Retention::InFlight
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AccountState {
    SignedIn,
    /// The platform refused the credential. Fit to show.
    Rejected(String),
    /// The platform could not be asked. Fit to show.
    Unverified(String),
    /// The CLI lists the host but cannot deliver its token; the CLI's own reason.
    Unusable(String),
}

/// A credential held for a host, without the credential.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Account {
    pub host: String,
    pub kind: PlatformKind,
    pub source: CredentialSource,
    pub user: Option<String>,
    pub state: AccountState,
    /// This is the credential [`Accounts::credential`] hands out for the host.
    pub in_use: bool,
    /// When the credential stops working (unix seconds), if the platform said.
    pub expires_at: Option<i64>,
}

/// Why a credential was not accepted. Every message is fit to show.
#[derive(Clone, Debug)]
pub enum AccountError {
    InvalidHost(String),
    EmptyToken,
    Rejected(String),
    Unreachable(String),
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHost(why) | Self::Rejected(why) | Self::Unreachable(why) => f.write_str(why),
            Self::EmptyToken => f.write_str("the token is empty"),
        }
    }
}

/// What the platform said about a credential.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Identity {
    Unchecked,
    User(Owner),
    Rejected(String),
    Unreachable(String),
}

impl Identity {
    /// A refusal stays true of the same token; anything less is asked again.
    fn needs_check(&self) -> bool {
        matches!(self, Self::Unchecked | Self::Unreachable(_))
    }
}

#[derive(Clone, Debug)]
struct Held {
    token: Token,
    /// Unix seconds from which the token is not trusted any more.
    refresh_at: i64,
    identity: Identity,
}

#[derive(Default)]
struct HostSlot {
    cli: Option<Held>,
    /// Why the CLI lists the host but gave no token, in the CLI's words.
    cli_unusable: Option<String>,
    pat: Option<Held>,
    /// Unix seconds until which a CLI read that gave no token is not asked again.
    absent_until: i64,
}

impl HostSlot {
    fn of(&mut self, source: CredentialSource) -> &mut Option<Held> {
        match source {
            CredentialSource::Cli => &mut self.cli,
            CredentialSource::Pat => &mut self.pat,
        }
    }
}

#[derive(Default)]
struct State {
    hosts: BTreeMap<String, HostSlot>,
    /// Hosts `needed` was called for since they last got a credential.
    asked: BTreeSet<String>,
}

pub struct Accounts<R, V> {
    /// Where the CLIs are looked at. Moved through an `Arc` into a read's flight,
    /// which is owned by the engine, not by the caller that began it.
    runner: Arc<R>,
    validator: V,
    /// Where the CLIs run; they do not depend on it.
    cwd: PathBuf,
    /// The reads of the CLIs, so that the callers asking at the same moment share one.
    flights: Engine<CliCall>,
    /// Called with a host that something needed a credential for and there was none.
    needed: Box<dyn Fn(&str) + Send + Sync>,
    state: Mutex<State>,
}

impl<R, V> Accounts<R, V>
where
    R: ProcessRunner + Send + Sync + 'static,
    R::Process: Send,
    V: CredentialValidator + Sync,
{
    pub fn new(runner: R, validator: V, cwd: PathBuf, needed: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            runner: Arc::new(runner),
            validator,
            cwd,
            flights: Engine::new(Limits::entries(0)),
            needed: Box::new(needed),
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// The credential to use against `host` at `now` (unix seconds): the signed-in CLI's
    /// token, else the personal access token. With neither, `needed` is called once, until
    /// the host gets a credential, and there is none.
    pub async fn credential(&self, host: &str, now: i64) -> Option<Credential> {
        if let Some(token) = self.cli_token(host, now).await {
            return Some(Credential {
                token,
                source: CredentialSource::Cli,
            });
        }
        let pat = self.state().hosts.get(host).and_then(|s| s.pat.clone());
        if let Some(held) = pat {
            return Some(Credential {
                token: held.token,
                source: CredentialSource::Pat,
            });
        }
        let first_time = self.state().asked.insert(host.to_owned());
        if first_time {
            (self.needed)(host);
        }
        None
    }

    /// Whether a credential is held or available from a CLI for `host`. Unlike
    /// [`Self::credential`] it never asks anyone to provide one.
    pub async fn has_account(&self, host: &str, now: i64) -> bool {
        self.cli_token(host, now).await.is_some() || self.state().hosts.get(host).is_some_and(|s| s.pat.is_some())
    }

    /// Whether `token` is the personal access token held for `host` already, as when the shell
    /// hands back what its keychain holds after the platform refused it.
    pub fn holds(&self, host: &str, token: &str) -> bool {
        let (Ok(host), Some(token)) = (normalize_host(host), Token::new(token)) else {
            return false;
        };
        let state = self.state();
        state
            .hosts
            .get(&host)
            .and_then(|slot| slot.pat.as_ref())
            .is_some_and(|held| held.token == token)
    }

    /// Keeps `token` as the personal access token of `host` (as typed or pasted). With
    /// `validate` the platform is asked first and nothing is kept unless it accepts the
    /// token. Answers with the account, whose host is the normalized one.
    pub async fn set_credentials(
        &self,
        host: &str,
        token: &str,
        validate: bool,
        now: i64,
    ) -> Result<Account, AccountError> {
        let host = normalize_host(host).map_err(AccountError::InvalidHost)?;
        let token = Token::new(token).ok_or(AccountError::EmptyToken)?;
        let mut identity = Identity::Unchecked;
        if validate {
            let credential = Credential {
                token: token.clone(),
                source: CredentialSource::Pat,
            };
            identity = match self
                .validator
                .user(&host, PlatformKind::of_account(&host), &credential)
                .await
            {
                Ok(user) => Identity::User(user),
                Err(ValidationError::Rejected(why)) => return Err(AccountError::Rejected(why)),
                Err(ValidationError::Unreachable(why)) => return Err(AccountError::Unreachable(why)),
            };
        }
        {
            let mut state = self.state();
            state.asked.remove(&host);
            let slot = state.hosts.entry(host.clone()).or_default();
            // The host's account changed: whatever the CLI was read to have just now,
            // it is read again for the answer whether the CLI still shadows it.
            slot.absent_until = 0;
            slot.pat = Some(Held {
                token,
                refresh_at: i64::MAX,
                identity,
            });
        }
        // Whether a CLI shadows the token is part of the answer.
        self.cli_token(&host, now).await;
        self.check_identities(Some(&host)).await;
        self.rows()
            .into_iter()
            .find(|a| a.host == host && a.source == CredentialSource::Pat)
            .ok_or_else(|| AccountError::InvalidHost(format!("the token for {host} was removed meanwhile")))
    }

    /// Forgets the personal access token of `host`. Answers with the accounts afterwards.
    pub async fn remove_credentials(&self, host: &str, now: i64) -> Result<Vec<Account>, AccountError> {
        let host = normalize_host(host).map_err(AccountError::InvalidHost)?;
        {
            let mut state = self.state();
            if let Some(slot) = state.hosts.get_mut(&host) {
                slot.pat = None;
                // Signing out is an account change: the CLI is read again, at once.
                slot.absent_until = 0;
            }
        }
        Ok(self.accounts(now).await)
    }

    /// Every credential held or available from a CLI, hosts in name order, a host's CLI
    /// credential before its personal access token. Credentials the platform was not
    /// asked about yet are asked about now.
    pub async fn accounts(&self, now: i64) -> Vec<Account> {
        let signed_in: Vec<String> = match Self::run(&self.runner, &cli::hosts_command(&self.cwd)).await {
            Some(status) => cli::parse_hosts(&status.stdout).into_iter().map(|h| h.host).collect(),
            None => Vec::new(),
        };
        let hosts: BTreeSet<String> = {
            let state = self.state();
            let known = state.hosts.keys().cloned();
            known.chain(DEFAULT_HOSTS.map(str::to_owned)).chain(signed_in).collect()
        };
        for host in &hosts {
            self.cli_token(host, now).await;
        }
        self.check_identities(None).await;
        self.rows()
    }

    fn rows(&self) -> Vec<Account> {
        let state = self.state();
        let mut accounts = Vec::new();
        for (host, slot) in &state.hosts {
            let in_use = if slot.cli.is_some() {
                CredentialSource::Cli
            } else {
                CredentialSource::Pat
            };
            if let Some(why) = &slot.cli_unusable {
                accounts.push(Account {
                    host: host.clone(),
                    kind: PlatformKind::of_account(host),
                    source: CredentialSource::Cli,
                    user: None,
                    state: AccountState::Unusable(why.clone()),
                    in_use: false,
                    expires_at: None,
                });
            }
            for (source, held) in [(CredentialSource::Cli, &slot.cli), (CredentialSource::Pat, &slot.pat)] {
                let Some(held) = held else { continue };
                let (owner, state) = match &held.identity {
                    Identity::User(owner) => (Some(owner), AccountState::SignedIn),
                    Identity::Rejected(why) => (None, AccountState::Rejected(why.clone())),
                    Identity::Unreachable(why) => (None, AccountState::Unverified(why.clone())),
                    Identity::Unchecked => (None, AccountState::Unverified("not checked yet".to_owned())),
                };
                accounts.push(Account {
                    host: host.clone(),
                    kind: PlatformKind::of_account(host),
                    source,
                    user: owner.map(|o| o.user.clone()),
                    state,
                    in_use: source == in_use,
                    expires_at: owner.and_then(|o| o.expires_at),
                });
            }
        }
        accounts
    }

    /// Asks the platforms about the credentials they have not answered for yet: those of
    /// `only` if given, else of every host.
    async fn check_identities(&self, only: Option<&str>) {
        let mut pending = Vec::new();
        for (host, slot) in self
            .state()
            .hosts
            .iter()
            .filter(|(host, _)| only.is_none_or(|o| o == *host))
        {
            for source in [CredentialSource::Cli, CredentialSource::Pat] {
                let held = match source {
                    CredentialSource::Cli => &slot.cli,
                    CredentialSource::Pat => &slot.pat,
                };
                if let Some(held) = held.as_ref().filter(|h| h.identity.needs_check()) {
                    pending.push((host.clone(), source, held.token.clone()));
                }
            }
        }
        for (host, source, token) in pending {
            let credential = Credential { token, source };
            let answer = self
                .validator
                .user(&host, PlatformKind::of_account(&host), &credential)
                .await;
            let mut state = self.state();
            let Some(held) = state.hosts.get_mut(&host).and_then(|slot| slot.of(source).as_mut()) else {
                continue;
            };
            if held.token == credential.token {
                held.identity = match answer {
                    Ok(user) => Identity::User(user),
                    Err(ValidationError::Rejected(why)) => Identity::Rejected(why),
                    Err(ValidationError::Unreachable(why)) => Identity::Unreachable(why),
                };
            }
        }
    }

    /// The signed-in CLI's token for `host`, from what was read earlier while it is
    /// still good, else read from the CLI again: one read for every caller asking
    /// while it reads, and a read that gave no token trusted to give none for a
    /// moment, so that a burst of asks is not a burst of spawns (R6).
    async fn cli_token(&self, host: &str, now: i64) -> Option<Token> {
        let cached = self.state().hosts.get(host).and_then(|s| s.cli.clone());
        if let Some(held) = cached.filter(|h| now < h.refresh_at) {
            return Some(held.token);
        }
        // No token was there moments ago, so it still is not: asking again waits for
        // the window instead of another spawn. Vet's own account changes forget this.
        if self.state().hosts.get(host).is_some_and(|s| now < s.absent_until) {
            return None;
        }
        let call = CliCall(host.to_owned());
        let (runner, cwd) = (Arc::clone(&self.runner), self.cwd.clone());
        let read = match self
            .flights
            .get(call.clone(), move |_| async move {
                // The read cannot fail; the moved-in call names its host.
                Ok::<_, Infallible>(Computed::new(Self::read_cli(&runner, &cwd, &call.0).await, false))
            })
            .await
        {
            Ok(got) => got.value,
            // A CLI read cannot fail.
            Err(never) => match never {},
        };
        let gave_no_token = matches!(read, CliRead::Absent | CliRead::Unusable(_));
        let mut state = self.state();
        let slot = state.hosts.entry(host.to_owned()).or_default();
        slot.cli_unusable = None;
        slot.cli = match read {
            CliRead::Token(cli) => {
                let same_token = slot.cli.as_ref().filter(|old| old.token == cli.token);
                Some(Held {
                    refresh_at: cli.expires_at.map_or(now + CLI_TOKEN_LIFETIME, |at| at - EXPIRY_MARGIN),
                    identity: same_token.map_or(Identity::Unchecked, |old| old.identity.clone()),
                    token: cli.token,
                })
            }
            CliRead::Absent => None,
            CliRead::Unusable(why) => {
                slot.cli_unusable = Some(why);
                None
            }
        };
        // A read that gave no token — the CLI missing, signed out, or its sign-in
        // broken — is trusted to give none again while the window runs, so that a
        // burst of asks is one read, instead of two spawns per ask.
        slot.absent_until = if gave_no_token { now + ABSENT_LIFETIME } else { 0 };
        slot.cli.as_ref().map(|held| held.token.clone())
    }

    /// What the CLI prints. A `gh` that lists the host but cannot deliver its token is
    /// [`CliRead::Unusable`], with `gh`'s own words for why. Standalone over the runner
    /// and the cwd they need: a read belongs to its flight, so outlives any caller.
    async fn read_cli(runner: &R, cwd: &Path, host: &str) -> CliRead {
        let kind = PlatformKind::of_account(host);
        let Some(output) = Self::run(runner, &cli::command(kind, host, cwd)).await else {
            return CliRead::Absent;
        };
        if output.succeeded() {
            return cli::parse(kind, &output.stdout).map_or(CliRead::Absent, CliRead::Token);
        }
        if kind != PlatformKind::GitHub {
            return CliRead::Absent;
        }
        let listed = match Self::run(runner, &cli::hosts_command(cwd)).await {
            Some(status) => cli::parse_hosts(&status.stdout).into_iter().find(|h| h.host == host),
            None => None,
        };
        match listed {
            Some(listed) => CliRead::Unusable(listed.problem.unwrap_or_else(|| output.failure())),
            None => CliRead::Absent,
        }
    }

    /// Runs a command; `None` if it could not be started. Only what is needed to say why a
    /// failure happened is kept of stderr; stdout may hold a token and is never logged.
    async fn run(runner: &R, command: &Command) -> Option<Output> {
        let mut process = runner.spawn(command).ok()?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let finished = async {
            loop {
                match process.next().await {
                    ProcessEvent::Stdout(bytes) => stdout.extend(bytes),
                    ProcessEvent::Stderr(bytes) => stderr.extend(bytes),
                    ProcessEvent::Exit(code) => return code,
                }
            }
        };
        let (code, timed_out) = match tokio::time::timeout(CLI_TIMEOUT, finished).await {
            Ok(code) => (code, false),
            Err(_) => {
                process.kill();
                (None, true)
            }
        };
        let output = Output {
            code,
            timed_out,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr_line: String::from_utf8_lossy(&stderr)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_owned),
        };
        if !output.succeeded() {
            tracing::debug!(
                argv = %format!("{} {}", command.program, command.args.join(" ")),
                exit_code = ?output.code,
                timed_out = output.timed_out,                stderr = output.stderr_line.as_deref().unwrap_or(""),
                "CLI call failed"
            );
        }
        Some(output)
    }
}

/// What a CLI said for a host.
#[derive(Clone, Debug)]
enum CliRead {
    Token(cli::CliToken),
    /// Not installed, not signed in, or it does not answer.
    Absent,
    /// It lists the host and still cannot deliver the token: `gh`'s reason.
    Unusable(String),
}

/// A command that ran.
struct Output {
    /// `None` if a signal ended it or it was killed for taking too long.
    code: Option<i32>,
    timed_out: bool,
    stdout: String,
    /// The first line it printed to stderr.
    stderr_line: Option<String>,
}

impl Output {
    fn succeeded(&self) -> bool {
        self.code == Some(0)
    }

    /// Why it failed, in the command's own words if it had any.
    fn failure(&self) -> String {
        if let Some(line) = &self.stderr_line {
            return line.clone();
        }
        match self.code {
            Some(code) => format!("exited with code {code}"),
            None if self.timed_out => "did not answer in time".to_owned(),
            None => "was ended by a signal".to_owned(),
        }
    }
}

impl<R, V> CredentialProvider for Accounts<R, V>
where
    R: ProcessRunner + Send + Sync + 'static,
    R::Process: Send,
    V: CredentialValidator + Sync,
{
    fn credential(&self, host: &str, now: i64) -> impl Future<Output = Option<Credential>> + Send {
        Accounts::credential(self, host, now)
    }

    fn cli_unusable(&self, host: &str) -> Option<String> {
        self.state().hosts.get(host).and_then(|s| s.cli_unusable.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use futures::future::join_all;

    use super::*;

    /// The CLIs on this machine: what each program prints and its exit code, changeable
    /// between calls. A program that is not in the script is not installed. A script for a
    /// command line (`gh auth status`) comes before the program's own. What is recorded of
    /// a spawn is its whole command line.
    #[derive(Clone, Default)]
    struct Machine {
        programs: Arc<Mutex<HashMap<String, (i32, String)>>>,
        spawned: Arc<Mutex<Vec<String>>>,
        /// Programs whose next spawn holds until the test lets it answer.
        holds: Arc<Mutex<HashMap<String, Hold>>>,
    }

    /// The hold [`Machine::hold`]'s spawn gets: how it tells the test it went out,
    /// and the release the test answers it with.
    type Hold = (tokio::sync::oneshot::Sender<()>, Arc<tokio::sync::Notify>);

    impl Machine {
        fn install(&self, program: &str, code: i32, stdout: &str) {
            self.programs
                .lock()
                .unwrap()
                .insert(program.to_owned(), (code, stdout.to_owned()));
        }

        /// How often a command line was spawned, by its first words.
        fn runs_of(&self, command: &str) -> usize {
            self.spawned
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.starts_with(command))
                .count()
        }

        /// Makes `program`'s next spawn hold until the test says so. What comes back:
        /// when the held spawn went out, and how to let it go.
        fn hold(&self, program: &str) -> Gate {
            let (sender, held) = tokio::sync::oneshot::channel();
            let release = Arc::new(tokio::sync::Notify::new());
            self.holds
                .lock()
                .unwrap()
                .insert(program.to_owned(), (sender, Arc::clone(&release)));
            Gate { held, release }
        }
    }

    /// A fake CLI held before it answers, on behalf of the test: it learns when the
    /// spawn went out, and holds it until `release`; only then is its read over.
    struct Gate {
        /// The spawned and held CLI.
        held: tokio::sync::oneshot::Receiver<()>,
        /// `notify_one` and the held read goes on.
        release: Arc<tokio::sync::Notify>,
    }

    struct Script {
        events: std::vec::IntoIter<ProcessEvent>,
        /// Set once for the held spawn: the script answers when the test does.
        hold: Option<(tokio::sync::oneshot::Sender<()>, Arc<tokio::sync::Notify>)>,
    }

    impl RunningProcess for Script {
        async fn next(&mut self) -> ProcessEvent {
            if let Some((held, release)) = self.hold.take() {
                let _ = held.send(());
                // Held on until the test released it, so that the read stays in flight
                // while the other callers asking for an answer arrive on it.
                release.notified().await;
            }
            self.events.next().unwrap_or(ProcessEvent::Exit(None))
        }

        fn kill(&self) {}
    }

    impl ProcessRunner for Machine {
        type Process = Script;

        fn spawn(&self, command: &Command) -> Result<Script, String> {
            let line = format!("{} {} ", command.program, command.args.join(" "));
            self.spawned.lock().unwrap().push(line.trim_end().to_owned());
            let programs = self.programs.lock().unwrap();
            let (code, stdout) = programs
                .iter()
                .filter(|(script, _)| line.starts_with(&format!("{script} ")))
                .max_by_key(|(script, _)| script.len())
                .map(|(_, answer)| answer.clone())
                .ok_or_else(|| format!("{}: not found", command.program))?;
            let events = vec![
                ProcessEvent::Stdout(stdout.into_bytes()),
                ProcessEvent::Exit(Some(code)),
            ];
            let hold = self.holds.lock().unwrap().remove(command.program.as_str());
            Ok(Script {
                events: events.into_iter(),
                hold,
            })
        }
    }

    /// The platforms: which token belongs to whom, and what they were asked.
    #[derive(Clone, Default)]
    struct Platforms {
        tokens: Arc<Mutex<HashMap<String, Result<Owner, ValidationError>>>>,
        asked: Arc<Mutex<Vec<(String, PlatformKind, CredentialSource)>>>,
    }

    impl Platforms {
        fn knows(&self, token: &str, answer: Result<&str, ValidationError>) {
            self.tokens
                .lock()
                .unwrap()
                .insert(token.to_owned(), answer.map(Owner::named));
        }

        fn knows_expiring(&self, token: &str, user: &str, expires_at: i64) {
            let owner = Owner {
                user: user.to_owned(),
                expires_at: Some(expires_at),
            };
            self.tokens.lock().unwrap().insert(token.to_owned(), Ok(owner));
        }
    }

    impl CredentialValidator for Platforms {
        async fn user(
            &self,
            host: &str,
            kind: PlatformKind,
            credential: &Credential,
        ) -> Result<Owner, ValidationError> {
            self.asked
                .lock()
                .unwrap()
                .push((host.to_owned(), kind, credential.source));
            self.tokens
                .lock()
                .unwrap()
                .get(credential.token.expose())
                .cloned()
                .unwrap_or_else(|| Err(ValidationError::Rejected("Bad credentials".to_owned())))
        }
    }

    struct Fixture {
        accounts: Arc<Accounts<Machine, Platforms>>,
        machine: Machine,
        platforms: Platforms,
        needed: Arc<Mutex<Vec<String>>>,
    }

    fn fixture() -> Fixture {
        let machine = Machine::default();
        let platforms = Platforms::default();
        let needed = Arc::new(Mutex::new(Vec::new()));
        let record = needed.clone();
        let accounts = Arc::new(Accounts::new(
            machine.clone(),
            platforms.clone(),
            PathBuf::from("/"),
            move |host| {
                record.lock().unwrap().push(host.to_owned());
            },
        ));
        Fixture {
            accounts,
            machine,
            platforms,
            needed,
        }
    }

    fn az_output(token: &str, expires_on: i64) -> String {
        format!(r#"{{"accessToken":"{token}","expires_on":{expires_on}}}"#)
    }

    /// The reason `gh` gives for the host whose sign-in is broken.
    const BROKEN_SIGN_IN: &str = "HTTP 401: Bad credentials";

    /// The `gh` of a host whose sign-in is broken: it exits non-zero for the token, and
    /// lists the host with `gh`'s own reason.
    fn gh_with_a_broken_sign_in(machine: &Machine, reason: &str) {
        machine.install("gh", 1, "");
        let listing = serde_json::json!({
            "hosts": { "github.com": [{ "active": true, "state": "error", "error": reason }] }
        });
        machine.install("gh auth status", 0, &listing.to_string());
    }

    async fn token_for(
        accounts: &Accounts<Machine, Platforms>,
        host: &str,
        now: i64,
    ) -> Option<(String, CredentialSource)> {
        let credential = accounts.credential(host, now).await?;
        Some((credential.token.expose().to_owned(), credential.source))
    }

    #[tokio::test]
    async fn the_cli_comes_before_a_personal_access_token_which_comes_before_nothing() {
        let f = fixture();
        assert_eq!(token_for(&f.accounts, "github.com", 0).await, None);

        f.accounts
            .set_credentials("github.com", "ghp_pat", false, 0)
            .await
            .unwrap();
        assert_eq!(
            token_for(&f.accounts, "github.com", 0).await,
            Some(("ghp_pat".into(), CredentialSource::Pat))
        );

        f.machine.install("gh", 0, "gho_cli\n");
        // The appearance of a CLI in the shell is not seen in what Vet trusts missing,
        // until the memory of it just now ran out; a sign in from a terminal is picked
        // up at the next read after it.
        assert_eq!(
            token_for(&f.accounts, "github.com", ABSENT_LIFETIME + 1).await,
            Some(("gho_cli".into(), CredentialSource::Cli))
        );
    }

    #[tokio::test]
    async fn asking_whether_a_host_has_an_account_never_asks_the_shell_for_one() {
        let f = fixture();
        assert!(!f.accounts.has_account("gitlab.com", 0).await);
        f.machine.install("gh", 0, "gho_cli\n");
        assert!(f.accounts.has_account("github.com", 0).await);
        f.accounts
            .set_credentials("ghe.corp.example", "ghp_pat", false, 0)
            .await
            .unwrap();
        assert!(f.accounts.has_account("ghe.corp.example", 0).await);
        assert!(f.needed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_cli_that_is_missing_or_signed_out_is_passed_over() {
        let f = fixture();
        f.accounts
            .set_credentials("github.com", "ghp_pat", false, 0)
            .await
            .unwrap();
        // `gh` is not installed.
        assert_eq!(
            token_for(&f.accounts, "github.com", 0).await.unwrap().1,
            CredentialSource::Pat
        );
        // `gh` is installed but not signed in to the host: it exits non-zero.
        f.machine.install("gh", 1, "");
        assert_eq!(
            token_for(&f.accounts, "github.com", 0).await.unwrap().1,
            CredentialSource::Pat
        );
    }

    #[tokio::test]
    async fn azure_devops_asks_az_and_not_gh() {
        let f = fixture();
        f.machine.install("gh", 0, "gho_cli");
        f.machine.install("az", 0, &az_output("eyJ_entra", 1_000_000));
        assert_eq!(
            token_for(&f.accounts, "dev.azure.com", 0).await,
            Some(("eyJ_entra".into(), CredentialSource::Cli))
        );
        assert_eq!(f.machine.runs_of("gh"), 0);
    }

    #[tokio::test]
    async fn a_cli_token_is_read_again_once_it_has_expired() {
        let f = fixture();
        f.machine.install("az", 0, &az_output("first", 10_000));
        assert_eq!(token_for(&f.accounts, "dev.azure.com", 1_000).await.unwrap().0, "first");

        f.machine.install("az", 0, &az_output("second", 20_000));
        assert_eq!(token_for(&f.accounts, "dev.azure.com", 5_000).await.unwrap().0, "first");
        assert_eq!(
            f.machine.runs_of("az"),
            1,
            "a token that is still good is not asked for again"
        );

        assert_eq!(
            token_for(&f.accounts, "dev.azure.com", 10_000).await.unwrap().0,
            "second"
        );
        assert_eq!(f.machine.runs_of("az"), 2);
    }

    #[tokio::test]
    async fn a_cli_token_that_does_not_say_when_it_expires_is_read_again_after_a_while() {
        let f = fixture();
        f.machine.install("gh", 0, "gho_first");
        assert_eq!(token_for(&f.accounts, "github.com", 0).await.unwrap().0, "gho_first");

        f.machine.install("gh", 0, "gho_second");
        assert_eq!(token_for(&f.accounts, "github.com", 60).await.unwrap().0, "gho_first");
        assert_eq!(
            token_for(&f.accounts, "github.com", 3_600).await.unwrap().0,
            "gho_second"
        );
    }

    #[tokio::test]
    async fn callers_asking_while_a_host_s_cli_reads_share_its_one_read() {
        let f = fixture();
        // `gh` is installed and signed out: its read is a miss, and it is held in
        // flight until the test lets it go.
        f.machine.install("gh", 1, "");
        let gate = f.machine.hold("gh");
        let reads = tokio::spawn({
            let accounts = Arc::clone(&f.accounts);
            async move {
                // Many requests for the host decide at the same moment, all of them a
                // miss, that they need its CLI's token.
                let reads = (0..5).map(|_| {
                    let accounts = Arc::clone(&accounts);
                    async move { accounts.credential("github.com", 0).await }
                });
                join_all(reads).await
            }
        });
        gate.held.await.unwrap();
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            1,
            "however many askers while the read is in flight, the one spawn"
        );
        gate.release.notify_one();
        let reads = reads.await.unwrap();
        assert_eq!(reads.len(), 5);
        assert!(reads.into_iter().all(|read| read.is_none()));
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            1,
            "so it stayed, all of it being one read"
        );
    }

    #[tokio::test]
    async fn a_second_miss_within_a_short_window_of_the_first_spawns_the_cli_nothing_more() {
        let f = fixture();
        // `gh` is installed and signed out: every read is a miss.
        f.machine.install("gh", 1, "");
        assert!(f.accounts.credential("github.com", 0).await.is_none());
        assert_eq!(f.machine.runs_of("gh auth token"), 1);

        // A moment later the answer is remembered, whatever asked again.
        f.accounts.credential("github.com", 1).await;
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            1,
            "still inside the window: no spawn"
        );

        // Past the window the CLI is asked again, its answer unknown meanwhile.
        f.accounts.credential("github.com", ABSENT_LIFETIME + 1).await;
        assert_eq!(f.machine.runs_of("gh auth token"), 2, "the window ran out: asked again");
    }

    #[tokio::test]
    async fn a_host_s_account_change_asks_the_cli_again_right_away() {
        let f = fixture();
        // `gh` is installed but signed out: a miss, remembered for a moment.
        f.machine.install("gh", 1, "");
        assert!(f.accounts.credential("github.com", 0).await.is_none());
        f.accounts.credential("github.com", 1).await;
        assert_eq!(f.machine.runs_of("gh auth token"), 1, "the miss is remembered");

        // Signing in holds a token of the host in the very moment `gh` is signed in
        // here: the shadow answer has the CLI read again, though the miss was fresh.
        f.machine.install("gh", 0, "gho_cli\n");
        f.accounts
            .set_credentials("github.com", "ghp_pat", false, 5)
            .await
            .unwrap();
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            2,
            "read again at the change, instead of trusted missing"
        );
        assert_eq!(
            token_for(&f.accounts, "github.com", 6).await.map(|(_, source)| source),
            Some(CredentialSource::Cli)
        );
    }

    #[tokio::test]
    async fn signing_out_of_a_host_asks_the_cli_again_right_away() {
        let f = fixture();
        // `gh` is installed but signed out: a miss, remembered for a moment.
        f.machine.install("gh", 1, "");
        assert!(f.accounts.credential("github.com", 0).await.is_none());
        assert_eq!(f.machine.runs_of("gh auth token"), 1);

        f.accounts.remove_credentials("github.com", 1).await.unwrap();
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            2,
            "the accounts answer of the sign out read the CLI again, though the miss was fresh"
        );
    }

    #[tokio::test]
    async fn a_broken_cli_sign_in_is_read_once_for_a_burst_of_asks() {
        let f = fixture();
        gh_with_a_broken_sign_in(&f.machine, BROKEN_SIGN_IN);
        assert!(f.accounts.credential("github.com", 0).await.is_none());
        assert_eq!(
            f.machine.runs_of("gh"),
            2,
            "the token ask and the host listing of the one read"
        );

        // Whatever asks again while the window runs is answered from that one read.
        for now in 1..ABSENT_LIFETIME {
            f.accounts.credential("github.com", now).await;
        }
        assert_eq!(f.machine.runs_of("gh"), 2, "still inside the window: no spawn");

        // The window ran out: the next ask reads the CLI again.
        f.accounts.credential("github.com", ABSENT_LIFETIME + 1).await;
        assert_eq!(f.machine.runs_of("gh"), 4, "the window ran out: read again");
    }

    #[tokio::test]
    async fn a_broken_cli_sign_in_keeps_showing_as_unusable_while_the_window_runs() {
        let f = fixture();
        gh_with_a_broken_sign_in(&f.machine, BROKEN_SIGN_IN);
        assert!(f.accounts.credential("github.com", 0).await.is_none());

        for now in [0, 1, ABSENT_LIFETIME - 1] {
            let stated: Vec<_> = f
                .accounts
                .accounts(now)
                .await
                .into_iter()
                .filter(|account| account.host == "github.com")
                .map(|account| account.state)
                .collect();
            assert_eq!(stated, [AccountState::Unusable(BROKEN_SIGN_IN.to_owned())], "at {now}");
        }
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            1,
            "the listings were answered from the one read"
        );
    }

    #[tokio::test]
    async fn a_repaired_cli_sign_in_is_seen_once_the_window_ran_out() {
        let f = fixture();
        gh_with_a_broken_sign_in(&f.machine, BROKEN_SIGN_IN);
        assert!(f.accounts.credential("github.com", 0).await.is_none());
        assert_eq!(f.accounts.cli_unusable("github.com").as_deref(), Some(BROKEN_SIGN_IN));

        // `gh auth login` repaired it while the window ran; the next read past the
        // window sees the token, so the fix lands within the window's reach.
        f.machine.install("gh", 0, "gho_cli\n");
        assert_eq!(
            token_for(&f.accounts, "github.com", ABSENT_LIFETIME + 1).await,
            Some(("gho_cli".into(), CredentialSource::Cli))
        );
        assert_eq!(f.accounts.cli_unusable("github.com"), None, "no longer unusable");
    }

    #[tokio::test]
    async fn a_vet_sign_in_forgets_a_broken_cli_sign_in_right_away() {
        let f = fixture();
        gh_with_a_broken_sign_in(&f.machine, BROKEN_SIGN_IN);
        assert!(f.accounts.credential("github.com", 0).await.is_none());
        assert_eq!(f.machine.runs_of("gh auth token"), 1);

        f.accounts
            .set_credentials("github.com", "ghp_pat", false, 5)
            .await
            .unwrap();
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            2,
            "read again at the change, instead of trusted broken"
        );
    }

    #[tokio::test]
    async fn a_vet_sign_out_forgets_a_broken_cli_sign_in_right_away() {
        let f = fixture();
        gh_with_a_broken_sign_in(&f.machine, BROKEN_SIGN_IN);
        assert!(f.accounts.credential("github.com", 0).await.is_none());
        assert_eq!(f.machine.runs_of("gh auth token"), 1);

        f.accounts.remove_credentials("github.com", 1).await.unwrap();
        assert_eq!(
            f.machine.runs_of("gh auth token"),
            2,
            "the accounts answer of the sign out read the CLI again, though the sign-in was broken just now"
        );
    }

    #[tokio::test]
    async fn signing_out_of_the_cli_ends_its_credential_at_the_next_read() {
        let f = fixture();
        f.machine.install("gh", 0, "gho_cli");
        assert!(f.accounts.credential("github.com", 0).await.is_some());

        f.machine.install("gh", 1, "");
        assert!(f.accounts.credential("github.com", 3_600).await.is_none());
        assert_eq!(*f.needed.lock().unwrap(), ["github.com"]);
    }

    #[tokio::test]
    async fn a_host_without_a_credential_is_reported_once_until_it_gets_one() {
        let f = fixture();
        f.accounts.credential("ghe.corp.example", 0).await;
        f.accounts.credential("ghe.corp.example", 1).await;
        f.accounts.credential("github.com", 2).await;
        assert_eq!(*f.needed.lock().unwrap(), ["ghe.corp.example", "github.com"]);

        f.accounts
            .set_credentials("ghe.corp.example", "pat", false, 3)
            .await
            .unwrap();
        assert!(f.accounts.credential("ghe.corp.example", 4).await.is_some());
        f.accounts.remove_credentials("ghe.corp.example", 5).await.unwrap();
        f.accounts.credential("ghe.corp.example", 6).await;
        assert_eq!(
            f.needed.lock().unwrap().len(),
            3,
            "asked again after the credential went"
        );
    }

    #[tokio::test]
    async fn a_token_handed_back_is_known_as_held_and_another_one_is_not() {
        let f = fixture();
        assert!(!f.accounts.holds("github.com", "ghp_pat"));
        f.accounts
            .set_credentials("github.com", "ghp_pat", false, 0)
            .await
            .unwrap();
        assert!(f.accounts.holds("GitHub.com", " ghp_pat "));
        assert!(!f.accounts.holds("github.com", "ghp_other"));
        assert!(!f.accounts.holds("ghe.corp.example", "ghp_pat"));
    }

    #[tokio::test]
    async fn a_token_the_platform_accepts_is_kept_with_its_user() {
        let f = fixture();
        f.platforms.knows("ghp_good", Ok("octocat"));
        let account = f
            .accounts
            .set_credentials("GitHub.com", " ghp_good ", true, 0)
            .await
            .unwrap();
        assert_eq!(
            account,
            Account {
                host: "github.com".into(),
                kind: PlatformKind::GitHub,
                source: CredentialSource::Pat,
                user: Some("octocat".into()),
                state: AccountState::SignedIn,
                in_use: true,
                expires_at: None,
            }
        );
        assert_eq!(token_for(&f.accounts, "github.com", 0).await.unwrap().0, "ghp_good");
    }

    #[tokio::test]
    async fn a_token_shows_when_it_expires_if_the_platform_says() {
        let f = fixture();
        f.platforms.knows_expiring("ghp_fine", "octocat", 1_900_000_000);
        f.accounts
            .set_credentials("github.com", "ghp_fine", false, 0)
            .await
            .unwrap();
        let accounts = f.accounts.accounts(0).await;
        assert_eq!(accounts[0].expires_at, Some(1_900_000_000));
    }

    #[tokio::test]
    async fn a_token_the_platform_refuses_is_not_kept() {
        let f = fixture();
        f.platforms
            .knows("ghp_bad", Err(ValidationError::Rejected("Bad credentials".into())));
        let error = f
            .accounts
            .set_credentials("github.com", "ghp_bad", true, 0)
            .await
            .unwrap_err();
        assert!(matches!(error, AccountError::Rejected(why) if why == "Bad credentials"));
        assert!(f.accounts.accounts(0).await.is_empty());
        assert!(f.accounts.credential("github.com", 0).await.is_none());
    }

    #[tokio::test]
    async fn a_token_is_not_kept_when_the_platform_cannot_be_asked() {
        let f = fixture();
        f.platforms
            .knows("ghp_x", Err(ValidationError::Unreachable("no route to host".into())));
        let error = f
            .accounts
            .set_credentials("ghe.corp.example", "ghp_x", true, 0)
            .await
            .unwrap_err();
        assert!(matches!(error, AccountError::Unreachable(_)));
        assert!(f.accounts.accounts(0).await.is_empty());
    }

    #[tokio::test]
    async fn the_platform_is_asked_in_the_manner_of_the_host() {
        let f = fixture();
        f.platforms.knows("pat", Ok("someone"));
        f.accounts
            .set_credentials("https://Contoso.visualstudio.com", "pat", true, 0)
            .await
            .unwrap();
        f.accounts
            .set_credentials("ghe.corp.example", "pat", true, 0)
            .await
            .unwrap();
        let asked = f.platforms.asked.lock().unwrap().clone();
        assert!(asked.contains(&(
            "dev.azure.com/contoso".into(),
            PlatformKind::AzureDevOps,
            CredentialSource::Pat
        )));
        assert!(asked.contains(&("ghe.corp.example".into(), PlatformKind::GitHub, CredentialSource::Pat)));
    }

    #[tokio::test]
    async fn blank_tokens_and_hosts_that_are_none_are_refused() {
        let f = fixture();
        assert!(matches!(
            f.accounts.set_credentials("github.com", "  ", false, 0).await,
            Err(AccountError::EmptyToken)
        ));
        assert!(matches!(
            f.accounts.set_credentials("not a host", "pat", false, 0).await,
            Err(AccountError::InvalidHost(_))
        ));
    }

    #[tokio::test]
    async fn the_accounts_show_the_cli_in_use_and_a_token_it_shadows() {
        let f = fixture();
        f.machine.install("gh", 0, "gho_cli");
        f.platforms.knows("gho_cli", Ok("octocat"));
        f.platforms.knows("ghp_pat", Ok("octocat-work"));
        f.accounts
            .set_credentials("github.com", "ghp_pat", false, 0)
            .await
            .unwrap();

        let accounts = f.accounts.accounts(0).await;
        let summary: Vec<_> = accounts
            .iter()
            .map(|a| (a.host.as_str(), a.source, a.user.as_deref(), a.in_use))
            .collect();
        assert_eq!(
            summary,
            [
                ("github.com", CredentialSource::Cli, Some("octocat"), true),
                ("github.com", CredentialSource::Pat, Some("octocat-work"), false),
            ]
        );
    }

    #[tokio::test]
    async fn the_accounts_show_every_host_gh_is_signed_in_to_with_that_hosts_token() {
        let f = fixture();
        f.machine.install(
            "gh auth status",
            0,
            r#"{"hosts":{"ghe.corp.example":[{"active":true,"login":"ada","state":"success"}]}}"#,
        );
        f.machine
            .install("gh auth token --hostname ghe.corp.example", 0, "gho_enterprise");
        f.platforms.knows("gho_enterprise", Ok("ada"));

        let accounts = f.accounts.accounts(0).await;
        let summary: Vec<_> = accounts
            .iter()
            .map(|a| (a.host.as_str(), a.kind, a.source, a.user.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [(
                "ghe.corp.example",
                PlatformKind::GitHub,
                CredentialSource::Cli,
                Some("ada")
            )]
        );
        assert_eq!(
            token_for(&f.accounts, "ghe.corp.example", 0).await,
            Some(("gho_enterprise".into(), CredentialSource::Cli))
        );
    }

    #[tokio::test]
    async fn a_stored_token_that_the_platform_no_longer_accepts_shows_as_rejected() {
        let f = fixture();
        f.accounts
            .set_credentials("github.com", "ghp_revoked", false, 0)
            .await
            .unwrap();
        let accounts = f.accounts.accounts(0).await;
        assert_eq!(accounts[0].state, AccountState::Rejected("Bad credentials".into()));
    }

    #[tokio::test]
    async fn removing_forgets_the_token_but_not_a_cli() {
        let f = fixture();
        f.machine.install("gh", 0, "gho_cli");
        f.accounts
            .set_credentials("github.com", "ghp_pat", false, 0)
            .await
            .unwrap();
        let accounts = f.accounts.remove_credentials("github.com", 0).await.unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].source, CredentialSource::Cli);
    }

    #[test]
    fn tokens_are_not_in_debug_output() {
        let credential = Credential {
            token: Token::new("ghp_supersecret").unwrap(),
            source: CredentialSource::Pat,
        };
        let held = Held {
            token: credential.token.clone(),
            refresh_at: 0,
            identity: Identity::Unchecked,
        };
        for shown in [
            format!("{credential:?}"),
            format!("{held:?}"),
            format!("{:?}", credential.token),
        ] {
            assert!(!shown.contains("supersecret"), "{shown}");
        }
    }
}
