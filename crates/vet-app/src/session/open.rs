//! Opening a session: resuming the working tree's open review or starting out browsing.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use vet_core::content::Snapshot;
use vet_core::filter::ReviewFilter;

use super::ahead::Copies;
use super::exchange::Exchange;
use super::facts::Facts;
use super::hold::{Published, Writer};
use super::source::Stored;
use super::state::{OpenReview, State};
use super::{LiveSession, Result, UnixClock};
use crate::acquisition::Acquisition;
use crate::base;
use crate::changeset::{BaseChoice, ChangesetId, CommitId, Source, Version};
use crate::platform::PullRequestCache;
use crate::ports::{Dirty, GitRepo, LiveStore, ReviewWrites, Scan};
use crate::progress::Tasks;
use crate::review::{ReviewId, visible_threads};
use crate::settle::Wanted;

/// What a session is called: the repository's directory name, else its whole path.
pub(super) fn name_of_repo(repo: &str) -> String {
    std::path::Path::new(repo)
        .file_name()
        .map_or_else(|| repo.to_owned(), |n| n.to_string_lossy().into_owned())
}

/// Git's `user.name`, else "You".
async fn author_of<G: GitRepo>(git: &G) -> String {
    git.user_name()
        .await
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "You".into())
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Resumes the open review of the working tree of the repository at `repo`
    /// (the root, as the key changesets are recorded under), or else starts out
    /// browsing the tree against the current branch's tip. The threads that are
    /// still open from earlier reviews are on view either way.
    pub async fn open(git: G, store: S, repo: String, clock: UnixClock) -> Result<Self> {
        Self::open_reporting(git, store, repo, clock, Tasks::default()).await
    }

    /// Like [`LiveSession::open`], with the scans and diffs of the session reporting to `tasks`.
    pub async fn open_reporting(git: G, store: S, repo: String, clock: UnixClock, tasks: Tasks) -> Result<Self> {
        let acquisition = Arc::new(Acquisition::new(git, store).reporting(tasks));
        let filter = acquisition.store.filter(repo.clone()).await?.normalized();
        let name = name_of_repo(&repo);
        let source = Source::WorkingTree;
        let writes = acquisition.store.writes();
        let (changeset, threads) = match Self::stored(&acquisition, &writes, &repo, &source, &filter, clock).await? {
            Stored::Resumed(mut state) => {
                state.off_base = base::head_left(&acquisition.git, &state.base_commit).await;
                let session = Self::assemble(acquisition, name, repo, clock, *state).await;
                session.catch_up().await?;
                return Ok(session);
            }
            Stored::Browsing { changeset, threads } => (changeset, threads),
        };
        let git = &acquisition.git;
        let head = git.head().await?;
        // Browsing keeps the base the tree was opened against across restarts, until a review
        // that ends lets the next session pin its own.
        let pinned = acquisition.store.pinned_base(repo.clone(), source.clone()).await?;
        let (base_choice, base_commit, base) = match pinned {
            Some(commit) => match git.commit_snapshot(&commit).await {
                Ok(base) => {
                    let (choice, commit) = base::for_working_tree(&commit);
                    (choice, commit, base)
                }
                Err(_) => Self::pin(&acquisition, &writes, &repo, &source, &head).await?,
            },
            None => Self::pin(&acquisition, &writes, &repo, &source, &head).await?,
        };
        let base = Arc::new(base);
        let off_base = base::head_left(git, &base_commit).await;
        // The session opens at `HEAD`, which builds on itself.
        let Scan {
            snapshot: target,
            status,
        } = acquisition.reconcile(Dirty::All).await?;
        let target = Arc::new(target);
        acquisition.record_commit(base.id, base_commit.clone());
        acquisition.record_commit(target.id, head.clone());
        let state = State {
            source,
            revision: 0,
            snapshot: 0,
            seq: 0,
            changeset,
            review: None,
            filter,
            project: None,
            base,
            base_choice,
            base_commit,
            head,
            off_base,
            target,
            target_status: status,
            known: HashMap::new(),
            recorded: Vec::new(),
            acks: Vec::new(),
            threads,
            changes: None,
        };
        Ok(Self::assemble(acquisition, name, repo, clock, state).await)
    }

    /// Pins the working tree to `head` as its base and remembers that.
    async fn pin(
        acquisition: &Acquisition<G, S>,
        writes: &S::Writes,
        repo: &str,
        source: &Source,
        head: &CommitId,
    ) -> Result<(BaseChoice, CommitId, Snapshot)> {
        let (choice, commit) = base::for_working_tree(head);
        let base = acquisition.git.commit_snapshot(&commit).await?;
        writes
            .set_pinned_base(repo.to_owned(), source.clone(), Some(commit.clone()))
            .await?;
        Ok((choice, commit, base))
    }

    async fn assemble(
        acquisition: Arc<Acquisition<G, S>>,
        name: String,
        repo: String,
        clock: UnixClock,
        state: State,
    ) -> Self {
        let writes = acquisition.store.writes();
        // A session opens with the working tree on view, so the settle loop wants it at once.
        let wanted = state.source == Source::WorkingTree;
        Self {
            author: author_of(&acquisition.git).await,
            facts: Facts::new(&acquisition, repo.clone(), clock),
            exchange: Exchange::new(Arc::default()),
            copies: Copies::new(&acquisition),
            acquisition,
            name,
            repo,
            clock,
            published: Arc::new(Published::new(state)),
            token: Mutex::new(Writer::new(writes)),
            degraded: std::sync::Mutex::new(None),
            revisions: Arc::default(),
            settle_wanted: Arc::new(Wanted::new(wanted)),
        }
    }

    /// The same session, its copies ahead of a review reading through `git` and writing through
    /// `store`, other handles on the session's repository and store.
    pub fn copying_through(self, git: G, store: S) -> Self {
        Self {
            copies: self.copies.reading_through(self.acquisition.through(git, store)),
            ..self
        }
    }

    /// The same session, keeping what it learns of pull requests in `known`, which is shared with
    /// whoever else asks the platforms about them.
    pub fn knowing_pull_requests_in(self, known: Arc<PullRequestCache>) -> Self {
        Self {
            exchange: Exchange::new(known),
            ..self
        }
    }

    /// The state of the stored review `id` of `changeset`, a changeset of `source`.
    /// `None` if it cannot be reconstructed: its versions, its latest snapshot or its
    /// base commit is gone.
    pub(super) async fn resumed(
        acquisition: &Acquisition<G, S>,
        changeset: ChangesetId,
        id: ReviewId,
        filter: ReviewFilter,
        source: Source,
    ) -> Result<Option<State>> {
        let store = &acquisition.store;
        let Some(review) = store.review(id).await? else {
            return Ok(None);
        };
        let versions = store.versions(changeset).await?;
        let Some(latest) = versions.last() else {
            return Ok(None);
        };
        // The latest version is the state the pull request was last fetched at: it has content.
        let Some(latest_snapshots) = latest.snapshots else {
            return Ok(None);
        };
        let Some((target, target_status)) = store.snapshot(latest_snapshots.target).await? else {
            return Ok(None);
        };
        let Ok(base) = acquisition.git.commit_snapshot(&latest.base_commit).await else {
            return Ok(None);
        };
        let (base, target) = (Arc::new(base), Arc::new(target));
        // The review keeps its base however far the branches have moved since:
        // commits made while Vet was closed are part of what is under review,
        // and re-resolving would turn every mark against the old base into
        // `SeenAgainstOldBase`. Moving the base is the user's call (§4.4).
        // The refs that keep the versions' commits are already in git; taking each again would
        // write it on every return, so only what a rebuild lacks is set.
        let existing: HashMap<(u32, String), CommitId> = acquisition
            .git
            .changeset_refs(changeset)
            .await?
            .into_iter()
            .map(|kept| ((kept.version, kept.name), kept.commit))
            .collect();
        for (i, version) in versions.iter().enumerate() {
            if let Some(snapshots) = version.snapshots {
                acquisition.record_commit(snapshots.target, version.tip.clone());
                acquisition.record_commit(snapshots.base, version.base_commit.clone());
            }
            let previous = Version::last_commits(&versions[..i]);
            Self::keep_commits(acquisition, changeset, version, previous, Some(&existing)).await?;
        }
        let threads = visible_threads(store.threads(changeset).await?, Some(id));
        Ok(Some(State {
            source,
            revision: 0,
            snapshot: 0,
            seq: 0,
            changeset: Some(changeset),
            review: Some(OpenReview {
                id,
                version: latest.n,
                updated_at: latest.at,
                last_reviewed: review.last_reviewed,
                summary: review.summary,
            }),
            filter,
            project: None,
            base,
            base_choice: review.base,
            base_commit: latest.base_commit.clone(),
            head: latest.tip.clone(),
            off_base: false,
            target,
            target_status,
            known: HashMap::new(),
            recorded: review.spans.into_iter().map(Arc::new).collect(),
            acks: review.acks,
            threads,
            changes: None,
        }))
    }

    /// Settles the tree as it is now, so recorded spans and threads remap onto
    /// whatever changed while Vet was not running (I2).
    async fn catch_up(&self) -> Result<()> {
        let scan = self.acquisition.reconcile(Dirty::All).await?;
        if scan.snapshot.id != self.settled_snapshot().await {
            let (revision, retained) = self.settled(scan).await?;
            if let Some(e) = retained {
                tracing::warn!(revision, error = %e, "could not keep the content of the revision");
            }
        }
        Ok(())
    }
}
