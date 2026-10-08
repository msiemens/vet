//! Requests about the language servers, which are shared by all projects.

use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};
use vet_semantic::ports::BackendId;

use crate::{Open, Orchestrator, message};

impl Orchestrator {
    pub(crate) async fn list_servers(&self, project: ProjectId) -> Result<Response, String> {
        let open = self.find(project).await?;
        self.servers_of(&open).await
    }

    pub(crate) async fn install_server(&self, project: ProjectId, language: String) -> Result<Response, String> {
        let open = self.find(project).await?;
        let progress = |line: &str| {
            open.subscribers.broadcast(Event::ServerInstallOutput {
                project,
                language: language.clone(),
                line: line.to_owned(),
            });
        };
        self.servers.install(&language, &progress).await.map_err(message)?;
        self.servers_replaced(&language).await;
        self.servers_of(&open).await
    }

    pub(crate) async fn set_server_path(
        &self,
        project: ProjectId,
        language: String,
        path: Option<String>,
    ) -> Result<Response, String> {
        let open = self.find(project).await?;
        self.servers.set_custom_path(&language, path).await.map_err(message)?;
        self.servers_replaced(&language).await;
        self.servers_of(&open).await
    }

    async fn servers_of(&self, open: &Open) -> Result<Response, String> {
        let all = self.servers.status(&open.repo).await.map_err(message)?;
        Ok(mapping::semantic::servers(&all))
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
