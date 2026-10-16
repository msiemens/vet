//! Requests about trust (§6.5): what analysis of a project's changesets may execute.

use vet_protocol::{Event, ProjectId, Response};

use crate::Orchestrator;
use crate::error::Result;
use crate::open::Ready;

impl Orchestrator {
    pub(crate) async fn set_project_trusted(&self, project: ProjectId, trusted: bool) -> Result<Response> {
        let open = self.ready(project).await?;
        open.session.set_project_trusted(trusted).await?;
        trust_changed(open);
        Ok(Response::Done)
    }

    pub(crate) async fn trust_pull_request(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        open.session.trust_pull_request().await?;
        trust_changed(open);
        Ok(Response::Done)
    }
}

/// The policy of the servers and restores changed: the manifest says so, what the servers
/// answer is asked again, and the supervisor stops the servers that run under a policy no longer
/// allowed and starts the ones the revision needs under the new policy.
fn trust_changed(ready: Ready) {
    ready.subscribers.broadcast(Event::TargetChanged { project: ready.id });
    ready.subscribers.broadcast(Event::ServersChanged { project: ready.id });
    ready.open.policy.policy_changed();
}
