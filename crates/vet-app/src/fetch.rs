//! Keeping remote-tracking refs current (SPEC M1 "Git fetch"): every remote of the
//! repository is fetched, as one background task, on demand and on a timer. A fetch
//! that cannot be done (offline, refused) ends its task as failed and is otherwise
//! harmless: the review does not wait for it or depend on it.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::mutex::lock;
use crate::platform::{Credential, PlatformKind, parse_remote};
use crate::ports::{FetchAuth, Fetcher, GitError, RefTip, RefUpdate, RemoteInfo};
use crate::progress::{TaskKind, Tasks};

/// How often a project's remotes are fetched while it is open.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Where a fetch finds the platform credential for the host of an HTTPS remote.
pub trait Credentials {
    fn credential(&self, host: &str) -> impl Future<Output = Option<Credential>> + Send;
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
        self.note_fetching(&updates);
        let task = self.tasks.start(TaskKind::Fetch, title);
        let auth = authentication(credentials, remote).await;
        match self.git.fetch_refs(&remote.name, updates, auth).await {
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
        self.note_fetching(&updates);
        let auth = authentication(credentials, remote).await;
        self.git
            .fetch_refs(&remote.name, updates, auth)
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

    /// Fetches every remote, one after the other, as one task; a remote that fails does not
    /// stop the others, and what they brought counts. Fails only if the remotes cannot be read.
    pub async fn run(&self, credentials: &(impl Credentials + Sync)) -> Result<Ran, GitError> {
        let _only_one = self.running.lock().await;
        let remotes = self.git.remotes().await?;
        if remotes.is_empty() {
            return Ok(Ran {
                fetched: 0,
                updated: 0,
                failure: None,
            });
        }
        let task = self.tasks.start(TaskKind::Fetch, title(&remotes));
        let total = remotes.len() as u32;
        let mut failures = Vec::new();
        let mut updated = 0;
        for (done, remote) in remotes.iter().enumerate() {
            task.report(Some((done as u32, total)), Some(remote.name.clone()));
            let auth = authentication(credentials, remote).await;
            match self.git.fetch(&remote.name, auth).await {
                Ok(refs) => updated += refs,
                Err(e) => {
                    tracing::warn!(remote = %remote.name, error = %e, "fetch failed");
                    failures.push(format!("{}: {e}", remote.name));
                }
            }
        }
        let fetched = remotes.len() - failures.len();
        if failures.is_empty() {
            task.finish();
            return Ok(Ran {
                fetched,
                updated,
                failure: None,
            });
        }
        let message = failures.join("; ");
        task.fail(message.clone());
        Ok(Ran {
            fetched,
            updated,
            failure: Some(message),
        })
    }
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

fn title(remotes: &[RemoteInfo]) -> String {
    match remotes {
        [only] => format!("Fetching {}", only.name),
        many => format!("Fetching {} remotes", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;
    use crate::platform::{CredentialSource, Token};
    use crate::progress::{Task, TaskState};

    /// The remotes fetched so far, each with the token it was fetched with.
    type Fetched = Arc<StdMutex<Vec<(String, Option<String>)>>>;

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

        async fn fetch(&self, remote: &str, auth: Option<FetchAuth>) -> Result<usize, GitError> {
            let token = auth.map(|a| a.credential.token.expose().to_owned());
            self.fetched.lock().unwrap().push((remote.to_owned(), token));
            if self.failing.contains(&remote) {
                return Err(GitError::Remote("connection refused".into()));
            }
            Ok(self.updated)
        }

        async fn fetch_refs(
            &self,
            remote: &str,
            updates: Vec<RefUpdate>,
            auth: Option<FetchAuth>,
        ) -> Result<(), GitError> {
            let token = auth.map(|a| a.credential.token.expose().to_owned());
            let wanted: Vec<_> = updates.iter().map(|u| u.source.as_str()).collect();
            self.fetched
                .lock()
                .unwrap()
                .push((format!("{remote} {}", wanted.join(" ")), token));
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

    fn remote(name: &str, https_host: Option<&str>) -> RemoteInfo {
        RemoteInfo {
            name: name.into(),
            url: format!("https://{}/x.git", https_host.unwrap_or("example.com")),
            https_host: https_host.map(str::to_owned),
        }
    }

    /// A signed-in account on `github.com` only.
    struct GitHubOnly;

    impl Credentials for GitHubOnly {
        async fn credential(&self, host: &str) -> Option<Credential> {
            (host == "github.com").then(|| Credential {
                token: Token::new("token-for-github.com").unwrap(),
                source: CredentialSource::Pat,
            })
        }
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
        let (fetch, heard, fetched) = fetching(vec![remote("origin", None), remote("backup", None)], vec!["origin"]);

        let result = fetch.run(&GitHubOnly).await;

        assert_eq!(
            result,
            Ok(Ran {
                fetched: 1,
                updated: 1,
                failure: Some("origin: connection refused".into())
            })
        );
        let names: Vec<_> = fetched.lock().unwrap().iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, ["origin", "backup"]);
        let last = heard.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state, TaskState::Failed("origin: connection refused".into()));
        assert_eq!(last.title, "Fetching 2 remotes");
    }

    #[tokio::test]
    async fn the_platform_credential_goes_to_https_remotes_by_host_and_to_nobody_else() {
        let (fetch, heard, fetched) = fetching(
            vec![
                remote("origin", Some("github.com")),
                remote("mirror", Some("git.example.org")),
                remote("ssh", None),
            ],
            vec![],
        );

        let result = fetch.run(&GitHubOnly).await;

        assert_eq!(
            result,
            Ok(Ran {
                fetched: 3,
                updated: 3,
                failure: None
            })
        );
        assert_eq!(
            *fetched.lock().unwrap(),
            [
                ("origin".to_owned(), Some("token-for-github.com".to_owned())),
                ("mirror".to_owned(), None),
                ("ssh".to_owned(), None),
            ]
        );
        assert_eq!(heard.lock().unwrap().last().unwrap().state, TaskState::Finished);
    }

    #[tokio::test]
    async fn fetching_refs_uses_the_hosts_credential_and_is_a_task_that_fails_with_the_remotes_name() {
        let (fetch, heard, fetched) = fetching(vec![], vec!["origin"]);
        let updates = vec![RefUpdate {
            source: "refs/pull/7/head".into(),
            destination: "refs/vet/pr/7/head".into(),
        }];

        let on_github = remote("upstream", Some("github.com"));
        let done = fetch
            .refs(&GitHubOnly, &on_github, updates.clone(), "Fetching #7".into())
            .await;
        let refused = fetch
            .refs(
                &GitHubOnly,
                &remote("origin", Some("git.example.org")),
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
                    Some("token-for-github.com".to_owned())
                ),
                ("origin refs/pull/7/head".to_owned(), None),
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
    async fn a_repository_without_remotes_has_nothing_to_report() {
        let (fetch, heard, _) = fetching(vec![], vec![]);

        assert_eq!(
            fetch.run(&GitHubOnly).await,
            Ok(Ran {
                fetched: 0,
                updated: 0,
                failure: None
            })
        );
        assert!(heard.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn what_the_remotes_updated_is_reported_apart_from_how_many_were_reached() {
        let (quiet, _, _) = fetching_updating(vec![remote("origin", None), remote("backup", None)], vec![], 0);
        let (busy, _, _) = fetching_updating(vec![remote("origin", None)], vec![], 4);

        assert_eq!(
            quiet.run(&GitHubOnly).await,
            Ok(Ran {
                fetched: 2,
                updated: 0,
                failure: None
            }),
            "both remotes answered, neither moved a ref"
        );
        assert_eq!(
            busy.run(&GitHubOnly).await,
            Ok(Ran {
                fetched: 1,
                updated: 4,
                failure: None
            }),
            "the reference count, not the remote count"
        );
    }
}
