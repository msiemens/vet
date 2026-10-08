//! Pull requests as changesets (design §4.1): where git keeps what a pull request is made of,
//! which of a project's remotes are hosted repositories, the pull requests the project's
//! repositories list, and bringing one in to review.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use crate::fetch::{Credentials, Fetch};
use crate::platform::{
    Platform, PlatformError, PlatformKind, PullRequest, PullRequestQuery, PullRequestRow, QueueTab, RemoteKey,
    list_rows, parse_remote, rows,
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

    /// What to fetch from the platform's repository to have both refs: the pull request's head and
    /// its target branch.
    ///
    /// GitHub keeps the head at `refs/pull/<n>/head`, for forks too. Azure DevOps has no such ref
    /// (its `refs/pull/<n>/merge` is a merge commit): the source branch is fetched, and the head
    /// commit itself when the branch is in a fork's repository.
    pub fn updates(&self, kind: PlatformKind, pull_request: &PullRequest) -> Vec<RefUpdate> {
        let head = match kind {
            PlatformKind::GitHub => format!("refs/pull/{}/head", pull_request.number),
            PlatformKind::AzureDevOps if pull_request.fork.is_some() => pull_request.head.clone(),
            PlatformKind::AzureDevOps => format!("refs/heads/{}", pull_request.source_branch),
        };
        vec![
            RefUpdate {
                source: head,
                destination: self.head.clone(),
            },
            RefUpdate {
                source: format!("refs/heads/{}", pull_request.target_branch),
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

/// The hosts `repositories` are on that are not github.com: the ones whose account decides whether
/// a repository there is GitHub's (a host that is neither GitHub nor Azure DevOps has none).
pub fn hosts_to_ask_about(repositories: &[HostedRepository]) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for host in repositories.iter().map(|h| &h.key.host).filter(|h| *h != GITHUB_COM) {
        if !hosts.contains(host) {
            hosts.push(host.clone());
        }
    }
    hosts
}

/// The repositories of `candidates` (those [`hosted_repositories`] answers for GitHub) that are
/// on GitHub: on github.com, or on a host with an account, which is how a GitHub Enterprise host
/// is told from any other git host.
pub fn on_github(candidates: Vec<HostedRepository>, hosts_with_account: &[String]) -> Vec<HostedRepository> {
    candidates
        .into_iter()
        .filter(|h| h.key.host == GITHUB_COM || hosts_with_account.contains(&h.key.host))
        .collect()
}

const GITHUB_COM: &str = "github.com";

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

impl PullRequestList {
    /// This list and `other`, as one list: the rows newest first, the sources of this one first.
    pub fn and(mut self, other: Self) -> Self {
        self.rows.extend(other.rows);
        self.sources.extend(other.sources);
        sort_newest_first(&mut self.rows);
        self
    }
}

fn sort_newest_first(rows: &mut [ListedPullRequest]) {
    rows.sort_by_key(|listed| std::cmp::Reverse((listed.row.updated_at, listed.row.number)));
}

/// How the platform last named a pull request: what a view of it is headed with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestHead {
    pub title: String,
    pub draft: bool,
}

/// The titles of the pull requests this process has heard of from the platform, so a view of
/// one can be headed without asking again.
#[derive(Default)]
pub struct KnownPullRequests(Mutex<HashMap<(RemoteKey, u64), PullRequestHead>>);

impl KnownPullRequests {
    fn learn(&self, remote: &RemoteKey, number: u64, title: &str, draft: bool) {
        let head = PullRequestHead {
            title: title.to_owned(),
            draft,
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((remote.clone(), number), head);
    }

    pub fn get(&self, remote: &RemoteKey, number: u64) -> Option<PullRequestHead> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(remote.clone(), number))
            .cloned()
    }
}

/// The open pull requests of `repositories` that wait for the viewer or are the viewer's own, and
/// `on_view` (a repository and a number) when it is one of them, whoever's it is. A repository the
/// platform cannot answer for is reported in `sources`; the others still list.
pub async fn list_pull_requests<P: Platform>(
    platform: &P,
    repositories: &[RemoteKey],
    on_view: Option<(&RemoteKey, u64)>,
    known: &KnownPullRequests,
) -> PullRequestList {
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
            if let Some((_, number)) =
                on_view.filter(|(r, n)| *r == remote && !listed.iter().any(|row| row.number == *n))
                && let Some(row) = row_of(platform, remote, number).await
            {
                listed.push(row);
            }
            for row in &listed {
                known.learn(remote, row.number, &row.title, row.draft);
            }
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
    sort_newest_first(&mut rows);
    PullRequestList { rows, sources }
}

/// The row of one pull request, as the queue would list it; `None` if the platform cannot say.
async fn row_of<P: Platform>(platform: &P, remote: &RemoteKey, number: u64) -> Option<PullRequestRow> {
    let viewer = platform.viewer(remote).await.ok()?;
    let detail = platform.pull_request(remote, number).await.ok()?;
    rows(
        vec![detail.pull_request],
        &viewer,
        &PullRequestQuery::new(QueueTab::AllOpen),
    )
    .pop()
}

/// Why a pull request could not be brought in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FetchError {
    /// The platform could not say what the pull request is.
    Platform(PlatformError),
    /// Git could not fetch it, or no remote is its repository. Fit to show.
    Git(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Platform(e) => e.fmt(f),
            Self::Git(message) => f.write_str(message),
        }
    }
}

/// Brings in the head of pull request `number` of `remote` and the branch it targets, so a
/// changeset can be made of them.
pub async fn fetch_pull_request<P: Platform, G: Fetcher>(
    platform: &P,
    fetch: &Fetch<G>,
    credentials: &(impl Credentials + Sync),
    known: &KnownPullRequests,
    remote: &RemoteKey,
    number: u64,
) -> Result<(), FetchError> {
    let detail = platform
        .pull_request(remote, number)
        .await
        .map_err(FetchError::Platform)?;
    known.learn(remote, number, &detail.pull_request.title, detail.pull_request.draft);
    let remotes = fetch.remotes().await.map_err(FetchError::Git)?;
    let hosted = hosted_repositories(&remotes, remote.kind());
    let git_remote = hosted
        .iter()
        .find(|h| h.key == *remote)
        .and_then(|h| remotes.iter().find(|r| r.name == h.remote))
        .ok_or_else(|| FetchError::Git(format!("none of the remotes of this repository is {remote}")))?;
    let updates = PullRequestRefs::of(remote, number).updates(remote.kind(), &detail.pull_request);
    fetch
        .refs(credentials, git_remote, updates, format!("Fetching #{number}"))
        .await
        .map_err(FetchError::Git)
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

    fn sources(refs: &PullRequestRefs, kind: PlatformKind, pull_request: &PullRequest) -> Vec<(String, String)> {
        refs.updates(kind, pull_request)
            .into_iter()
            .map(|u| (u.source, u.destination))
            .collect()
    }

    #[test]
    fn a_pull_request_is_fetched_from_where_its_platform_keeps_the_head_and_from_the_target_branch() {
        let refs = PullRequestRefs::of(&github("acme/app"), 7);
        let mut pull_request = pr(7);
        pull_request.target_branch = "release/1.x".to_owned();
        assert_eq!(
            sources(&refs, PlatformKind::GitHub, &pull_request),
            [
                ("refs/pull/7/head".to_owned(), refs.head.clone()),
                ("refs/heads/release/1.x".to_owned(), refs.target.clone())
            ]
        );
        assert_eq!(
            sources(&refs, PlatformKind::AzureDevOps, &pull_request),
            [
                ("refs/heads/feature".to_owned(), refs.head.clone()),
                ("refs/heads/release/1.x".to_owned(), refs.target.clone())
            ]
        );
        // The source branch of a fork is not in this repository; its commit is reachable from the pull request.
        pull_request.fork = Some("api-fork".to_owned());
        assert_eq!(
            sources(&refs, PlatformKind::AzureDevOps, &pull_request)[0],
            ("c2".to_owned(), refs.head.clone())
        );
    }

    #[test]
    fn only_github_com_and_hosts_with_an_account_are_github() {
        let remotes = [
            info("origin", "https://github.com/acme/app"),
            info("ghe", "git@ghe.corp.example:team/svc.git"),
            info("lab", "https://gitlab.com/acme/app.git"),
            info("lab2", "git@gitlab.com:acme/other.git"),
            info("work", "https://dev.azure.com/contoso/Shop/_git/api"),
        ];
        let candidates = hosted_repositories(&remotes, PlatformKind::GitHub);
        let to_ask = hosts_to_ask_about(&candidates);
        assert_eq!(to_ask, ["ghe.corp.example", "gitlab.com"]);

        let github = on_github(candidates, &["ghe.corp.example".to_owned()]);
        let found: Vec<_> = github.iter().map(|h| h.key.to_string()).collect();
        assert_eq!(found, ["github.com/acme/app", "ghe.corp.example/team/svc"]);
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

        async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
            if self.failing.contains(remote) {
                return Err(PlatformError::SignedOut(remote.host.clone()));
            }
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
        let known = KnownPullRequests::default();
        fetch_pull_request(&platform, &fetch, &Anonymous, &known, &app, 7)
            .await
            .unwrap();

        let refs = PullRequestRefs::of(&app, 7);
        let mut pull_request = pr(7);
        pull_request.target_branch = "release".to_owned();
        assert_eq!(
            *fetched.lock().unwrap(),
            [("upstream".to_owned(), refs.updates(PlatformKind::GitHub, &pull_request))]
        );
    }

    #[tokio::test]
    async fn an_azure_devops_pull_request_is_fetched_from_the_remote_of_its_repository_by_its_source_branch() {
        let api = RemoteKey {
            host: "dev.azure.com".to_owned(),
            path: "contoso/Shop/api".to_owned(),
        };
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
        };
        let (fetch, fetched) = fetching(vec![
            info("origin", "https://github.com/acme/app"),
            info("work", "git@ssh.dev.azure.com:v3/contoso/Shop/api"),
        ]);

        fetch_pull_request(&platform, &fetch, &Anonymous, &KnownPullRequests::default(), &api, 7)
            .await
            .unwrap();

        let refs = PullRequestRefs::of(&api, 7);
        let fetched = fetched.lock().unwrap();
        let [(remote, updates)] = &fetched[..] else {
            panic!("{fetched:?}")
        };
        assert_eq!(remote, "work");
        assert_eq!(
            updates
                .iter()
                .map(|u| (u.source.as_str(), u.destination.as_str()))
                .collect::<Vec<_>>(),
            [
                ("refs/heads/feature", refs.head.as_str()),
                ("refs/heads/release", refs.target.as_str())
            ]
        );
    }

    #[tokio::test]
    async fn a_pull_request_that_is_fetched_is_known_by_its_title_and_whether_it_is_a_draft() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
        };
        let (fetch, _) = fetching(vec![info("origin", "https://github.com/acme/app")]);
        let known = KnownPullRequests::default();
        assert_eq!(known.get(&app, 7), None);

        fetch_pull_request(&platform, &fetch, &Anonymous, &known, &app, 7)
            .await
            .unwrap();

        assert_eq!(
            known.get(&app, 7),
            Some(PullRequestHead {
                title: "PR 7".to_owned(),
                draft: false
            })
        );
        assert_eq!(known.get(&app, 8), None);
    }

    #[tokio::test]
    async fn a_pull_request_the_platform_cannot_describe_is_not_fetched() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![],
            failing: vec![app.clone()],
        };
        let (fetch, fetched) = fetching(vec![info("origin", "https://github.com/acme/app")]);

        let error = fetch_pull_request(&platform, &fetch, &Anonymous, &KnownPullRequests::default(), &app, 7)
            .await
            .unwrap_err();

        assert_eq!(
            error,
            FetchError::Platform(PlatformError::SignedOut("github.com".to_owned()))
        );
        assert!(fetched.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_repository_that_is_none_of_the_remotes_is_refused_and_nothing_is_fetched() {
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
        };
        let (fetch, fetched) = fetching(vec![info("origin", "https://github.com/ada/app")]);

        let known = KnownPullRequests::default();
        let error = fetch_pull_request(&platform, &fetch, &Anonymous, &known, &github("acme/app"), 7)
            .await
            .unwrap_err();

        assert_eq!(
            error,
            FetchError::Git("none of the remotes of this repository is github.com/acme/app".to_owned())
        );
        assert!(fetched.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_list_has_what_waits_for_the_viewer_and_what_is_theirs_newest_first() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![(app.clone(), vec![waiting(1, 100), mine(2, 300), pr(3), mine(4, 200)])],
            failing: vec![],
        };

        let known = KnownPullRequests::default();
        let list = list_pull_requests(&platform, std::slice::from_ref(&app), None, &known).await;

        let shown: Vec<_> = list.rows.iter().map(|r| (r.row.number, r.row.waiting_for_me)).collect();
        assert_eq!(shown, [(2, false), (4, false), (1, true)], "pr 3 is neither");
        assert_eq!(
            list.sources,
            [RepositoryStatus {
                remote: app.clone(),
                error: None
            }]
        );
        assert_eq!(known.get(&app, 4).map(|h| h.title), Some("PR 4".to_owned()));
        assert_eq!(known.get(&app, 3), None, "what is not listed is not known");
    }

    #[tokio::test]
    async fn the_pull_request_on_view_is_listed_though_it_is_neither_the_viewers_nor_waiting_for_them() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![(app.clone(), vec![mine(2, 300)])],
            failing: vec![],
        };
        let known = KnownPullRequests::default();

        let list = list_pull_requests(&platform, std::slice::from_ref(&app), Some((&app, 3)), &known).await;

        let shown: Vec<_> = list.rows.iter().map(|r| r.row.number).collect();
        assert_eq!(shown, [3, 2], "newest first like the others");
        assert!(known.get(&app, 3).is_some());

        // Listed anyway, it is not listed twice.
        let list = list_pull_requests(&platform, std::slice::from_ref(&app), Some((&app, 2)), &known).await;
        assert_eq!(list.rows.len(), 1);

        // Another repository's number is not this one's.
        let lib = github("acme/lib");
        let list = list_pull_requests(&platform, std::slice::from_ref(&app), Some((&lib, 3)), &known).await;
        assert_eq!(list.rows.len(), 1);
    }

    #[tokio::test]
    async fn a_repository_that_cannot_be_asked_is_reported_and_the_others_still_list() {
        let (app, lib) = (github("acme/app"), github("acme/lib"));
        let platform = Hosting {
            prs: vec![(lib.clone(), vec![mine(9, 50)])],
            failing: vec![app.clone()],
        };

        let list = list_pull_requests(
            &platform,
            &[app.clone(), lib.clone()],
            None,
            &KnownPullRequests::default(),
        )
        .await;

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
