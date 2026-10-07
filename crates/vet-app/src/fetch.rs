//! Keeping remote-tracking refs current (SPEC M1 "Git fetch"): every remote of the
//! repository is fetched, as one background task, on demand and on a timer. A fetch
//! that cannot be done (offline, refused) ends its task as failed and is otherwise
//! harmless: the review does not wait for it or depend on it.

use std::time::Duration;

use tokio::sync::Mutex;

use crate::platform::{Credential, PlatformKind};
use crate::ports::{FetchAuth, Fetcher, RemoteInfo};
use crate::progress::{TaskKind, Tasks};

/// How often a project's remotes are fetched while it is open.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub struct Fetch<G> {
    git: G,
    tasks: Tasks,
    /// A fetch started while another runs waits for it: both write the same refs.
    running: Mutex<()>,
}

impl<G: Fetcher> Fetch<G> {
    pub fn new(git: G, tasks: Tasks) -> Self {
        Self {
            git,
            tasks,
            running: Mutex::new(()),
        }
    }

    /// Fetches every remote, one after the other, as one task; a remote that fails does not
    /// stop the others. `credential` yields the platform credential for an HTTPS host.
    /// Answers with the message the task failed with, if it did.
    pub async fn run(&self, credential: impl AsyncFn(&str) -> Option<Credential>) -> Result<(), String> {
        let _only_one = self.running.lock().await;
        let remotes = self.git.remotes().await.map_err(|e| e.0)?;
        if remotes.is_empty() {
            return Ok(());
        }
        let task = self.tasks.start(TaskKind::Fetch, title(&remotes));
        let total = remotes.len() as u32;
        let mut failures = Vec::new();
        for (done, remote) in remotes.iter().enumerate() {
            task.report(Some((done as u32, total)), Some(remote.name.clone()));
            let auth = match &remote.https_host {
                Some(host) => credential(host).await.map(|credential| FetchAuth {
                    host: host.clone(),
                    kind: PlatformKind::of_host(host),
                    credential,
                }),
                None => None,
            };
            if let Err(e) = self.git.fetch(&remote.name, auth).await {
                failures.push(format!("{}: {}", remote.name, e.0));
            }
        }
        if failures.is_empty() {
            task.finish();
            Ok(())
        } else {
            let message = failures.join("; ");
            task.fail(message.clone());
            Err(message)
        }
    }
}

fn title(remotes: &[RemoteInfo]) -> String {
    match remotes {
        [only] => format!("Fetching {}", only.name),
        many => format!("Fetching {} remotes", many.len()),
    }
}

/// Calls `tick` now and then every `interval` after the previous call ended, for ever.
pub async fn every(interval: Duration, mut tick: impl AsyncFnMut()) {
    loop {
        tick().await;
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use super::*;
    use crate::platform::{CredentialSource, Token};
    use crate::ports::GitError;
    use crate::progress::{Task, TaskState};

    /// The remotes fetched so far, each with the token it was fetched with.
    type Fetched = Arc<StdMutex<Vec<(String, Option<String>)>>>;

    /// Remotes that fetch, except the ones named in `failing`.
    struct Remotes {
        remotes: Vec<RemoteInfo>,
        failing: Vec<&'static str>,
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
                return Err(GitError("connection refused".into()));
            }
            Ok(1)
        }
    }

    fn remote(name: &str, https_host: Option<&str>) -> RemoteInfo {
        RemoteInfo {
            name: name.into(),
            url: format!("https://{}/x.git", https_host.unwrap_or("example.com")),
            https_host: https_host.map(str::to_owned),
        }
    }

    fn token(host: &str) -> Option<Credential> {
        Some(Credential {
            token: Token::new(&format!("token-for-{host}")).unwrap(),
            source: CredentialSource::Pat,
        })
    }

    fn fetching(
        remotes: Vec<RemoteInfo>,
        failing: Vec<&'static str>,
    ) -> (Fetch<Remotes>, Arc<StdMutex<Vec<Task>>>, Fetched) {
        let heard = Arc::new(StdMutex::new(Vec::new()));
        let sink = heard.clone();
        let tasks = Tasks::new(Duration::ZERO, Box::new(move |t| sink.lock().unwrap().push(t)));
        let fetched = Fetched::default();
        let git = Remotes {
            remotes,
            failing,
            fetched: fetched.clone(),
        };
        (Fetch::new(git, tasks), heard, fetched)
    }

    #[tokio::test]
    async fn a_remote_that_fails_fails_the_task_with_its_message_and_the_others_are_still_fetched() {
        let (fetch, heard, fetched) = fetching(vec![remote("origin", None), remote("backup", None)], vec!["origin"]);

        let result = fetch.run(async |_| None).await;

        assert_eq!(result, Err("origin: connection refused".into()));
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

        let result = fetch
            .run(async |host| if host == "github.com" { token(host) } else { None })
            .await;

        assert_eq!(result, Ok(()));
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
    async fn a_repository_without_remotes_has_nothing_to_report() {
        let (fetch, heard, _) = fetching(vec![], vec![]);

        assert_eq!(fetch.run(async |_| None).await, Ok(()));
        assert!(heard.lock().unwrap().is_empty());
    }
}
