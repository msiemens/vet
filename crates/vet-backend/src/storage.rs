//! Requests about what the projects hold on disk: how much, clearing what is only speed, and
//! removing a project with everything held for it (§9).

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use vet_app::gc::Retention;
use vet_app::ports::SettingsStore;
use vet_app::storage::{ProjectUsage, Usage};
use vet_infra::clock::unix_now;
use vet_protocol::{AppStorage, Event, ProjectStorage, Response};

use crate::Orchestrator;
use crate::error::Result;

/// How long the figures of a computation are served again before the disk is measured anew.
const FRESH: Duration = Duration::from_secs(30);

/// The figures of the last computation, for every known project.
pub(crate) struct Computed {
    at: Instant,
    unix: i64,
    usage: Usage,
}

fn project_storage(project: &ProjectUsage, open: bool, computed_at: i64) -> ProjectStorage {
    ProjectStorage {
        root: project.root.clone(),
        folder_exists: project.folder_exists,
        open,
        recent: project.last_opened.is_some(),
        last_opened: project.last_opened.map(|at| u32::try_from(at).unwrap_or(0)),
        trusted: project.trusted,
        review_data: project.review_data,
        source_trees: project.source_trees,
        source_trees_in_use: project.source_trees_in_use,
        semantic_cache: project.semantic_cache,
        open_reviews: project.open_reviews,
        unposted_comments: project.unposted_comments,
        pending_votes: project.pending_votes,
        computed_at: u32::try_from(computed_at).unwrap_or(0),
    }
}

impl Orchestrator {
    /// What every known project takes, as last measured when that is recent and covers `also`. What
    /// removing a project loses is read anew every time: the figures in bytes may be a while old,
    /// the confirmation about to be shown must not be.
    async fn usage(&self, also: &[String]) -> Result<(Usage, i64)> {
        let (mut usage, unix) = self.measured_usage(also).await?;
        for project in &mut usage.projects {
            let facts = self.storage.facts(&project.root).await?;
            project.open_reviews = facts.open_reviews;
            project.unposted_comments = facts.unposted_comments;
            project.pending_votes = facts.pending_votes;
        }
        Ok((usage, unix))
    }

    async fn measured_usage(&self, also: &[String]) -> Result<(Usage, i64)> {
        let mut cached = self.measured.lock().await;
        if let Some(computed) = cached.as_ref().filter(|c| c.at.elapsed() < FRESH) {
            let covered = also
                .iter()
                .all(|root| computed.usage.projects.iter().any(|p| &p.root == root));
            if covered {
                return Ok((computed.usage.clone(), computed.unix));
            }
        }
        let usage = self.storage.usage(also).await?;
        let unix = unix_now();
        *cached = Some(Computed {
            at: Instant::now(),
            unix,
            usage: usage.clone(),
        });
        Ok((usage, unix))
    }

    /// Forgets the measured figures: the disk changed.
    pub(crate) async fn measure_again(&self) {
        *self.measured.lock().await = None;
    }

    async fn is_open(&self, root: &str) -> bool {
        self.projects
            .all()
            .await
            .iter()
            .any(|open| open.repo == Path::new(root))
    }

    pub(crate) async fn project_storage(&self, root: String) -> Result<Response> {
        let (usage, computed_at) = self.usage(std::slice::from_ref(&root)).await?;
        let open = self.is_open(&root).await;
        let project = usage
            .projects
            .iter()
            .find(|p| p.root == root)
            .map(|p| project_storage(p, open, computed_at))
            .expect("a usage that covers the root has it");
        Ok(Response::ProjectStorage { project })
    }

    pub(crate) async fn projects_storage(&self) -> Result<Response> {
        let (usage, computed_at) = self.usage(&[]).await?;
        let open: HashSet<_> = self.projects.all().await.iter().map(|o| o.repo.clone()).collect();
        Ok(Response::ProjectsStorage {
            projects: usage
                .projects
                .iter()
                .map(|p| project_storage(p, open.contains(Path::new(&p.root)), computed_at))
                .collect(),
            app: AppStorage {
                semantic_cache: usage.shared.semantic_cache,
                language_servers: usage.shared.language_servers,
                databases: usage.shared.databases,
            },
        })
    }

    /// Removes the kept source trees of the project that nothing uses and the answers cached for
    /// its snapshots.
    pub(crate) async fn clear_project_caches(&self, root: String) -> Result<Response> {
        self.trees.at(&root).clear().await;
        self.storage.clear_answers(&root).await?;
        self.measure_again().await;
        self.process_subscribers.broadcast(Event::StorageChanged);
        self.project_storage(root).await
    }

    /// Deletes everything held for the project at `root`, which need not exist any more: its
    /// connections are told and the project shut first, and opening the repository again waits
    /// until the files and rows are gone.
    pub(crate) async fn remove_project(&self, root: String) -> Result<Response> {
        let facts = self.storage.facts(&root).await?;
        let retention = Retention {
            closed_changesets: self.store.backend_settings().await?.finished_review_retention(),
            ..Retention::default()
        };
        self.projects
            .evict(
                Path::new(&root),
                self.storage.remove(&root, &facts, unix_now(), &retention),
            )
            .await?;
        self.measure_again().await;
        self.process_subscribers.broadcast(Event::ProjectRemoved { root });
        self.process_subscribers.broadcast(Event::RecentProjectsChanged);
        self.process_subscribers.broadcast(Event::StorageChanged);
        Ok(Response::Done)
    }
}
