//! Keeping remote-tracking refs current (SPEC §4.3): the remotes of the repository are fetched,
//! as one background task, on demand and on a timer. Only the viewer's fetch reaches every remote,
//! as git is configured to; what runs without the viewer asking, and every pull request fetch,
//! takes only the remotes on platform hosts there is an account for, over HTTPS with the account's
//! token, so it never runs `ssh`. A fetch that cannot be done (offline, refused) ends its task as
//! failed and is otherwise harmless: the review does not wait for it or depend on it.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::mutex::lock;
use crate::platform::{Credential, PlatformKind, parse_remote};
use crate::ports::{FetchAuth, Fetcher, GitError, RefTip, RefUpdate, RemoteInfo, Via};
use crate::progress::{TaskKind, Tasks};

/// How often a project's remotes are fetched while it is open.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Where a fetch finds the platform credential for the host of an HTTPS remote.
pub trait Credentials {
    fn credential(&self, host: &str) -> impl Future<Output = Option<Credential>> + Send;
    /// Whether there is an account for `host`. Unlike asking for its credential, this never tells
    /// the viewer that one is needed.
    fn has_account(&self, host: &str) -> impl Future<Output = bool> + Send;
}

pub struct Fetch<G> {
    git: G,
    tasks: Tasks,
    /// A fetch started while another runs waits for it: both write the same refs. Refs are
    /// deleted under it too.
    running: Mutex<()>,
    listed_heads: Mutex<()>,
    /// How many times [`Fetch::refs`] was called, and the count when each ref was last fetched.
    fetched: StdMutex<(u64, HashMap<String, u64>)>,
    /// The refs a remote refused when asked for them alone.
    lost: StdMutex<HashSet<String>>,
}

/// A moment in the life of a [`Fetch`]: refs fetched after it are newer than anything known at it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FetchMark(u64);

impl<G: Fetcher> Fetch<G> {
    pub fn new(git: G, tasks: Tasks) -> Self {
        Self {
            git,
            tasks,
            running: Mutex::new(()),
            listed_heads: Mutex::new(()),
            fetched: StdMutex::default(),
            lost: StdMutex::default(),
        }
    }

    fn fetched(&self) -> std::sync::MutexGuard<'_, (u64, HashMap<String, u64>)> {
        lock(&self.fetched)
    }

    /// Now, for [`Fetch::delete_refs`] to keep what is fetched after it.
    pub fn mark(&self) -> FetchMark {
        FetchMark(self.fetched().0)
    }

    /// The refs whose names start with `prefix`, each with its commit.
    pub async fn refs_below(&self, prefix: &str) -> Result<Vec<RefTip>, GitError> {
        self.git.refs_below(prefix).await
    }

    /// Whether the commit `descendant` is `ancestor` or has it in its history.
    pub async fn descends_from(&self, descendant: &str, ancestor: &str) -> Result<bool, GitError> {
        self.git.descends_from(descendant, ancestor).await
    }

    /// Held while the pull requests of a listing are brought in: a second listing waits, and then
    /// finds what the first fetched instead of fetching it again.
    pub async fn listed_heads(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.listed_heads.lock().await
    }

    /// Deletes those of the refs `names` that were not fetched since `since`, which must all be
    /// below `refs/vet/pr/`. Answers with the names it did not keep, there or not.
    pub async fn delete_refs(&self, names: Vec<String>, since: FetchMark) -> Result<Vec<String>, GitError> {
        let _only_one = self.running.lock().await;
        let doomed: Vec<String> = {
            let fetched = self.fetched();
            names
                .into_iter()
                .filter(|name| fetched.1.get(name).is_none_or(|at| *at <= since.0))
                .collect()
        };
        if doomed.is_empty() {
            return Ok(doomed);
        }
        self.git.delete_refs(doomed.clone()).await?;
        let mut fetched = self.fetched();
        for name in &doomed {
            fetched.1.remove(name);
        }
        Ok(doomed)
    }

    /// The remotes that have a fetch URL.
    pub async fn remotes(&self) -> Result<Vec<RemoteInfo>, GitError> {
        self.git.remotes().await
    }

    /// Fetches exactly `updates` from `remote`, as a task called `title`. The error is the
    /// task's failure, fit to show.
    pub async fn refs(
        &self,
        credentials: &(impl Credentials + Sync),
        remote: &RemoteInfo,
        updates: Vec<RefUpdate>,
        title: String,
    ) -> Result<(), GitError> {
        let _only_one = self.running.lock().await;
        let task = self.tasks.start(TaskKind::Fetch, title);
        let Some(via) = platform_route(credentials, remote).await else {
            let message = no_account(remote);
            task.fail(message.clone());
            return Err(GitError::Remote(message));
        };
        self.note_fetching(&updates);
        match self.git.fetch_refs(&remote.name, updates, via).await {
            Ok(()) => {
                task.finish();
                Ok(())
            }
            Err(e) => {
                tracing::warn!(remote = %remote.name, error = %e, "fetching refs failed");
                let message = format!("{}: {e}", remote.name);
                task.fail(message.clone());
                Err(GitError::Remote(message))
            }
        }
    }

    /// Fetches `updates` of the sort a remote may well refuse (a commit it dropped), with no task
    /// to fail for it. A ref the remote refused alone is not asked for again by [`Fetch::lost`].
    pub async fn refs_asking(
        &self,
        credentials: &(impl Credentials + Sync),
        remote: &RemoteInfo,
        updates: Vec<RefUpdate>,
    ) -> Result<(), GitError> {
        let _only_one = self.running.lock().await;
        let Some(via) = platform_route(credentials, remote).await else {
            return Err(GitError::Remote(no_account(remote)));
        };
        self.note_fetching(&updates);
        self.git
            .fetch_refs(&remote.name, updates, via)
            .await
            .map_err(|e| GitError::Remote(format!("{}: {e}", remote.name)))
    }

    /// Notes the refs `updates` write as fetched now.
    fn note_fetching(&self, updates: &[RefUpdate]) {
        let mut fetched = self.fetched();
        fetched.0 += 1;
        let at = fetched.0;
        for update in updates {
            fetched.1.insert(update.destination.clone(), at);
        }
    }

    /// Remembers that the remote refused the ref `destination` when asked for it alone.
    pub fn refused(&self, destination: &str) {
        lock(&self.lost).insert(destination.to_owned());
    }

    /// The remote refused the ref `destination` when asked for it alone before.
    pub fn lost(&self, destination: &str) -> bool {
        lock(&self.lost).contains(destination)
    }

    /// Fetches the remotes `started` calls for, one after the other, as one task; a remote that
    /// fails does not stop the others, and what they brought counts. A background fetch takes the
    /// remotes on platform hosts there is an account for and leaves the others alone, so it never
    /// runs `ssh`; the viewer's fetch takes every remote. Fails only if the remotes cannot be read.
    pub async fn run(&self, credentials: &(impl Credentials + Sync), started: Started) -> Result<Ran, GitError> {
        let _only_one = self.running.lock().await;
        let remotes = self.git.remotes().await?;
        let mut routed = Vec::new();
        for remote in &remotes {
            let via = match (platform_route(credentials, remote).await, started) {
                (Some(via), _) => via,
                (None, Started::Viewer) => Via::Configured(authentication(credentials, remote).await),
                (None, Started::Background) => continue,
            };
            routed.push((remote, via));
        }
        if routed.is_empty() {
            return Ok(Ran {
                fetched: 0,
                updated: 0,
                failure: None,
                platform_remotes: 0,
                platform_failure: None,
            });
        }
        let platform_remotes = routed
            .iter()
            .filter(|(_, via)| matches!(via, Via::Platform { .. }))
            .count();
        let names: Vec<&str> = routed.iter().map(|(remote, _)| remote.name.as_str()).collect();
        let task = self.tasks.start(TaskKind::Fetch, title(&names));
        let total = routed.len() as u32;
        let mut failures = Vec::new();
        let mut platform_failures = Vec::new();
        let mut updated = 0;
        for (done, (remote, via)) in routed.iter().enumerate() {
            task.report(Some((done as u32, total)), Some(remote.name.clone()));
            let platform = matches!(via, Via::Platform { .. });
            match self.git.fetch(&remote.name, via.clone()).await {
                Ok(refs) => updated += refs,
                Err(e) => {
                    tracing::warn!(remote = %remote.name, error = %e, "fetch failed");
                    let failure = format!("{}: {e}", remote.name);
                    if platform {
                        platform_failures.push(failure.clone());
                    }
                    failures.push(failure);
                }
            }
        }
        let fetched = routed.len() - failures.len();
        let joined = |failures: Vec<String>| (!failures.is_empty()).then(|| failures.join("; "));
        let failure = joined(failures);
        match &failure {
            Some(message) => task.fail(message.clone()),
            None => task.finish(),
        }
        Ok(Ran {
            fetched,
            updated,
            failure,
            platform_remotes,
            platform_failure: joined(platform_failures),
        })
    }
}

/// Who a fetch is for: what it may reach.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Started {
    /// Vet itself: only the remotes on platform hosts there is an account for, over HTTPS.
    Background,
    /// The viewer: every remote.
    Viewer,
}

/// What [`Fetch::run`] did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ran {
    /// How many remotes were fetched.
    pub fetched: usize,
    /// How many refs the fetched remotes updated.
    pub updated: usize,
    /// Why the others were not, as the task failed with it, naming each remote.
    pub failure: Option<String>,
    /// How many of the remotes the run took are on platform hosts, whether they were fetched or not.
    pub platform_remotes: usize,
    /// The part of `failure` that is of remotes on platform hosts.
    pub platform_failure: Option<String>,
}

/// The platform credential to present to `remote`, if it is fetched over HTTPS from a host there is
/// one for.
async fn authentication(credentials: &(impl Credentials + Sync), remote: &RemoteInfo) -> Option<FetchAuth> {
    let host = remote.https_host.as_ref()?;
    // An Azure DevOps remote is reached by the account of its organization; a credential for any
    // other host is a GitHub Enterprise account's, so the kind is the account's.
    let account = match PlatformKind::of_host(host) {
        Some(PlatformKind::AzureDevOps) => parse_remote(&remote.url)?.account(),
        _ => host.clone(),
    };
    credentials.credential(&account).await.map(|credential| FetchAuth {
        host: host.clone(),
        kind: PlatformKind::of_account(host),
        credential,
    })
}

/// How `remote` is fetched without the viewer asking: from the HTTPS URL of its repository with the
/// account's token, if it is on a platform host there is an account for. None otherwise, and the
/// credential of a host without an account is never asked for.
async fn platform_route(credentials: &(impl Credentials + Sync), remote: &RemoteInfo) -> Option<Via> {
    let key = parse_remote(&remote.url)?;
    let account = key.account();
    if !credentials.has_account(&account).await {
        return None;
    }
    let credential = credentials.credential(&account).await?;
    Some(Via::Platform {
        url: key.https_url(),
        auth: FetchAuth {
            kind: PlatformKind::of_account(&key.host),
            host: key.host,
            credential,
        },
    })
}

fn no_account(remote: &RemoteInfo) -> String {
    format!("{}: no account for the host of this remote", remote.name)
}

fn title(names: &[&str]) -> String {
    match names {
        [only] => format!("Fetching {only}"),
        many => format!("Fetching {} remotes", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;
    use crate::platform::{CredentialSource, Token};
    use crate::progress::{Task, TaskState};

    /// How a remote was reached.
    #[derive(PartialEq, Eq, Debug)]
    enum Seen {
        Platform { url: String, token: String },
        Configured(Option<String>),
    }

    impl Seen {
        fn of(via: Via) -> Self {
            let token = |auth: FetchAuth| auth.credential.token.expose().to_owned();
            match via {
                Via::Configured(auth) => Seen::Configured(auth.map(token)),
                Via::Platform { url, auth } => Seen::Platform {
                    url,
                    token: token(auth),
                },
            }
        }
    }

    /// The remotes fetched so far, each with how it was reached.
    type Fetched = Arc<StdMutex<Vec<(String, Seen)>>>;

    /// Remotes that fetch, except the ones named in `failing`.
    struct Remotes {
        remotes: Vec<RemoteInfo>,
        failing: Vec<&'static str>,
        /// How many refs each successful remote reports it updated.
        updated: usize,
        fetched: Fetched,
    }

    impl Fetcher for Remotes {
        async fn remotes(&self) -> Result<Vec<RemoteInfo>, GitError> {
            Ok(self.remotes.clone())
        }

        async fn fetch(&self, remote: &str, via: Via) -> Result<usize, GitError> {
            self.fetched.lock().unwrap().push((remote.to_owned(), Seen::of(via)));
            if self.failing.contains(&remote) {
                return Err(GitError::Remote("connection refused".into()));
            }
            Ok(self.updated)
        }

        async fn fetch_refs(&self, remote: &str, updates: Vec<RefUpdate>, via: Via) -> Result<(), GitError> {
            let wanted: Vec<_> = updates.iter().map(|u| u.source.as_str()).collect();
            self.fetched
                .lock()
                .unwrap()
                .push((format!("{remote} {}", wanted.join(" ")), Seen::of(via)));
            if self.failing.contains(&remote) {
                return Err(GitError::Remote("connection refused".into()));
            }
            Ok(())
        }

        async fn refs_below(&self, _: &str) -> Result<Vec<RefTip>, GitError> {
            unreachable!("nothing is deleted")
        }

        async fn descends_from(&self, _: &str, _: &str) -> Result<bool, GitError> {
            unreachable!("no history is read")
        }

        async fn delete_refs(&self, _: Vec<String>) -> Result<(), GitError> {
            unreachable!("nothing is deleted")
        }
    }

    fn remote(name: &str, url: &str) -> RemoteInfo {
        RemoteInfo {
            name: name.into(),
            url: url.into(),
            https_host: url
                .strip_prefix("https://")
                .and_then(|rest| rest.split('/').next())
                .map(str::to_owned),
        }
    }

    fn credential_for(host: &str) -> Credential {
        Credential {
            token: Token::new(&format!("token-for-{host}")).unwrap(),
            source: CredentialSource::Pat,
        }
    }

    /// A signed-in account on `github.com` only.
    struct GitHubOnly;

    impl Credentials for GitHubOnly {
        async fn credential(&self, host: &str) -> Option<Credential> {
            (host == "github.com").then(|| credential_for(host))
        }

        async fn has_account(&self, host: &str) -> bool {
            host == "github.com"
        }
    }

    /// Accounts on the listed hosts, counting how often a credential was asked for.
    struct Accounts {
        hosts: Vec<&'static str>,
        asked: AtomicUsize,
    }

    impl Accounts {
        fn on(hosts: Vec<&'static str>) -> Self {
            Self {
                hosts,
                asked: AtomicUsize::new(0),
            }
        }
    }

    impl Credentials for Accounts {
        async fn credential(&self, host: &str) -> Option<Credential> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.hosts.contains(&host).then(|| credential_for(host))
        }

        async fn has_account(&self, host: &str) -> bool {
            self.hosts.contains(&host)
        }
    }

    fn platform(url: &str, host: &str) -> Seen {
        Seen::Platform {
            url: url.into(),
            token: format!("token-for-{host}"),
        }
    }

    /// Four remotes: one on GitHub and three that no platform account reaches.
    fn mixed() -> Vec<RemoteInfo> {
        vec![
            remote("origin", "git@github.com:acme/app.git"),
            remote("mirror", "https://git.example.org/x.git"),
            remote("other", "git@gitlab.com:x/y.git"),
            remote("local", "/srv/repo"),
        ]
    }

    fn fetching(
        remotes: Vec<RemoteInfo>,
        failing: Vec<&'static str>,
    ) -> (Fetch<Remotes>, Arc<StdMutex<Vec<Task>>>, Fetched) {
        fetching_updating(remotes, failing, 1)
    }

    /// [`fetching`] with the remotes that fetch reporting `updated` refs each.
    fn fetching_updating(
        remotes: Vec<RemoteInfo>,
        failing: Vec<&'static str>,
        updated: usize,
    ) -> (Fetch<Remotes>, Arc<StdMutex<Vec<Task>>>, Fetched) {
        let heard = Arc::new(StdMutex::new(Vec::new()));
        let sink = heard.clone();
        let tasks = Tasks::new(Duration::ZERO, Box::new(move |t| sink.lock().unwrap().push(t)));
        let fetched = Fetched::default();
        let git = Remotes {
            remotes,
            failing,
            updated,
            fetched: fetched.clone(),
        };
        (Fetch::new(git, tasks), heard, fetched)
    }

    #[tokio::test]
    async fn a_remote_that_fails_fails_the_task_with_its_message_and_the_others_are_still_fetched() {
        let (fetch, heard, fetched) = fetching(
            vec![
                remote("origin", "https://example.com/x.git"),
                remote("backup", "https://example.com/y.git"),
            ],
            vec!["origin"],
        );

        let result = fetch.run(&GitHubOnly, Started::Viewer).await;

        assert_eq!(
            result,
            Ok(Ran {
                fetched: 1,
                updated: 1,
                failure: Some("origin: connection refused".into()),
                platform_remotes: 0,
                platform_failure: None,
            })
        );
        let names: Vec<_> = fetched.lock().unwrap().iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, ["origin", "backup"]);
        let last = heard.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state, TaskState::Failed("origin: connection refused".into()));
        assert_eq!(last.title, "Fetching 2 remotes");
    }

    #[tokio::test]
    async fn a_background_fetch_takes_platform_remotes_over_https_with_the_token_and_skips_the_others() {
        let (fetch, heard, fetched) = fetching(mixed(), vec![]);

        let result = fetch.run(&GitHubOnly, Started::Background).await;

        assert_eq!(
            result,
            Ok(Ran {
                fetched: 1,
                updated: 1,
                failure: None,
                platform_remotes: 1,
                platform_failure: None,
            })
        );
        assert_eq!(
            *fetched.lock().unwrap(),
            [(
                "origin".to_owned(),
                platform("https://github.com/acme/app.git", "github.com")
            )]
        );
        let last = heard.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.title, "Fetching origin");
        assert_eq!(last.state, TaskState::Finished);
    }

    #[tokio::test]
    async fn a_viewer_fetch_reaches_every_remote_and_takes_platform_ones_over_https() {
        let (fetch, _, fetched) = fetching(mixed(), vec![]);

        let result = fetch.run(&GitHubOnly, Started::Viewer).await;

        assert_eq!(
            result,
            Ok(Ran {
                fetched: 4,
                updated: 4,
                failure: None,
                platform_remotes: 1,
                platform_failure: None,
            })
        );
        assert_eq!(
            *fetched.lock().unwrap(),
            [
                (
                    "origin".to_owned(),
                    platform("https://github.com/acme/app.git", "github.com")
                ),
                ("mirror".to_owned(), Seen::Configured(None)),
                ("other".to_owned(), Seen::Configured(None)),
                ("local".to_owned(), Seen::Configured(None)),
            ]
        );
    }

    #[tokio::test]
    async fn a_background_fetch_with_no_platform_remote_starts_no_task() {
        let (fetch, heard, fetched) = fetching(
            vec![remote("other", "git@gitlab.com:x/y.git"), remote("local", "/srv/repo")],
            vec![],
        );

        let result = fetch.run(&GitHubOnly, Started::Background).await;

        assert_eq!(
            result,
            Ok(Ran {
                fetched: 0,
                updated: 0,
                failure: None,
                platform_remotes: 0,
                platform_failure: None,
            })
        );
        assert!(fetched.lock().unwrap().is_empty());
        assert!(heard.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_azure_devops_ssh_remote_is_fetched_with_its_organizations_account() {
        let (fetch, _, fetched) = fetching(
            vec![remote("origin", "git@ssh.dev.azure.com:v3/contoso/Shop/api")],
            vec![],
        );

        fetch
            .run(&Accounts::on(vec!["dev.azure.com/contoso"]), Started::Background)
            .await
            .unwrap();

        assert_eq!(
            *fetched.lock().unwrap(),
            [(
                "origin".to_owned(),
                platform("https://dev.azure.com/contoso/Shop/_git/api", "dev.azure.com/contoso")
            )]
        );
    }

    #[tokio::test]
    async fn a_host_without_an_account_is_never_asked_for_a_credential() {
        let (fetch, _, _) = fetching(
            vec![
                remote("other", "git@gitlab.com:x/y.git"),
                remote("mirror", "https://git.example.org/x/y.git"),
            ],
            vec![],
        );
        let accounts = Accounts::on(vec![]);

        fetch.run(&accounts, Started::Background).await.unwrap();

        assert_eq!(accounts.asked.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_viewer_run_tells_the_platform_failures_apart() {
        let (fetch, _, _) = fetching(
            vec![
                remote("origin", "https://github.com/acme/app.git"),
                remote("other", "git@gitlab.com:x/y.git"),
            ],
            vec!["origin", "other"],
        );

        let ran = fetch.run(&GitHubOnly, Started::Viewer).await.unwrap();

        assert_eq!(
            ran.failure.as_deref(),
            Some("origin: connection refused; other: connection refused")
        );
        assert_eq!(ran.platform_failure.as_deref(), Some("origin: connection refused"));
    }

    #[tokio::test]
    async fn fetching_refs_goes_over_https_with_the_token_and_fails_its_task_with_the_remotes_name() {
        let (fetch, heard, fetched) = fetching(vec![], vec!["origin"]);
        let updates = vec![RefUpdate {
            source: "refs/pull/7/head".into(),
            destination: "refs/vet/pr/7/head".into(),
        }];

        let upstream = remote("upstream", "git@github.com:acme/app.git");
        let done = fetch
            .refs(&GitHubOnly, &upstream, updates.clone(), "Fetching #7".into())
            .await;
        let refused = fetch
            .refs(
                &GitHubOnly,
                &remote("origin", "https://github.com/acme/lib.git"),
                updates,
                "Fetching #7".into(),
            )
            .await;

        assert_eq!(done, Ok(()));
        assert_eq!(refused, Err(GitError::Remote("origin: connection refused".into())));
        assert_eq!(
            *fetched.lock().unwrap(),
            [
                (
                    "upstream refs/pull/7/head".to_owned(),
                    platform("https://github.com/acme/app.git", "github.com")
                ),
                (
                    "origin refs/pull/7/head".to_owned(),
                    platform("https://github.com/acme/lib.git", "github.com")
                ),
            ]
        );
        let states: Vec<_> = heard.lock().unwrap().iter().map(|t| t.state.clone()).collect();
        assert_eq!(
            states,
            [
                TaskState::Running,
                TaskState::Finished,
                TaskState::Running,
                TaskState::Failed("origin: connection refused".into())
            ]
        );
    }

    #[tokio::test]
    async fn fetching_refs_without_an_account_fails_its_task_and_asks_no_remote() {
        let (fetch, heard, fetched) = fetching(vec![], vec![]);
        let updates = vec![RefUpdate {
            source: "refs/pull/7/head".into(),
            destination: "refs/vet/pr/7/head".into(),
        }];

        let other = remote("other", "git@gitlab.com:x/y.git");
        let result = fetch
            .refs(&GitHubOnly, &other, updates.clone(), "Fetching #7".into())
            .await;
        let asking = fetch.refs_asking(&GitHubOnly, &other, updates).await;

        let message = "other: no account for the host of this remote";
        assert_eq!(result, Err(GitError::Remote(message.into())));
        assert_eq!(asking, Err(GitError::Remote(message.into())));
        assert!(fetched.lock().unwrap().is_empty());
        let last = heard.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state, TaskState::Failed(message.into()));
    }

    #[tokio::test]
    async fn a_repository_without_remotes_has_nothing_to_report() {
        let (fetch, heard, _) = fetching(vec![], vec![]);

        assert_eq!(
            fetch.run(&GitHubOnly, Started::Viewer).await,
            Ok(Ran {
                fetched: 0,
                updated: 0,
                failure: None,
                platform_remotes: 0,
                platform_failure: None,
            })
        );
        assert!(heard.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn what_the_remotes_updated_is_reported_apart_from_how_many_were_reached() {
        let (quiet, _, _) = fetching_updating(
            vec![
                remote("origin", "https://example.com/x.git"),
                remote("backup", "https://example.com/y.git"),
            ],
            vec![],
            0,
        );
        let (busy, _, _) = fetching_updating(vec![remote("origin", "https://example.com/x.git")], vec![], 4);

        assert_eq!(
            quiet.run(&GitHubOnly, Started::Viewer).await,
            Ok(Ran {
                fetched: 2,
                updated: 0,
                failure: None,
                platform_remotes: 0,
                platform_failure: None,
            }),
            "both remotes answered, neither moved a ref"
        );
        assert_eq!(
            busy.run(&GitHubOnly, Started::Viewer).await,
            Ok(Ran {
                fetched: 1,
                updated: 4,
                failure: None,
                platform_remotes: 0,
                platform_failure: None,
            }),
            "the reference count, not the remote count"
        );
    }
}
