//! Requests about the language servers, which are shared by all projects.

use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::BackendId;

use crate::error::Result;
use crate::{Open, Orchestrator};

impl Orchestrator {
    pub(crate) async fn list_servers(&self, project: ProjectId) -> Result<Response> {
        let open = self.find(project).await?;
        self.servers_of(&open).await
    }

    pub(crate) async fn install_server(&self, project: ProjectId, language: String) -> Result<Response> {
        let open = self.find(project).await?;
        let progress = |line: &str| {
            open.subscribers.broadcast(Event::ServerInstallOutput {
                project,
                language: language.clone(),
                line: line.to_owned(),
            });
        };
        self.servers.install(&language, &progress).await?;
        self.servers_replaced(&language).await;
        self.servers_of(&open).await
    }

    pub(crate) async fn set_server_path(
        &self,
        project: ProjectId,
        language: String,
        path: Option<String>,
    ) -> Result<Response> {
        let open = self.find(project).await?;
        self.servers.set_custom_path(&language, path).await?;
        self.servers_replaced(&language).await;
        self.servers_of(&open).await
    }

    /// Replaces the project's servers of `language`. Other projects' servers, which are their
    /// own, keep running. Every connection of the project is told.
    pub(crate) async fn restart_server(&self, project: ProjectId, language: String) -> Result<Response> {
        let language = self.servers.known(&language)?;
        let ready = self.ready(project).await?;
        let started = ready.restart_server(language).await;
        ready.subscribers.broadcast(Event::ServersChanged { project });
        started?;
        self.servers_of(&ready.open).await
    }

    async fn servers_of(&self, open: &Open) -> Result<Response> {
        // The servers are shown as they would run for the changeset on view; before its
        // first scan nothing is known of it, and nothing executes.
        let policy = match open.booted_now() {
            Some(booted) => booted
                .session
                .trust()
                .await
                .map_or(AnalysisPolicy::LIMITED, |(trust, head)| trust.policy(&head)),
            None => AnalysisPolicy::LIMITED,
        };
        let all = self.servers.status(&open.repo, policy).await?;
        // What a server offers is known from the project's own server; where none ran, from another's.
        let booted = self.projects.all().await;
        let features = |language: &BackendId| {
            let mut projects = open
                .booted_now()
                .into_iter()
                .chain(booted.iter().filter_map(|o| o.booted_now()));
            projects.find_map(|b| b.semantics.features(language))
        };
        Ok(mapping::semantic::servers(&all, features))
    }

    /// The language's executable changed: its running servers restart in every project, and
    /// every connection of every project is told.
    async fn servers_replaced(&self, language: &str) {
        let language = BackendId(language.to_owned());
        for open in self.projects.all().await {
            if let Some(booted) = open.booted_now() {
                booted.semantics.restart(&language).await;
            }
        }
        for open in self.projects.all().await {
            open.subscribers.broadcast(Event::ServersChanged { project: open.id });
        }
    }
}
