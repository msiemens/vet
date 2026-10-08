//! Pull requests as changesets (design §4.1): where git keeps what a pull request is made of,
//! which of a project's remotes are hosted repositories, the pull requests the project's
//! repositories list, and bringing one in to review.

use crate::fetch::{Credentials, Fetch, FetchMark};
use crate::platform::{
    Platform, PlatformError, PlatformKind, PullRequest, PullRequestHead, PullRequestQuery, PullRequestRow, QueueTab,
    RemoteKey, list_rows, parse_remote, rows,
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
        let prefix = format!("{}{number}", Self::of_repository(remote));
        Self {
            head: format!("{prefix}/{HEAD}"),
            target: format!("{prefix}/{TARGET}"),
        }
    }

    /// What the refs of every pull request of `remote` start with.
    fn of_repository(remote: &RemoteKey) -> String {
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
        format!("refs/vet/pr/{key}/")
    }

    /// The number of the pull request of `remote` that `name` is a ref of, if it is one.
    fn number_of(remote: &RemoteKey, name: &str) -> Option<u64> {
        let (number, which) = name.strip_prefix(&Self::of_repository(remote))?.split_once('/')?;
        let parsed: u64 = number.parse().ok()?;
        (parsed.to_string() == number && matches!(which, HEAD | TARGET)).then_some(parsed)
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

const HEAD: &str = "head";
const TARGET: &str = "target";

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
    /// The platform stopped at the most it lists: a pull request that is not listed may still
    /// be open.
    pub truncated: bool,
}

impl RepositoryStatus {
    /// The repository's open pull requests are all listed.
    pub fn listed_whole(&self) -> bool {
        self.error.is_none() && !self.truncated
    }
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

/// The open pull requests of `repositories`, and `on_view` (a repository and a number) when it is
/// one of them. A repository the platform cannot answer for is reported in `sources`; the others
/// still list.
pub async fn list_pull_requests<P: Platform>(
    platform: &P,
    repositories: &[RemoteKey],
    on_view: Option<(&RemoteKey, u64)>,
) -> PullRequestList {
    let mut rows = Vec::new();
    let mut sources = Vec::new();
    for remote in repositories {
        let (mut listed, truncated, error) =
            match list_rows(platform, remote, &PullRequestQuery::new(QueueTab::AllOpen)).await {
                Ok(found) => (found.rows, found.truncated, None),
                Err(e) => (Vec::new(), false, Some(e)),
            };
        if error.is_none() {
            if let Some((_, number)) =
                on_view.filter(|(r, n)| *r == remote && !listed.iter().any(|row| row.number == *n))
                && let Some(row) = row_of(platform, remote, number).await
            {
                listed.push(row);
            }
            rows.extend(listed.into_iter().map(|row| ListedPullRequest {
                remote: remote.clone(),
                row,
            }));
        }
        sources.push(RepositoryStatus {
            remote: remote.clone(),
            error,
            truncated,
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
/// changeset can be made of them. Answers with how the platform names it now.
pub async fn fetch_pull_request<P: Platform, G: Fetcher>(
    platform: &P,
    fetch: &Fetch<G>,
    credentials: &(impl Credentials + Sync),
    remote: &RemoteKey,
    number: u64,
) -> Result<PullRequestHead, FetchError> {
    let detail = platform
        .pull_request(remote, number)
        .await
        .map_err(FetchError::Platform)?;
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
        .map_err(FetchError::Git)?;
    Ok(PullRequestHead::of(&detail.pull_request))
}

/// The pull requests that were fetched into the repository and that `list` no longer has, of the
/// repositories it lists whole: those closed since, each once.
/// A repository whose refs are named like another's of `list` is left out, as its refs cannot be
/// told apart.
pub async fn unlisted_fetched<G: Fetcher>(
    fetch: &Fetch<G>,
    list: &PullRequestList,
) -> Result<Vec<(RemoteKey, u64)>, String> {
    let mut found = Vec::new();
    for source in list.sources.iter().filter(|s| s.listed_whole()) {
        let remote = &source.remote;
        let prefix = PullRequestRefs::of_repository(remote);
        let shared = list
            .sources
            .iter()
            .any(|other| other.remote != *remote && PullRequestRefs::of_repository(&other.remote) == prefix);
        if shared {
            continue;
        }
        for name in fetch.refs_below(&prefix).await? {
            let Some(number) = PullRequestRefs::number_of(remote, &name) else {
                continue;
            };
            let listed = list.rows.iter().any(|l| l.remote == *remote && l.row.number == number);
            let pull_request = (remote.clone(), number);
            if !listed && !found.contains(&pull_request) {
                found.push(pull_request);
            }
        }
    }
    Ok(found)
}

/// Deletes the refs of `pull_requests`, except those fetched since `since`: a pull request being
/// brought in meanwhile keeps its refs. Answers with the names of the refs that are gone.
pub async fn delete_pull_request_refs<G: Fetcher>(
    fetch: &Fetch<G>,
    pull_requests: &[(RemoteKey, u64)],
    since: FetchMark,
) -> Result<Vec<String>, String> {
    let names = pull_requests
        .iter()
        .flat_map(|(remote, number)| {
            let refs = PullRequestRefs::of(remote, *number);
            [refs.head, refs.target]
        })
        .collect();
    fetch.delete_refs(names, since).await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::platform::{
        CheckLog, CommitChecks, Credential, PullRequest, PullRequestDetail, PullRequestListing, ReviewRequest, Viewer,
        pr, viewer,
    };
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

    /// Lists a fixed set of pull requests per repository, or fails for some, or says it stopped
    /// at its bound for some.
    struct Hosting {
        prs: Vec<(RemoteKey, Vec<PullRequest>)>,
        failing: Vec<RemoteKey>,
        truncated: Vec<RemoteKey>,
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
        ) -> Result<PullRequestListing, PlatformError> {
            if self.failing.contains(remote) {
                return Err(PlatformError::Rejected("github.com refused the token".to_owned()));
            }
            Ok(PullRequestListing {
                pull_requests: self
                    .prs
                    .iter()
                    .find(|(r, _)| r == remote)
                    .map(|(_, prs)| prs.clone())
                    .unwrap_or_default(),
                truncated: self.truncated.contains(remote),
            })
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

        async fn checks(&self, _: &RemoteKey, _: &str) -> Result<CommitChecks, PlatformError> {
            unreachable!("pull requests are listed and fetched")
        }

        async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
            unreachable!("pull requests are listed and fetched")
        }
    }

    fn waiting(number: u64, updated_at: i64) -> PullRequest {
        let mut p = pr(number);
        p.review_requests = vec![ReviewRequest::User("ada".to_owned())];
        p.updated_at = updated_at;
        p
    }

    fn other(number: u64, updated_at: i64) -> PullRequest {
        let mut p = pr(number);
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

    /// The refs of the repository.
    type Refs = Arc<Mutex<Vec<String>>>;

    /// Git remotes that record what they were asked to fetch, into refs.
    struct Remotes {
        remotes: Vec<RemoteInfo>,
        fetched: Fetched,
        refs: Refs,
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
            let mut refs = self.refs.lock().unwrap();
            for update in &updates {
                if !refs.contains(&update.destination) {
                    refs.push(update.destination.clone());
                }
            }
            self.fetched.lock().unwrap().push((remote.to_owned(), updates));
            Ok(())
        }

        async fn refs_below(&self, prefix: &str) -> Result<Vec<String>, GitError> {
            let refs = self.refs.lock().unwrap();
            Ok(refs.iter().filter(|r| r.starts_with(prefix)).cloned().collect())
        }

        async fn delete_refs(&self, names: Vec<String>) -> Result<(), GitError> {
            assert!(names.iter().all(|n| n.starts_with("refs/vet/pr/")), "{names:?}");
            self.refs.lock().unwrap().retain(|r| !names.contains(r));
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
        let (fetch, fetched, _) = fetching_into(remotes, &[]);
        (fetch, fetched)
    }

    /// A repository with the refs `refs` already.
    fn fetching_into(remotes: Vec<RemoteInfo>, refs: &[&str]) -> (Fetch<Remotes>, Fetched, Refs) {
        let fetched = Arc::default();
        let refs: Refs = Arc::new(Mutex::new(refs.iter().map(|r| (*r).to_owned()).collect()));
        let git = Remotes {
            remotes,
            fetched: Arc::clone(&fetched),
            refs: Arc::clone(&refs),
        };
        (Fetch::new(git, Tasks::default()), fetched, refs)
    }

    #[tokio::test]
    async fn a_pull_request_is_fetched_from_the_remote_that_is_its_repository_with_the_target_the_platform_names() {
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
            truncated: vec![],
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
            truncated: vec![],
        };
        let (fetch, fetched) = fetching(vec![
            info("origin", "https://github.com/acme/app"),
            info("work", "git@ssh.dev.azure.com:v3/contoso/Shop/api"),
        ]);

        fetch_pull_request(&platform, &fetch, &Anonymous, &api, 7)
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
    async fn a_pull_request_that_is_fetched_answers_with_its_title_and_whether_it_is_a_draft() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
            truncated: vec![],
        };
        let (fetch, _) = fetching(vec![info("origin", "https://github.com/acme/app")]);

        let head = fetch_pull_request(&platform, &fetch, &Anonymous, &app, 7)
            .await
            .unwrap();

        assert_eq!(
            head,
            PullRequestHead {
                title: "PR 7".to_owned(),
                draft: false
            }
        );
    }

    #[tokio::test]
    async fn a_pull_request_the_platform_cannot_describe_is_not_fetched() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![],
            failing: vec![app.clone()],
            truncated: vec![],
        };
        let (fetch, fetched) = fetching(vec![info("origin", "https://github.com/acme/app")]);

        let error = fetch_pull_request(&platform, &fetch, &Anonymous, &app, 7)
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
            truncated: vec![],
        };
        let (fetch, fetched) = fetching(vec![info("origin", "https://github.com/ada/app")]);

        let error = fetch_pull_request(&platform, &fetch, &Anonymous, &github("acme/app"), 7)
            .await
            .unwrap_err();

        assert_eq!(
            error,
            FetchError::Git("none of the remotes of this repository is github.com/acme/app".to_owned())
        );
        assert!(fetched.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_list_has_every_open_pull_request_of_the_repository_newest_first() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![(
                app.clone(),
                vec![waiting(1, 100), mine(2, 300), other(3, 250), mine(4, 200)],
            )],
            failing: vec![],
            truncated: vec![],
        };

        let list = list_pull_requests(&platform, std::slice::from_ref(&app), None).await;

        let shown: Vec<_> = list.rows.iter().map(|r| (r.row.number, r.row.waiting_for_me)).collect();
        assert_eq!(
            shown,
            [(2, false), (3, false), (4, false), (1, true)],
            "pr 3 is neither the viewer's nor waiting for them, and is listed"
        );
        assert_eq!(
            list.sources,
            [RepositoryStatus {
                remote: app.clone(),
                error: None,
                truncated: false
            }]
        );
    }

    #[tokio::test]
    async fn the_pull_request_on_view_is_listed_though_it_is_neither_the_viewers_nor_waiting_for_them() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![(app.clone(), vec![mine(2, 300)])],
            failing: vec![],
            truncated: vec![],
        };
        let list = list_pull_requests(&platform, std::slice::from_ref(&app), Some((&app, 3))).await;

        let shown: Vec<_> = list.rows.iter().map(|r| r.row.number).collect();
        assert_eq!(shown, [3, 2], "newest first like the others");

        // Listed anyway, it is not listed twice.
        let list = list_pull_requests(&platform, std::slice::from_ref(&app), Some((&app, 2))).await;
        assert_eq!(list.rows.len(), 1);

        // Another repository's number is not this one's.
        let lib = github("acme/lib");
        let list = list_pull_requests(&platform, std::slice::from_ref(&app), Some((&lib, 3))).await;
        assert_eq!(list.rows.len(), 1);
    }

    #[tokio::test]
    async fn a_repository_that_cannot_be_asked_is_reported_and_the_others_still_list() {
        let (app, lib) = (github("acme/app"), github("acme/lib"));
        let platform = Hosting {
            prs: vec![(lib.clone(), vec![mine(9, 50)])],
            failing: vec![app.clone()],
            truncated: vec![],
        };

        let list = list_pull_requests(&platform, &[app.clone(), lib.clone()], None).await;

        assert_eq!(
            list.rows.iter().map(|r| (&r.remote, r.row.number)).collect::<Vec<_>>(),
            [(&lib, 9)]
        );
        assert_eq!(
            list.sources,
            [
                RepositoryStatus {
                    remote: app,
                    error: Some(PlatformError::Rejected("github.com refused the token".to_owned())),
                    truncated: false
                },
                RepositoryStatus {
                    remote: lib,
                    error: None,
                    truncated: false
                },
            ]
        );
    }

    #[tokio::test]
    async fn the_fetched_pull_requests_a_list_no_longer_has_are_those_of_repositories_it_lists_whole() {
        let (app, lib, svc) = (github("acme/app"), github("acme/lib"), github("acme/svc"));
        let platform = Hosting {
            prs: vec![(app.clone(), vec![mine(1, 10)]), (svc.clone(), vec![mine(3, 10)])],
            failing: vec![lib.clone()],
            truncated: vec![svc.clone()],
        };
        let of = |remote: &RemoteKey, number| PullRequestRefs::of(remote, number);
        let refs = [
            of(&app, 1).head,
            of(&app, 1).target,
            of(&app, 5).head,
            of(&app, 5).target,
            of(&app, 6).target,
            of(&lib, 2).head,
            of(&svc, 4).head,
            // Not the refs of a pull request of app.
            "refs/vet/pr/github_com/acme/app/7/merge".to_owned(),
            "refs/vet/pr/github_com/acme/app/08/head".to_owned(),
            "refs/vet/pr/github_com/acme/app/x/head".to_owned(),
            "refs/vet/changesets/3/1/target".to_owned(),
        ];
        let refs: Vec<&str> = refs.iter().map(String::as_str).collect();
        let (fetch, _, _) = fetching_into(vec![], &refs);

        let list = list_pull_requests(&platform, &[app.clone(), lib, svc], None).await;
        let unlisted = unlisted_fetched(&fetch, &list).await.unwrap();

        assert_eq!(
            unlisted,
            [(app.clone(), 5), (app, 6)],
            "a repository that failed or was cut short may still have them open"
        );
    }

    #[tokio::test]
    async fn repositories_whose_refs_are_named_alike_are_not_told_apart_so_neither_loses_any() {
        let (dotted, underscored) = (github("acme/a.b"), github("acme/a_b"));
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
            truncated: vec![],
        };
        let stale = PullRequestRefs::of(&dotted, 5).head;
        assert_eq!(stale, PullRequestRefs::of(&underscored, 5).head);
        let (fetch, _, _) = fetching_into(vec![], &[&stale]);

        let list = list_pull_requests(&platform, &[dotted.clone(), underscored], None).await;
        assert_eq!(unlisted_fetched(&fetch, &list).await.unwrap(), []);

        let list = list_pull_requests(&platform, std::slice::from_ref(&dotted), None).await;
        assert_eq!(unlisted_fetched(&fetch, &list).await.unwrap(), [(dotted, 5)]);
    }

    #[tokio::test]
    async fn the_refs_of_a_pull_request_fetched_after_the_mark_are_not_deleted() {
        let app = github("acme/app");
        let platform = Hosting {
            prs: vec![],
            failing: vec![],
            truncated: vec![],
        };
        let of = |number| PullRequestRefs::of(&app, number);
        let before = [of(5).head, of(5).target, of(6).head, of(6).target];
        let before: Vec<&str> = before.iter().map(String::as_str).collect();
        let (fetch, _, refs) = fetching_into(vec![info("origin", "https://github.com/acme/app")], &before);
        fetch_pull_request(&platform, &fetch, &Anonymous, &app, 4)
            .await
            .unwrap();

        let since = fetch.mark();
        // Opened while the list was being asked for.
        fetch_pull_request(&platform, &fetch, &Anonymous, &app, 5)
            .await
            .unwrap();
        let deleted = delete_pull_request_refs(&fetch, &[(app.clone(), 4), (app.clone(), 5), (app.clone(), 6)], since)
            .await
            .unwrap();

        assert_eq!(deleted, [of(4).head, of(4).target, of(6).head, of(6).target]);
        assert_eq!(*refs.lock().unwrap(), [of(5).head, of(5).target]);
    }
}
