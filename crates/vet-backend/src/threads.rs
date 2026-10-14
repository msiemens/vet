//! Requests about review threads.

use vet_core::review::ThreadId;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{ProjectId, Response};

use crate::Orchestrator;
use crate::announce::{announce_start, thread_changed, threads_changed};
use crate::error::Result;

impl Orchestrator {
    pub(crate) async fn threads(&self, project: ProjectId, view: vet_protocol::View) -> Result<Response> {
        let threads = self
            .asking(project, async |session| session.threads(mapping::core_view(view)).await)
            .await?;
        Ok(mapping::threads(&threads))
    }

    pub(crate) async fn comment_target(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        at: vet_protocol::CommentAt,
    ) -> Result<Response> {
        let spot = self
            .asking(project, async |session| {
                session
                    .comment_target(revision, mapping::core_view(view), path, mapping::core_comment_at(at))
                    .await
            })
            .await?;
        Ok(mapping::comment_spot(&spot))
    }

    pub(crate) async fn create_thread(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        target: vet_protocol::ThreadTarget,
        body: String,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let thread = open
            .session
            .create_thread(
                revision,
                mapping::core_view(view),
                mapping::new_thread(target),
                body,
                unix_now(),
            )
            .await?;
        announce_start(&open, thread.started);
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn reply(&self, project: ProjectId, thread: u32, body: String) -> Result<Response> {
        let open = self.ready(project).await?;
        let thread = open.session.reply(ThreadId(thread.into()), body, unix_now()).await?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn set_thread_status(
        &self,
        project: ProjectId,
        thread: u32,
        status: vet_protocol::ThreadStatus,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let thread = open
            .session
            .set_thread_status(ThreadId(thread.into()), mapping::core_thread_status(status))
            .await?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn delete_thread(&self, project: ProjectId, thread: u32) -> Result<Response> {
        let open = self.ready(project).await?;
        open.session
            .delete_thread(&open.hosting().await, ThreadId(thread.into()))
            .await?;
        threads_changed(&open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn confirm_anchor(&self, project: ProjectId, revision: u32, thread: u32) -> Result<Response> {
        let open = self.ready(project).await?;
        let thread = open.session.confirm_anchor(revision, ThreadId(thread.into())).await?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn attach_anchor(
        &self,
        project: ProjectId,
        revision: u32,
        thread: u32,
        candidate: u32,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let thread = open
            .session
            .attach_anchor(revision, ThreadId(thread.into()), candidate as usize)
            .await?;
        Ok(thread_changed(&open, &thread).await)
    }
}
