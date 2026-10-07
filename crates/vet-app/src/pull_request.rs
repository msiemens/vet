//! Pull requests as changesets (design §4.1): where git keeps what a pull request is made of,
//! which of a project's remotes are hosted repositories, the pull requests the project's
//! repositories list, and bringing one in to review.

use crate::fetch::{Credentials, Fetch};
use crate::platform::{
    Platform, PlatformError, PlatformKind, PullRequestQuery, PullRequestRow, QueueTab, RemoteKey, list_rows,
    parse_remote,
};
use crate::ports::{Fetcher, RefUpdate, RemoteInfo};

/// The refs of the repository that hold a pull request, kept apart from every branch and
/// remote-tracking ref of the user's.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestRefs {
    /// The pull request's head: the commit the review is of.
    pub head: String,
    /// The tip of the branch the pull request is merged into, which its base is measured from.
    pub target: String,
}

impl PullRequestRefs {
    pub fn of(remote: &RemoteKey, number: u64) -> Self {
        // A ref name cannot hold what a host with a port or a repository name may.
        let key: String = remote
            .to_string()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '/') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let prefix = format!("refs/vet/pr/{key}/{number}");
        Self {
            head: format!("{prefix}/head"),
            target: format!("{prefix}/target"),
        }
    }

    /// What to fetch from the platform's repository to have both refs: the pull request's head
    /// (the platform keeps it there for forks too) and its target branch.
    pub fn updates(&self, number: u64, target_branch: &str) -> Vec<RefUpdate> {
        vec![
            RefUpdate {
                source: format!("refs/pull/{number}/head"),
                destination: self.head.clone(),
            },
            RefUpdate {
                source: format!("refs/heads/{target_branch}"),
                destination: self.target.clone(),
            },
        ]
    }
}

/// A repository of a platform that one of the project's git remotes is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HostedRepository {
    /// The git remote to fetch it from.
    pub remote: String,
    pub key: RemoteKey,
}

/// The repositories on `kind` that `remotes` name, each once, in the order of the remotes (a
/// second remote for the same repository is the same repository).
pub fn hosted_repositories(remotes: &[RemoteInfo], kind: PlatformKind) -> Vec<HostedRepository> {
    let mut found: Vec<HostedRepository> = Vec::new();
    for remote in remotes {
        let Some(key) = parse_remote(&remote.url).filter(|k| k.kind() == kind) else {
            continue;
        };
        if !found.iter().any(|h| h.key == key) {
            found.push(HostedRepository {
                remote: remote.name.clone(),
                key,
            });
        }
    }
    found
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListedPullRequest {
    pub remote: RemoteKey,
    pub row: PullRequestRow,
}

/// How asking one repository went.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RepositoryStatus {
    pub remote: RemoteKey,
    /// Why the platform could not list the repository's pull requests.
    pub error: Option<PlatformError>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestList {
    /// Most recently updated first, across the repositories.
    pub rows: Vec<ListedPullRequest>,
    /// One per repository asked, in the order given.
    pub sources: Vec<RepositoryStatus>,
}

/// The open pull requests of `repositories` that wait for the viewer or are the viewer's own. A
/// repository the platform cannot answer for is reported in `sources`; the others still list.
pub async fn list_pull_requests<P: Platform>(platform: &P, repositories: &[RemoteKey]) -> PullRequestList {
    let mut rows = Vec::new();
    let mut sources = Vec::new();
    for remote in repositories {
        let mut error = None;
        let mut listed: Vec<PullRequestRow> = Vec::new();
        for tab in [QueueTab::WaitingForMe, QueueTab::Mine] {
            match list_rows(platform, remote, &PullRequestQuery::new(tab)).await {
                Ok(found) => {
                    for row in found {
                        if !listed.iter().any(|r| r.number == row.number) {
                            listed.push(row);
                        }
                    }
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        if error.is_none() {
            rows.extend(listed.into_iter().map(|row| ListedPullRequest {
                remote: remote.clone(),
                row,
            }));
        }
        sources.push(RepositoryStatus {
            remote: remote.clone(),
            error,
        });
    }
    rows.sort_by_key(|listed| std::cmp::Reverse((listed.row.updated_at, listed.row.number)));
    PullRequestList { rows, sources }
}

/// Brings in the head of pull request `number` of `remote` and the branch it targets, so a
/// changeset can be made of them. The error is fit to show.
pub async fn fetch_pull_request<P: Platform, G: Fetcher>(
    platform: &P,
    fetch: &Fetch<G>,
    credentials: &(impl Credentials + Sync),
    remote: &RemoteKey,
    number: u64,
) -> Result<(), String> {
    let detail = platform.pull_request(remote, number).await.map_err(|e| e.to_string())?;
    let remotes = fetch.remotes().await?;
    let hosted = hosted_repositories(&remotes, remote.kind());
    let git_remote = hosted
        .iter()
        .find(|h| h.key == *remote)
        .and_then(|h| remotes.iter().find(|r| r.name == h.remote))
        .ok_or_else(|| format!("none of the remotes of this repository is {remote}"))?;
    let updates = PullRequestRefs::of(remote, number).updates(number, &detail.pull_request.target_branch);
    fetch
        .refs(credentials, git_remote, updates, format!("Fetching #{number}"))
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::platform::{Credential, PullRequest, PullRequestDetail, ReviewRequest, Viewer, pr, viewer};
    use crate::ports::{FetchAuth, GitError};
    use crate::progress::Tasks;

    fn github(path: &str) -> RemoteKey {
        RemoteKey {
            host: "github.com".to_owned(),
            path: path.to_owned(),
        }
    }

    fn info(name: &str, url: &str) -> RemoteInfo {
        RemoteInfo {
            name: name.to_owned(),
            url: url.to_owned(),
            https_host: None,
        }
    }

    #[test]
    fn the_refs_of_two_repositories_and_two_pull_requests_never_meet() {
        let a = PullRequestRefs::of(&github("acme/app"), 7);
        assert_eq!(a.head, "refs/vet/pr/github_com/acme/app/7/head");
        assert_eq!(a.target, "refs/vet/pr/github_com/acme/app/7/target");
        let enterprise = RemoteKey {
            host: "ghe.corp.example:8443".to_owned(),
            path: "acme/app".to_owned(),
        };
        let others = [
            PullRequestRefs::of(&github("acme/app"), 8),
            PullRequestRefs::of(&github("acme/other"), 7),
            PullRequestRefs::of(&enterprise, 7),
        ];
        for other in others {
            assert_ne!(other.head, a.head);
            assert_ne!(other.target, a.target);
        }
    }

    #[test]
    fn a_pull_request_is_fetched_from_the_platforms_pull_ref_and_the_target_branch() {
        let refs = PullRequestRefs::of(&github("acme/app"), 7);
        let updates = refs.updates(7, "release/1.x");
        assert_eq!(
            updates
                .iter()
                .map(|u| (u.source.as_str(), u.destination.as_str()))
                .collect::<Vec<_>>(),
            [
                ("refs/pull/7/head", refs.head.as_str()),
                ("refs/heads/release/1.x", refs.target.as_str())
            ]
        );
    }

    #[test]
    fn a_repository_reached_by_several_remotes_and_urls_is_listed_once_under_the_first() {
        let remotes = [
            info("origin", "https://github.com/acme/app.git"),
            info("upstream", "git@github.com:acme/app"),
            info("fork", "https://github.com/ada/app"),
            info("local", "/srv/git/app"),
            info("work", "https://dev.azure.com/contoso/Shop/_git/api"),
        ];
        let found = hosted_repositories(&remotes, PlatformKind::GitHub);
        let names: Vec<_> = found.iter().map(|h| (h.remote.as_str(), h.key.path.as_str())).collect();
        assert_eq!(names, [("origin", "acme/app"), ("fork", "ada/app")]);
    }

    /// Lists a fixed set of pull requests per repository, or fails for some.
    struct Hosting {
        prs: Vec<(RemoteKey, Vec<PullRequest>)>,
        failing: Vec<RemoteKey>,
    }

    impl Platform for Hosting {
        fn traits(&self) -> crate::platform::PlatformTraits {
            crate::platform::PlatformTraits::github()
        }

        async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
            Ok(viewer())
        }

        async fn list_pull_requests(
            &self,
            remote: &RemoteKey,
            _: &PullRequestQuery,
        ) -> Result<Vec<PullRequest>, PlatformError> {
            if self.failing.contains(remote) {
                return Err(PlatformError::Rejected("github.com refused the token".to_owned()));
            }
            Ok(self
                .prs
                .iter()
                .find(|(r, _)| r == remote)
                .map(|(_, prs)| prs.clone())
                .unwrap_or_default())
        }

        async fn pull_request(&self, _: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
            let mut pull_request = pr(number);
            pull_request.target_branch = "release".to_owned();
            Ok(PullRequestDetail {
                pull_request,
                base: "b0".to_owned(),
                pushes: Vec::new(),
            })
        }
    }

    fn waiting(number: u64, updated_at: i64) -> PullRequest {
        let mut p = pr(number);
        p.review_requests = vec![ReviewRequest::User("ada".to_owned())];
        p.updated_at = updated_at;
        p
    }

    fn mine(number: u64, updated_at: i64) -> PullRequest {
        let mut p = pr(number);
        p.author = "ada".to_owned();
        p.updated_at = updated_at;
        p
    }

    /// The git remote and the refs of each fetch.
    type Fetched = Arc<Mutex<Vec<(String, Vec<RefUpdate>)>>>;

    /// Git remotes that record what they were asked to fetch.
    struct Remotes {
        remotes: Vec<RemoteInfo>,
        fetched: Fetched,
    }

    impl Fetcher for Remotes {
        async fn remotes(&self) -> Result<Vec<RemoteInfo>, GitError> {
            Ok(self.remotes.clone())
        }

        async fn fetch(&self, _: &str, _: Option<FetchAuth>) -> Result<usize, GitError> {
            unreachable!("only refs are fetched")
        }

        async fn fetch_refs(
            &self,
            remote: &str,
            updates: Vec<RefUpdate>,
            _: Option<FetchAuth>,
        ) -> Result<(), GitError> {
            self.fetched.lock().unwrap().push((remote.to_owned(), updates));
            Ok(())
        }
    }

    struct Anonymous;

    impl Credentials for Anonymous {
        async fn credential(&self, _: &str) -> Option<Credential> {
            None
        }
    }

    fn fetching(remotes: Vec<RemoteInfo>) -> (Fetch<Remotes>, Fetched) {
        let fetched = Arc::default();
        let git = Remotes {
            remotes,
            fetched: Arc::clone(&fetched),
        };
        (Fetch::new(git, Tasks::default()), fetched)
    }

    #[tokio::test]
    async fn a_pull_request_is_fetched_from_the_remote_that_is_its_repository_with_the_target_the_platform_names() {
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
        };
        let (fetch, fetched) = fetching(vec![
            info("fork", "https://github.com/ada/app"),
            info("upstream", "git@github.com:acme/app.git"),
        ]);

        let app = github("acme/app");
        fetch_pull_request(&platform, &fetch, &Anonymous, &app, 7)
            .await
            .unwrap();

        let refs = PullRequestRefs::of(&app, 7);
        assert_eq!(
            *fetched.lock().unwrap(),
            [("upstream".to_owned(), refs.updates(7, "release"))]
        );
    }

    #[tokio::test]
    async fn a_repository_that_is_none_of_the_remotes_is_refused_and_nothing_is_fetched() {
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
        };
        let (fetch, fetched) = fetching(vec![info("origin", "https://github.com/ada/app")]);

        let error = fetch_pull_request(&platform, &fetch, &Anonymous, &github("acme/app"), 7)
            .await
            .unwrap_err();

        assert_eq!(error, "none of the remotes of this repository is github.com/acme/app");
        assert!(fetched.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_list_has_what_waits_for_the_viewer_and_what_is_theirs_newest_first() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![(app.clone(), vec![waiting(1, 100), mine(2, 300), pr(3), mine(4, 200)])],
            failing: vec![],
        };

        let list = list_pull_requests(&platform, std::slice::from_ref(&app)).await;

        let shown: Vec<_> = list.rows.iter().map(|r| (r.row.number, r.row.waiting_for_me)).collect();
        assert_eq!(shown, [(2, false), (4, false), (1, true)], "pr 3 is neither");
        assert_eq!(
            list.sources,
            [RepositoryStatus {
                remote: app,
                error: None
            }]
        );
    }

    #[tokio::test]
    async fn a_repository_that_cannot_be_asked_is_reported_and_the_others_still_list() {
        let (app, lib) = (github("acme/app"), github("acme/lib"));
        let platform = Hosting {
            prs: vec![(lib.clone(), vec![mine(9, 50)])],
            failing: vec![app.clone()],
        };

        let list = list_pull_requests(&platform, &[app.clone(), lib.clone()]).await;

        assert_eq!(
            list.rows.iter().map(|r| (&r.remote, r.row.number)).collect::<Vec<_>>(),
            [(&lib, 9)]
        );
        assert_eq!(
            list.sources,
            [
                RepositoryStatus {
                    remote: app,
                    error: Some(PlatformError::Rejected("github.com refused the token".to_owned()))
                },
                RepositoryStatus {
                    remote: lib,
                    error: None
                },
            ]
        );
    }
}
