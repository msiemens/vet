//! A project as a whole (§9): what it takes on disk, and removing everything Vet holds for it.
//! A project is a repository root as it was recorded; removing works when the folder is gone.

use std::collections::{BTreeSet, HashMap, HashSet};

use vet_core::content::SnapshotId;

use crate::gc::{self, Retention};
use crate::ports::{
    AnswerStore, GcStore, ProjectFacts, ProjectFiles, ProjectStore, RecentStore, SharedFiles, StoreError, VetRefs,
};

/// What one project takes and what removing it would lose.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProjectUsage {
    pub root: String,
    pub folder_exists: bool,
    /// Unix seconds the project was last opened, while it is on the recent list.
    pub last_opened: Option<i64>,
    pub trusted: bool,
    /// Content only this project's snapshots reach.
    pub review_data: u64,
    pub source_trees: u64,
    pub source_trees_in_use: u64,
    pub semantic_cache: u64,
    pub open_reviews: u32,
    pub unposted_comments: u32,
    pub pending_votes: u32,
}

impl ProjectUsage {
    /// Everything of the project that is removed with it.
    pub fn total(&self) -> u64 {
        self.review_data + self.source_trees + self.semantic_cache
    }
}

/// What is kept for no project in particular.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SharedUsage {
    pub semantic_cache: u64,
    pub language_servers: u64,
    pub databases: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Usage {
    /// In root order.
    pub projects: Vec<ProjectUsage>,
    pub shared: SharedUsage,
}

/// The stores that hold a project's data, and the disk they take.
pub struct Storage<S, F, A, R> {
    store: S,
    files: F,
    answers: A,
    refs: R,
}

impl<S, F, A, R> Storage<S, F, A, R>
where
    S: ProjectStore + RecentStore + GcStore + Sync,
    F: ProjectFiles + Sync,
    A: AnswerStore + Sync,
    R: VetRefs + Sync,
{
    pub fn new(store: S, files: F, answers: A, refs: R) -> Self {
        Self {
            store,
            files,
            answers,
            refs,
        }
    }

    pub async fn facts(&self, root: &str) -> Result<ProjectFacts, StoreError> {
        self.store.project_facts(root.to_owned()).await
    }

    /// The roots anything is held for, in the store or in a project directory.
    async fn known(&self) -> Result<BTreeSet<String>, StoreError> {
        let mut roots: BTreeSet<String> = self.store.known_projects().await?.into_iter().collect();
        roots.extend(self.files.roots().await);
        Ok(roots)
    }

    /// What every known project and `also` (a root asked about that nothing may be held for) take.
    pub async fn usage(&self, also: &[String]) -> Result<Usage, StoreError> {
        let mut roots = self.known().await?;
        roots.extend(also.iter().cloned());
        let review_data = self.store.review_data().await?;
        let recent: HashMap<String, i64> = self
            .store
            .recent_projects()
            .await?
            .into_iter()
            .map(|p| (p.path, p.opened_at))
            .collect();
        let mut projects = Vec::new();
        for root in roots {
            let facts = self.store.project_facts(root.clone()).await?;
            let trees = self.files.trees(&root).await;
            projects.push(ProjectUsage {
                folder_exists: self.files.folder_exists(&root).await,
                last_opened: recent.get(&root).copied(),
                trusted: facts.trusted,
                review_data: review_data.get(&root).copied().unwrap_or(0),
                source_trees: trees.total,
                source_trees_in_use: trees.in_use,
                semantic_cache: self.answers.usage(facts.snapshots).await,
                open_reviews: facts.open_reviews,
                unposted_comments: facts.unposted_comments,
                pending_votes: facts.pending_votes,
                root,
            });
        }
        let SharedFiles {
            language_servers,
            databases,
        } = self.files.shared().await;
        Ok(Usage {
            projects,
            shared: SharedUsage {
                semantic_cache: self.answers.file_size().await,
                language_servers,
                databases,
            },
        })
    }

    /// Deletes what is held for `root`, whose `facts` were read before the project was closed:
    /// the rows, then what no other project's snapshots keep alive (content, answers), the files
    /// and the refs in the repository. Only the rows are required: what the rest cannot delete
    /// is garbage the next collection takes, or a stale ref that harms nothing. `now` is unix
    /// seconds.
    pub async fn remove(
        &self,
        root: &str,
        facts: &ProjectFacts,
        now: i64,
        retention: &Retention,
    ) -> Result<(), StoreError> {
        self.store.remove_project(root.to_owned()).await?;
        if let Err(e) = gc::collect(&self.store, now, retention).await {
            tracing::warn!(error = %e.0, "garbage collection after removing a project failed");
        }
        self.forget_answers(&facts.snapshots).await;
        self.files.remove(root).await;
        self.remove_refs(root, facts).await;
        Ok(())
    }

    /// Deletes the answers for every snapshot of `root`, which stays otherwise as it is.
    pub async fn clear_answers(&self, root: &str) -> Result<(), StoreError> {
        let facts = self.store.project_facts(root.to_owned()).await?;
        self.answers.forget(facts.snapshots).await;
        Ok(())
    }

    /// Deletes the answers for the snapshots no remaining changeset names.
    async fn forget_answers(&self, snapshots: &[SnapshotId]) {
        match self.store.named_snapshots(snapshots.to_vec()).await {
            Ok(named) => {
                let named: HashSet<SnapshotId> = named.into_iter().collect();
                let gone = snapshots.iter().copied().filter(|s| !named.contains(s)).collect();
                self.answers.forget(gone).await;
            }
            Err(e) => tracing::warn!(error = %e.0, "the answers of a removed project were not deleted"),
        }
    }

    /// Deletes the refs Vet kept in the repository, if it is still one. Linked worktrees share
    /// the refs below `refs/vet/`, so the pull request refs stay while another known project
    /// shares the repository; changeset ids are unique, so those of this project go in any case.
    async fn remove_refs(&self, root: &str, facts: &ProjectFacts) {
        let Some(common) = self.refs.common_dir(root).await else {
            return;
        };
        let shared = match self.known().await {
            Ok(known) => {
                let mut shared = false;
                for other in known.iter().filter(|other| other.as_str() != root) {
                    if self.refs.common_dir(other).await.as_ref() == Some(&common) {
                        shared = true;
                        break;
                    }
                }
                shared
            }
            // Not knowing who else uses the repository keeps what they may need.
            Err(_) => true,
        };
        let deleted = self.refs.delete_refs(root, facts.changesets.clone(), !shared).await;
        if let Err(e) = deleted {
            tracing::warn!(error = %e, "the refs of a removed project were not deleted");
        }
    }

    /// Deletes what an earlier removal left half done.
    pub async fn sweep(&self) {
        self.files.sweep().await;
    }
}
