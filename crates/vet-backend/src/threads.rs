//! Requests about review threads.

use vet_core::review::ThreadId;
use vet_infra::clock::unix_now;
use vet_mapping as mapping;
use vet_protocol::{ProjectId, Response};

use crate::announce::{announce_start, thread_changed, threads_changed};
use crate::{Orchestrator, message};

impl Orchestrator {
    pub(crate) async fn threads(&self, project: ProjectId, view: vet_protocol::View) -> Result<Response, String> {
        let threads = (self.ready(project).await?.session)
            .threads(mapping::core_view(view))
            .await
            .map_err(message)?;
        Ok(mapping::threads(&threads))
    }

    pub(crate) async fn comment_target(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        at: vet_protocol::CommentAt,
    ) -> Result<Response, String> {
        let spot = (self.ready(project).await?.session)
            .comment_target(revision, mapping::core_view(view), path, mapping::core_comment_at(at))
            .await
            .map_err(message)?;
        Ok(mapping::comment_spot(&spot))
    }

    pub(crate) async fn create_thread(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        target: vet_protocol::ThreadTarget,
        body: String,
    ) -> Result<Response, String> {
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
            .await
            .map_err(message)?;
        announce_start(&open, thread.started);
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn reply(&self, project: ProjectId, thread: u32, body: String) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let thread = open
            .session
            .reply(ThreadId(thread.into()), body, unix_now())
            .await
            .map_err(message)?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn set_thread_status(
        &self,
        project: ProjectId,
        thread: u32,
        status: vet_protocol::ThreadStatus,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let thread = open
            .session
            .set_thread_status(ThreadId(thread.into()), mapping::core_thread_status(status))
            .await
            .map_err(message)?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn delete_thread(&self, project: ProjectId, thread: u32) -> Result<Response, String> {
        let open = self.ready(project).await?;
        open.session
            .delete_thread(&open.hosting().await, ThreadId(thread.into()))
            .await
            .map_err(message)?;
        threads_changed(&open).await;
        Ok(Response::Done)
    }

    pub(crate) async fn confirm_anchor(
        &self,
        project: ProjectId,
        revision: u32,
        thread: u32,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let thread = open
            .session
            .confirm_anchor(revision, ThreadId(thread.into()))
            .await
            .map_err(message)?;
        Ok(thread_changed(&open, &thread).await)
    }

    pub(crate) async fn attach_anchor(
        &self,
        project: ProjectId,
        revision: u32,
        thread: u32,
        candidate: u32,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let thread = open
            .session
            .attach_anchor(revision, ThreadId(thread.into()), candidate as usize)
            .await
            .map_err(message)?;
        Ok(thread_changed(&open, &thread).await)
    }
}
