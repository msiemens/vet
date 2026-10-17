//! Pull requests as changesets (design §4.1): where git keeps what a pull request is made of,
//! which of a project's remotes are hosted repositories, the pull requests the project's
//! repositories list, and bringing one in to review.

use crate::fetch::{Credentials, Fetch, FetchMark};
use crate::platform::{
    Platform, PlatformError, PlatformKind, Platforms, PullRequest, PullRequestHead, PullRequestQuery, PullRequestRow,
    Push, QueueTab, RemoteKey, list_rows, parse_remote, rows,
};
use crate::ports::{Fetcher, GitError, RefUpdate, RemoteInfo};

/// The refs of the repository that hold a pull request, kept apart from every branch and
/// remote-tracking ref of the user's.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestRefs {
    /// The pull request's head: the commit the review is of.
    pub head: String,
    /// The tip of the branch the pull request is merged into, which its base is measured from.
    pub target: String,
    /// What the refs of the pull request all start with.
    prefix: String,
}

impl PullRequestRefs {
    pub fn of(remote: &RemoteKey, number: u64) -> Self {
        let prefix = format!("{}{number}/", Self::of_repository(remote));
        Self {
            head: format!("{prefix}{HEAD}"),
            target: format!("{prefix}{TARGET}"),
            prefix,
        }
    }

    /// What the refs of the pull request all start with.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// What the refs of the earlier pushes of the pull request start with.
    fn pushes(&self) -> String {
        format!("{}{PUSH}/", self.prefix)
    }

    /// The ref that holds the earlier push of the pull request that was at `commit`.
    pub fn push(&self, commit: &str) -> String {
        format!("{}{commit}", self.pushes())
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
        let ours = matches!(which, HEAD | TARGET) || which.strip_prefix(PUSH).is_some_and(|c| c.starts_with('/'));
        (parsed.to_string() == number && ours).then_some(parsed)
    }

    /// What to fetch from the platform's repository to have both refs: the pull request's head and
    /// its target branch.
    ///
    /// GitHub keeps the head at `refs/pull/<n>/head`, for forks too. Azure DevOps has no such ref
    /// (its `refs/pull/<n>/merge` is a merge commit): the source branch is fetched, and the head
    /// commit itself when the branch is in a fork's repository.
    pub fn updates(&self, kind: PlatformKind, number: u64, branches: Branches<'_>) -> Vec<RefUpdate> {
        let head = match kind {
            PlatformKind::GitHub => format!("refs/pull/{number}/head"),
            PlatformKind::AzureDevOps if branches.fork => branches.head.to_owned(),
            PlatformKind::AzureDevOps => format!("refs/heads/{}", branches.source),
        };
        vec![
            RefUpdate {
                source: head,
                destination: self.head.clone(),
            },
            RefUpdate {
                source: format!("refs/heads/{}", branches.target),
                destination: self.target.clone(),
            },
        ]
    }
}

/// What of a pull request says where to fetch it from.
#[derive(Clone, Copy)]
pub struct Branches<'a> {
    pub source: &'a str,
    pub target: &'a str,
    /// The source branch is in a fork's repository.
    pub fork: bool,
    /// The commit the head is at.
    pub head: &'a str,
}

impl<'a> From<&'a PullRequest> for Branches<'a> {
    fn from(pr: &'a PullRequest) -> Self {
        Self {
            source: &pr.source_branch,
            target: &pr.target_branch,
            fork: pr.fork.is_some(),
            head: &pr.head,
        }
    }
}

impl<'a> From<&'a PullRequestRow> for Branches<'a> {
    fn from(row: &'a PullRequestRow) -> Self {
        Self {
            source: &row.source_branch,
            target: &row.target_branch,
            fork: row.fork.is_some(),
            head: &row.head,
        }
    }
}

const HEAD: &str = "head";
const TARGET: &str = "target";
const PUSH: &str = "push";

/// A repository of a platform that one of the project's git remotes is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HostedRepository {
    /// The git remote to fetch it from.
    pub remote: String,
    pub key: RemoteKey,
}

/// The repositories on `kind` that `remotes` name, each once, in the order of the remotes (a
/// second remote for the same repository is the same repository). A host Vet does not know (and the
/// viewer has no account for) is not one of any kind.
pub fn hosted_repositories(remotes: &[RemoteInfo], platforms: &Platforms, kind: PlatformKind) -> Vec<HostedRepository> {
    let mut found: Vec<HostedRepository> = Vec::new();
    for remote in remotes {
        let Some(key) = parse_remote(&remote.url).filter(|k| platforms.of(&k.host) == Some(kind)) else {
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

/// The names of the hosts of `remotes` that Vet does not know by name, whose account decides
/// whether a repository there is a platform's at all.
pub fn hosts_to_ask_about(remotes: &[RemoteInfo]) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for remote in remotes {
        let Some(key) = parse_remote(&remote.url) else { continue };
        if key.kind().is_none() && !hosts.contains(&key.host) {
            hosts.push(key.host);
        }
    }
    hosts
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

    /// This list with `extra` among its rows, newest first.
    pub fn with_row(mut self, extra: Option<&ListedPullRequest>) -> Self {
        if let Some(extra) = extra {
            self.rows.push(extra.clone());
            sort_newest_first(&mut self.rows);
        }
        self
    }

    /// Whether a row is of pull request `number` of `remote`.
    pub fn has(&self, remote: &RemoteKey, number: u64) -> bool {
        self.rows.iter().any(|l| l.remote == *remote && l.row.number == number)
    }
}

fn sort_newest_first(rows: &mut [ListedPullRequest]) {
    rows.sort_by_key(|listed| std::cmp::Reverse((listed.row.updated_at, listed.row.number)));
}

/// The open pull requests of `repositories`. A repository the platform cannot answer for is
/// reported in `sources`; the others still list.
pub async fn list_pull_requests<P: Platform>(platform: &P, repositories: &[RemoteKey]) -> PullRequestList {
    let mut rows = Vec::new();
    let mut sources = Vec::new();
    for remote in repositories {
        let (listed, truncated, error) =
            match list_rows(platform, remote, &PullRequestQuery::new(QueueTab::AllOpen)).await {
                Ok(found) => (found.rows, found.truncated, None),
                Err(e) => (Vec::new(), false, Some(e)),
            };
        if error.is_none() {
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

/// The row of pull request `number` of `remote` when `list` leaves it out though it listed the
/// repository: the queue shows the pull request on view even when it is neither the viewer's nor
/// waiting for them. `None` when the list has it, did not list the repository, or the platform
/// cannot say what the pull request is.
pub async fn unlisted_row<P: Platform>(
    platform: &P,
    list: &PullRequestList,
    remote: &RemoteKey,
    number: u64,
) -> Option<ListedPullRequest> {
    let asked = list.sources.iter().any(|s| s.remote == *remote && s.error.is_none());
    if !asked || list.has(remote, number) {
        return None;
    }
    let detail = platform.pull_request(remote, number).await.ok()?;
    row_of(platform, remote, detail.pull_request).await
}

/// The row of `pull_request` of `remote`, as the queue would list it; `None` if the platform cannot
/// name the viewer.
pub async fn row_of<P: Platform>(
    platform: &P,
    remote: &RemoteKey,
    pull_request: PullRequest,
) -> Option<ListedPullRequest> {
    let viewer = platform.viewer(remote).await.ok()?;
    let row = rows(vec![pull_request], &viewer, &PullRequestQuery::new(QueueTab::AllOpen)).pop()?;
    Some(ListedPullRequest {
        remote: remote.clone(),
        row,
    })
}

/// Why a pull request could not be brought in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FetchError {
    /// The platform could not say what the pull request is.
    Platform(PlatformError),
    /// None of the repository's remotes is the pull request's repository. Fit to show.
    NoRemote(String),
    /// Git could not fetch it.
    Git(GitError),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Platform(e) => e.fmt(f),
            Self::NoRemote(message) => f.write_str(message),
            Self::Git(e) => e.fmt(f),
        }
    }
}

/// What bringing in a pull request learned of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BroughtIn {
    /// The pull request as the platform has it now, from which its row of the queue is made.
    pub pull_request: PullRequest,
    /// How the platform names it now.
    pub head: PullRequestHead,
    /// The states of its source branch the platform lists, oldest first, the current one last. The
    /// earlier ones that the remote still has are fetched.
    pub pushes: Vec<Push>,
}

/// Brings in the head of pull request `number` of `remote`, the branch it targets and the states
/// the head had before (as far as the remote gives them), so a changeset can be made of them.
pub async fn fetch_pull_request<P: Platform, G: Fetcher>(
    platform: &P,
    fetch: &Fetch<G>,
    credentials: &(impl Credentials + Sync),
    remote: &RemoteKey,
    number: u64,
) -> Result<BroughtIn, FetchError> {
    let detail = platform
        .pull_request(remote, number)
        .await
        .map_err(FetchError::Platform)?;
    let remotes = fetch.remotes().await.map_err(FetchError::Git)?;
    let git_remote = remotes
        .iter()
        .find(|r| parse_remote(&r.url).as_ref() == Some(remote))
        .ok_or_else(|| FetchError::NoRemote(format!("none of the remotes of this repository is {remote}")))?;
    let refs = PullRequestRefs::of(remote, number);
    let updates = refs.updates(
        PlatformKind::of_account(&remote.host),
        number,
        (&detail.pull_request).into(),
    );
    fetch
        .refs(credentials, git_remote, updates, format!("Fetching #{number}"))
        .await
        .map_err(FetchError::Git)?;
    fetch_earlier_pushes(fetch, credentials, git_remote, &refs, &detail.pushes).await;
    Ok(BroughtIn {
        head: PullRequestHead::of(&detail.pull_request),
        pull_request: detail.pull_request,
        pushes: detail.pushes,
    })
}

/// Fetches the states of the source branch before the current one (the last of `pushes`) that are
/// not in the repository, each into a ref of its own. A remote may have dropped some after a
/// force-push; they are asked for one at a time then, and not again while the session lives.
async fn fetch_earlier_pushes<G: Fetcher>(
    fetch: &Fetch<G>,
    credentials: &(impl Credentials + Sync),
    git_remote: &RemoteInfo,
    refs: &PullRequestRefs,
    pushes: &[Push],
) {
    let Some((_, earlier)) = pushes.split_last() else {
        return;
    };
    let have = fetch.refs_below(&refs.pushes()).await.unwrap_or_default();
    let wanted: Vec<RefUpdate> = earlier
        .iter()
        .map(|push| RefUpdate {
            source: push.head.clone(),
            destination: refs.push(&push.head),
        })
        .filter(|update| !fetch.lost(&update.destination) && !have.iter().any(|tip| tip.name == update.destination))
        .collect();
    if wanted.is_empty() || fetch.refs_asking(credentials, git_remote, wanted.clone()).await.is_ok() {
        return;
    }
    for update in wanted {
        let destination = update.destination.clone();
        if fetch.refs_asking(credentials, git_remote, vec![update]).await.is_err() {
            fetch.refused(&destination);
        }
    }
}

/// The pull requests that were fetched into the repository and that `list` no longer has, of the
/// repositories it lists whole: those closed since, each once.
/// A repository whose refs are named like another's of `list` is left out, as its refs cannot be
/// told apart.
pub async fn unlisted_fetched<G: Fetcher>(
    fetch: &Fetch<G>,
    list: &PullRequestList,
) -> Result<Vec<(RemoteKey, u64)>, GitError> {
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
        for tip in fetch.refs_below(&prefix).await? {
            let Some(number) = PullRequestRefs::number_of(remote, &tip.name) else {
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
) -> Result<Vec<String>, GitError> {
    let mut names = Vec::new();
    for (remote, number) in pull_requests {
        let refs = PullRequestRefs::of(remote, *number);
        let held = fetch.refs_below(refs.prefix()).await?;
        names.extend([refs.head.clone(), refs.target.clone()]);
        names.extend(held.into_iter().map(|tip| tip.name));
    }
    names.sort();
    names.dedup();
    fetch.delete_refs(names, since).await
}

/// How many pull requests one fetch brings in.
const HEADS_PER_FETCH: usize = 8;

/// How many of the listed pull requests, newest first across the repositories, are fetched for
/// their lines; the rest has none until it is opened.
const LISTED_FETCHED: usize = 50;

/// Fetches the newest [`LISTED_FETCHED`] pull requests of `list` whose refs are not in the
/// repository yet or whose head ref is not at the head the list names (it was pushed to since), so
/// every such row of the queue has current commits to count lines from. Does not touch a pull
/// request that is there and current, nor one whose head ref is ahead of the list (its commit has
/// the listed head in its history), which a list that lags leaves behind; a force-pushed head is
/// fetched. A batch the remote refuses (a branch deleted since the
/// listing) is fetched one pull request at a time, so one cannot hold back the rest. A second call
/// meanwhile waits and then fetches what the first did not. Answers whether anything was brought in.
pub async fn fetch_listed_heads<G: Fetcher>(
    fetch: &Fetch<G>,
    credentials: &(impl Credentials + Sync),
    list: &PullRequestList,
) -> Result<bool, GitError> {
    let _one_listing = fetch.listed_heads().await;
    let remotes = fetch.remotes().await?;
    let newest: Vec<&ListedPullRequest> = list.rows.iter().take(LISTED_FETCHED).collect();
    let mut brought = false;
    for source in list.sources.iter().filter(|s| s.error.is_none()) {
        let repository = &source.remote;
        let Some(git_remote) = remotes
            .iter()
            .find(|r| parse_remote(&r.url).as_ref() == Some(repository))
        else {
            continue;
        };
        let have = fetch.refs_below(&PullRequestRefs::of_repository(repository)).await?;
        let mut missing: Vec<Vec<RefUpdate>> = Vec::new();
        for listed in newest.iter().filter(|listed| listed.remote == *repository) {
            let number = listed.row.number;
            let refs = PullRequestRefs::of(repository, number);
            let at = |name: &str| have.iter().find(|tip| tip.name == name).map(|tip| tip.commit.as_str());
            let head_current = match at(&refs.head) {
                Some(held) if held == listed.row.head => true,
                // The list lags behind a head that was brought in from a newer source (the pull
                // request's own detail, or its newest push): the ref is kept.
                Some(held) => fetch.descends_from(held, &listed.row.head).await.unwrap_or(false),
                None => false,
            };
            if !head_current || at(&refs.target).is_none() {
                missing.push(refs.updates(PlatformKind::of_account(&repository.host), number, (&listed.row).into()));
            }
        }
        for batch in missing.chunks(HEADS_PER_FETCH) {
            let title = format!("Fetching {} pull requests", batch.len());
            let together = batch.concat();
            if fetch.refs(credentials, git_remote, together, title).await.is_ok() {
                brought = true;
                continue;
            }
            for updates in batch {
                let title = "Fetching a pull request".to_owned();
                brought |= fetch
                    .refs(credentials, git_remote, updates.clone(), title)
                    .await
                    .is_ok();
            }
        }
    }
    Ok(brought)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::platform::{
        CheckLog, CommitChecks, Credential, PullRequest, PullRequestDetail, PullRequestListing, ReviewRequest, Viewer,
        pr, viewer,
    };
    use crate::ports::{FetchAuth, GitError, RefTip};
    use crate::progress::Tasks;

    pub(crate) fn github(path: &str) -> RemoteKey {
        RemoteKey {
            host: "github.com".to_owned(),
            path: path.to_owned(),
        }
    }

    pub(crate) fn info(name: &str, url: &str) -> RemoteInfo {
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
        refs.updates(kind, 7, pull_request.into())
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
    fn only_github_com_and_hosts_with_an_account_are_repositories_and_an_unknown_host_is_refused() {
        let remotes = [
            info("origin", "https://github.com/acme/app"),
            info("ghe", "git@ghe.corp.example:team/svc.git"),
            info("lab", "https://gitlab.com/acme/app.git"),
            info("lab2", "git@gitlab.com:acme/other.git"),
            info("work", "https://dev.azure.com/contoso/Shop/_git/api"),
        ];
        assert_eq!(
            hosts_to_ask_about(&remotes),
            ["ghe.corp.example", "gitlab.com"],
            "the hosts Vet does not know by name"
        );

        // Only the host the viewer signed in to is a platform: gitlab.com is refused, never taken
        // for GitHub.
        let platforms = Platforms::new().with_account("ghe.corp.example");
        let github: Vec<_> = hosted_repositories(&remotes, &platforms, PlatformKind::GitHub)
            .iter()
            .map(|h| h.key.to_string())
            .collect();
        assert_eq!(github, ["github.com/acme/app", "ghe.corp.example/team/svc"]);
        let azure: Vec<_> = hosted_repositories(&remotes, &platforms, PlatformKind::AzureDevOps)
            .iter()
            .map(|h| h.key.to_string())
            .collect();
        assert_eq!(azure, ["dev.azure.com/contoso/Shop/api"]);
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
        let found = hosted_repositories(&remotes, &Platforms::new(), PlatformKind::GitHub);
        let names: Vec<_> = found.iter().map(|h| (h.remote.as_str(), h.key.path.as_str())).collect();
        assert_eq!(names, [("origin", "acme/app"), ("fork", "ada/app")]);
    }

    /// The one pull request `Hosting` describes as a draft.
    const DRAFT: u64 = 9;

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
            pull_request.draft = number == DRAFT;
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
    pub(crate) type Fetched = Arc<Mutex<Vec<(String, Vec<RefUpdate>)>>>;

    /// The refs of the repository.
    pub(crate) type Refs = Arc<Mutex<Vec<RefTip>>>;

    /// The commit the remote has at each of its refs; a ref not in it is at `tip`.
    pub(crate) type Served = Arc<Mutex<std::collections::HashMap<String, String>>>;

    pub(crate) fn names(refs: &Refs) -> Vec<String> {
        refs.lock().unwrap().iter().map(|r| r.name.clone()).collect()
    }

    /// Git remotes that record what they were asked to fetch, into refs.
    pub(crate) struct Remotes {
        remotes: Vec<RemoteInfo>,
        fetched: Fetched,
        refs: Refs,
        served: Served,
        /// Sources the remote does not have: a fetch asking for one fails.
        gone: Vec<String>,
        /// Which commit has which in its history, as (descendant, ancestor).
        history: Vec<(String, String)>,
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
            self.fetched.lock().unwrap().push((remote.to_owned(), updates.clone()));
            if let Some(lacking) = updates.iter().find(|u| self.gone.contains(&u.source)) {
                return Err(GitError::Remote(format!("{} is not on the remote", lacking.source)));
            }
            // The transfer takes a moment, in which another call may look at the refs.
            tokio::task::yield_now().await;
            let mut refs = self.refs.lock().unwrap();
            for update in &updates {
                let commit = self
                    .served
                    .lock()
                    .unwrap()
                    .get(&update.source)
                    .cloned()
                    .unwrap_or_else(|| "tip".to_owned());
                refs.retain(|r| r.name != update.destination);
                refs.push(RefTip {
                    name: update.destination.clone(),
                    commit,
                });
            }
            Ok(())
        }

        async fn refs_below(&self, prefix: &str) -> Result<Vec<RefTip>, GitError> {
            let refs = self.refs.lock().unwrap();
            Ok(refs.iter().filter(|r| r.name.starts_with(prefix)).cloned().collect())
        }

        async fn descends_from(&self, descendant: &str, ancestor: &str) -> Result<bool, GitError> {
            Ok(descendant == ancestor || self.history.iter().any(|(d, a)| d == descendant && a == ancestor))
        }

        async fn delete_refs(&self, names: Vec<String>) -> Result<(), GitError> {
            assert!(names.iter().all(|n| n.starts_with("refs/vet/pr/")), "{names:?}");
            self.refs.lock().unwrap().retain(|r| !names.contains(&r.name));
            Ok(())
        }
    }

    pub(crate) struct Anonymous;

    impl Credentials for Anonymous {
        async fn credential(&self, _: &str) -> Option<Credential> {
            None
        }
    }

    pub(crate) fn fetching(remotes: Vec<RemoteInfo>) -> (Fetch<Remotes>, Fetched) {
        let (fetch, fetched, _) = fetching_into(remotes, &[]);
        (fetch, fetched)
    }

    /// A repository with the refs `refs` already.
    pub(crate) fn fetching_into(remotes: Vec<RemoteInfo>, refs: &[&str]) -> (Fetch<Remotes>, Fetched, Refs) {
        let refs = refs
            .iter()
            .map(|r| RefTip {
                name: (*r).to_owned(),
                commit: "tip".to_owned(),
            })
            .collect();
        let (fetch, fetched, refs, _) = fetching_with(remotes, refs, &[]);
        (fetch, fetched, refs)
    }

    /// A repository with `refs` already, and the remote's [`Served`] to move; the remote lacks the
    /// sources `gone`.
    pub(crate) fn fetching_with(
        remotes: Vec<RemoteInfo>,
        refs: Vec<RefTip>,
        gone: &[&str],
    ) -> (Fetch<Remotes>, Fetched, Refs, Served) {
        fetching_with_history(remotes, refs, gone, &[])
    }

    /// As [`fetching_with`], with commits that have others in their history, as (descendant, ancestor).
    fn fetching_with_history(
        remotes: Vec<RemoteInfo>,
        refs: Vec<RefTip>,
        gone: &[&str],
        history: &[(&str, &str)],
    ) -> (Fetch<Remotes>, Fetched, Refs, Served) {
        let fetched = Arc::default();
        let refs: Refs = Arc::new(Mutex::new(refs));
        let served = Served::default();
        let git = Remotes {
            remotes,
            fetched: Arc::clone(&fetched),
            refs: Arc::clone(&refs),
            served: Arc::clone(&served),
            gone: gone.iter().map(|g| (*g).to_owned()).collect(),
            history: history
                .iter()
                .map(|(d, a)| ((*d).to_owned(), (*a).to_owned()))
                .collect(),
        };
        (Fetch::new(git, Tasks::default()), fetched, refs, served)
    }

    fn listed_ado(numbers: &[u64]) -> PullRequestList {
        let remote = RemoteKey {
            host: "dev.azure.com".to_owned(),
            path: "contoso/Shop/api".to_owned(),
        };
        PullRequestList {
            rows: numbers
                .iter()
                .map(|n| {
                    let mut listed = pr(*n);
                    listed.source_branch = format!("pr{n}");
                    listed.head = format!("c{n}");
                    ListedPullRequest {
                        remote: remote.clone(),
                        row: rows(vec![listed], &viewer(), &PullRequestQuery::new(QueueTab::AllOpen)).remove(0),
                    }
                })
                .collect(),
            sources: vec![RepositoryStatus {
                remote,
                error: None,
                truncated: false,
            }],
        }
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
            [(
                "upstream".to_owned(),
                refs.updates(PlatformKind::GitHub, 7, (&pull_request).into())
            )]
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

        let ready = fetch_pull_request(&platform, &fetch, &Anonymous, &app, 7)
            .await
            .unwrap();
        let draft = fetch_pull_request(&platform, &fetch, &Anonymous, &app, DRAFT)
            .await
            .unwrap();

        assert_eq!(
            ready.head,
            PullRequestHead {
                title: "PR 7".to_owned(),
                draft: false,
                fork: false,
                target_branch: "release".to_owned(),
            }
        );
        assert_eq!(
            draft.head,
            PullRequestHead {
                title: format!("PR {DRAFT}"),
                draft: true,
                fork: false,
                target_branch: "release".to_owned(),
            }
        );
    }

    /// A platform that lists pull request 7 with the given states of its source branch.
    pub(crate) struct Pushed(Vec<Push>);

    impl Pushed {
        pub(crate) fn new(pushes: Vec<Push>) -> Self {
            Self(pushes)
        }
    }

    impl Platform for Pushed {
        fn traits(&self) -> crate::platform::PlatformTraits {
            crate::platform::PlatformTraits::github()
        }

        async fn viewer(&self, _: &RemoteKey) -> Result<Viewer, PlatformError> {
            unreachable!("a pull request is fetched")
        }

        async fn list_pull_requests(
            &self,
            _: &RemoteKey,
            _: &PullRequestQuery,
        ) -> Result<PullRequestListing, PlatformError> {
            unreachable!("a pull request is fetched")
        }

        async fn pull_request(&self, _: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
            Ok(PullRequestDetail {
                pull_request: pr(number),
                base: "b0".to_owned(),
                pushes: self.0.clone(),
            })
        }

        async fn checks(&self, _: &RemoteKey, _: &str) -> Result<CommitChecks, PlatformError> {
            unreachable!("a pull request is fetched")
        }

        async fn check_log(&self, _: &RemoteKey, _: &str) -> Result<CheckLog, PlatformError> {
            unreachable!("a pull request is fetched")
        }
    }

    fn push(head: &str) -> Push {
        Push {
            head: head.to_owned(),
            at: None,
            forced: false,
        }
    }

    #[tokio::test]
    async fn the_earlier_pushes_are_fetched_once_and_one_the_remote_dropped_is_not_asked_for_again() {
        let app = github("acme/app");
        let platform = Pushed(vec![push("c1"), push("c2"), push("c3")]);
        let (fetch, fetched, refs, _) =
            fetching_with(vec![info("origin", "https://github.com/acme/app")], vec![], &["c1"]);

        let brought = fetch_pull_request(&platform, &fetch, &Anonymous, &app, 7)
            .await
            .unwrap();
        fetch_pull_request(&platform, &fetch, &Anonymous, &app, 7)
            .await
            .unwrap();

        let of = PullRequestRefs::of(&app, 7);
        assert_eq!(
            brought.pushes.iter().map(|p| p.head.as_str()).collect::<Vec<_>>(),
            ["c1", "c2", "c3"],
            "the platform's list is passed on whole"
        );
        let names = names(&refs);
        assert!(names.contains(&of.push("c2")), "{names:?}");
        assert!(!names.contains(&of.push("c1")), "the remote lacks it");
        assert!(!names.contains(&of.push("c3")), "the current head has its own ref");
        let sources: Vec<String> = fetched
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, updates)| updates.iter().map(|u| u.source.clone()))
            .filter(|s| s.starts_with('c'))
            .collect();
        assert_eq!(
            sources,
            ["c1", "c2", "c1", "c2"],
            "c1 and c2 together, then each alone; the second call asks for neither"
        );
    }

    #[tokio::test]
    async fn the_refs_of_earlier_pushes_go_with_the_pull_request_when_it_is_closed() {
        let app = github("acme/app");
        let of = PullRequestRefs::of(&app, 7);
        let (fetch, _, refs) = fetching_into(
            vec![],
            &[
                &of.head,
                &of.target,
                &of.push("c1"),
                &PullRequestRefs::of(&app, 70).push("c1"),
            ],
        );
        let since = fetch.mark();

        let deleted = delete_pull_request_refs(&fetch, &[(app.clone(), 7)], since)
            .await
            .unwrap();

        assert_eq!(deleted.len(), 3);
        assert_eq!(names(&refs), [PullRequestRefs::of(&app, 70).push("c1")]);
        let unlisted = unlisted_fetched(&fetch, &listed_none(&app)).await.unwrap();
        assert_eq!(unlisted, [(app, 70)], "a push ref belongs to its pull request");
    }

    fn listed_none(remote: &RemoteKey) -> PullRequestList {
        PullRequestList {
            rows: vec![],
            sources: vec![RepositoryStatus {
                remote: remote.clone(),
                error: None,
                truncated: false,
            }],
        }
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
            FetchError::NoRemote("none of the remotes of this repository is github.com/acme/app".to_owned())
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

        let list = list_pull_requests(&platform, std::slice::from_ref(&app)).await;

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
        let list = list_pull_requests(&platform, std::slice::from_ref(&app)).await;

        let extra = unlisted_row(&platform, &list, &app, 3).await;
        let shown: Vec<_> = list
            .clone()
            .with_row(extra.as_ref())
            .rows
            .iter()
            .map(|r| r.row.number)
            .collect();
        assert_eq!(shown, [3, 2], "newest first like the others");

        // Listed anyway, it is not listed twice.
        assert!(unlisted_row(&platform, &list, &app, 2).await.is_none());

        // Another repository's number is not this one's, and a repository that was not listed has
        // no row added.
        let lib = github("acme/lib");
        assert!(unlisted_row(&platform, &list, &lib, 3).await.is_none());
    }

    #[tokio::test]
    async fn a_repository_that_cannot_be_asked_is_reported_and_the_others_still_list() {
        let (app, lib) = (github("acme/app"), github("acme/lib"));
        let platform = Hosting {
            prs: vec![(lib.clone(), vec![mine(9, 50)])],
            failing: vec![app.clone()],
            truncated: vec![],
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

        let list = list_pull_requests(&platform, &[app.clone(), lib, svc]).await;
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

        let list = list_pull_requests(&platform, &[dotted.clone(), underscored]).await;
        assert_eq!(unlisted_fetched(&fetch, &list).await.unwrap(), []);

        let list = list_pull_requests(&platform, std::slice::from_ref(&dotted)).await;
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
        assert_eq!(names(&refs), [of(5).head, of(5).target]);
    }

    fn ado_remote() -> RemoteInfo {
        info("origin", "https://dev.azure.com/contoso/Shop/_git/api")
    }

    /// The refs of pull request `number` of `list` as a fetch of it leaves them.
    fn fetched_refs(list: &PullRequestList, number: u64) -> Vec<RefTip> {
        let refs = PullRequestRefs::of(&list.sources[0].remote, number);
        vec![
            RefTip {
                name: refs.head,
                commit: format!("c{number}"),
            },
            RefTip {
                name: refs.target,
                commit: "tip".to_owned(),
            },
        ]
    }

    /// The remote has each listed pull request's source branch at the head the list names.
    fn serve_heads(served: &Served, list: &PullRequestList) {
        let mut served = served.lock().unwrap();
        for listed in &list.rows {
            served.insert(
                format!("refs/heads/{}", listed.row.source_branch),
                listed.row.head.clone(),
            );
        }
    }

    fn destinations(fetched: &Fetched) -> Vec<String> {
        fetched
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, u)| u.iter().map(|u| u.destination.clone()))
            .collect()
    }

    #[tokio::test]
    async fn the_listed_pull_requests_that_are_not_in_the_repository_are_fetched_once_in_one_go() {
        let list = listed_ado(&[1, 2, 3]);
        let remote = list.sources[0].remote.clone();
        let of = |number| PullRequestRefs::of(&remote, number);
        let (fetch, fetched, refs, served) = fetching_with(vec![ado_remote()], fetched_refs(&list, 2), &[]);
        serve_heads(&served, &list);

        assert!(fetch_listed_heads(&fetch, &Anonymous, &list).await.unwrap());

        assert_eq!(
            destinations(&fetched),
            [of(1).head, of(1).target, of(3).head, of(3).target]
        );
        assert_eq!(fetched.lock().unwrap().len(), 1, "one fetch for the lot");
        assert!(names(&refs).contains(&of(3).head));

        assert!(!fetch_listed_heads(&fetch, &Anonymous, &list).await.unwrap());
        assert_eq!(fetched.lock().unwrap().len(), 1, "nothing is fetched twice");
    }

    #[tokio::test]
    async fn a_pull_request_pushed_to_since_it_was_fetched_is_fetched_again_and_the_others_are_not() {
        let mut list = listed_ado(&[1, 2]);
        let remote = list.sources[0].remote.clone();
        let refs: Vec<RefTip> = [1, 2].iter().flat_map(|n| fetched_refs(&list, *n)).collect();
        let (fetch, fetched, refs, served) = fetching_with(vec![ado_remote()], refs, &[]);
        list.rows[1].row.head = "c2-pushed".to_owned();
        serve_heads(&served, &list);

        assert!(fetch_listed_heads(&fetch, &Anonymous, &list).await.unwrap());

        let of = PullRequestRefs::of(&remote, 2);
        assert_eq!(destinations(&fetched), [of.head.clone(), of.target]);
        let head = refs
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.name == of.head)
            .cloned()
            .unwrap();
        assert_eq!(head.commit, "c2-pushed");
        assert!(!fetch_listed_heads(&fetch, &Anonymous, &list).await.unwrap());
        assert_eq!(fetched.lock().unwrap().len(), 1, "the head is current now");
    }

    #[tokio::test]
    async fn a_head_ref_ahead_of_a_lagging_list_is_kept_and_a_force_pushed_one_is_fetched() {
        let list = listed_ado(&[1, 2]);
        let remote = list.sources[0].remote.clone();
        let mut refs: Vec<RefTip> = [1, 2].iter().flat_map(|n| fetched_refs(&list, *n)).collect();
        // #1's ref is at a newer push than the list names; #2's was force-pushed to a commit that
        // does not have the listed head in its history.
        for tip in &mut refs {
            if tip.name == PullRequestRefs::of(&remote, 1).head {
                tip.commit = "c1-newer".to_owned();
            }
            if tip.name == PullRequestRefs::of(&remote, 2).head {
                tip.commit = "c2-other".to_owned();
            }
        }
        let (fetch, fetched, _, served) = fetching_with_history(vec![ado_remote()], refs, &[], &[("c1-newer", "c1")]);
        serve_heads(&served, &list);

        assert!(fetch_listed_heads(&fetch, &Anonymous, &list).await.unwrap());

        let of = PullRequestRefs::of(&remote, 2);
        assert_eq!(destinations(&fetched), [of.head, of.target]);
    }

    #[tokio::test]
    async fn only_the_newest_fifty_listed_pull_requests_are_fetched() {
        let last = LISTED_FETCHED as u64;
        let numbers: Vec<u64> = (1..=last + 5).collect();
        let list = listed_ado(&numbers);
        let remote = list.sources[0].remote.clone();
        let (fetch, _, refs, served) = fetching_with(vec![ado_remote()], Vec::new(), &[]);
        serve_heads(&served, &list);

        fetch_listed_heads(&fetch, &Anonymous, &list).await.unwrap();

        let names = names(&refs);
        let has = |number| names.contains(&PullRequestRefs::of(&remote, number).head);
        assert!(has(1) && has(last), "the first fifty rows of the list");
        assert!(!has(last + 1) && !has(last + 5));
    }

    #[tokio::test]
    async fn listings_that_overlap_do_not_fetch_the_same_pull_requests_twice() {
        let list = listed_ado(&[1, 2, 3]);
        let (fetch, fetched, _, served) = fetching_with(vec![ado_remote()], Vec::new(), &[]);
        serve_heads(&served, &list);

        let (first, second) = tokio::join!(
            fetch_listed_heads(&fetch, &Anonymous, &list),
            fetch_listed_heads(&fetch, &Anonymous, &list)
        );

        assert_eq!((first, second), (Ok(true), Ok(false)));
        assert_eq!(fetched.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_pull_request_whose_branch_is_gone_does_not_hold_back_the_others() {
        let mut list = listed_ado(&[1, 2, 3]);
        list.rows[1].row.source_branch = "deleted".to_owned();
        let remote = list.sources[0].remote.clone();
        let (fetch, _, refs, _) = fetching_with(vec![ado_remote()], Vec::new(), &["refs/heads/deleted"]);

        assert!(fetch_listed_heads(&fetch, &Anonymous, &list).await.unwrap());

        let names = names(&refs);
        let has = |number| names.contains(&PullRequestRefs::of(&remote, number).head);
        assert_eq!((has(1), has(2), has(3)), (true, false, true));
    }
}
