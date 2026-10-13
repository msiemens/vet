//! Requests about a project's review: its changeset, files, marks, base and
//! filter. The rules are in `vet_app::session`; this maps requests to them and
//! tells every subscriber what changed.

use vet_app::changeset::Source;
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};

use crate::announce::{announce_start, review_changed};
use crate::events::settled;
use crate::open::Ready;
use crate::{Orchestrator, message, start_servers_in_background};

impl Orchestrator {
    pub(crate) async fn finish_review(&self, project: ProjectId) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let complete = open.session.finish_review().await.map_err(message)?;
        let revision = open.session.revision().await;
        settled(&open.subscribers, project, revision);
        open.subscribers.broadcast(Event::ChangesetsChanged { project });
        Ok(Response::ReviewFinished { complete })
    }

    pub(crate) async fn versions(&self, project: ProjectId) -> Result<Response, String> {
        let versions = self.ready(project).await?.session.versions().await.map_err(message)?;
        Ok(mapping::versions(&versions))
    }

    pub(crate) async fn commits(&self, project: ProjectId) -> Result<Response, String> {
        let list = self.ready(project).await?.session.commits().await.map_err(message)?;
        Ok(mapping::commits(&list))
    }

    pub(crate) async fn set_lens(&self, project: ProjectId, lens: vet_protocol::Lens) -> Result<Response, String> {
        let open = self.ready(project).await?;
        open.session.set_lens(mapping::core_lens(lens)).await.map_err(message)?;
        open.subscribers.broadcast(Event::LensChanged { project, lens });
        Ok(Response::Done)
    }

    pub(crate) async fn manifest(&self, project: ProjectId, view: vet_protocol::View) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let view = open.session.manifest(mapping::core_view(view)).await.map_err(message)?;
        let failures = open.semantics.restore_failures();
        let (trust, head) = open.session.trust().await.map_err(message)?;
        let mut manifest = mapping::manifest(project, &view, &failures, mapping::analysis_trust(&trust, &head));
        if matches!(view.target.source, Source::PullRequest { .. })
            && let Some(head) = open
                .session
                .pull_request_head(&view.target.source)
                .await
                .map_err(message)?
        {
            manifest.target.title = Some(head.title);
            manifest.target.draft = head.draft;
        }
        Ok(Response::Manifest(Box::new(manifest)))
    }

    pub(crate) async fn file_contents(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        path: &str,
    ) -> Result<Response, String> {
        let text = self
            .ready(project)
            .await?
            .session
            .file_text(mapping::core_view(view), path)
            .await
            .map_err(message)?;
        Ok(self.highlighter.file_contents(path, text).await)
    }

    pub(crate) async fn file_text(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        path: &str,
    ) -> Result<Response, String> {
        let text = self
            .ready(project)
            .await?
            .session
            .target_text(mapping::core_view(view), path)
            .await
            .map_err(message)?;
        Ok(self.highlighter.file_text(path, &text).await)
    }

    pub(crate) async fn toggle_hunk(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        hunk: u32,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = open
            .session
            .toggle_hunk(revision, view, path, hunk)
            .await
            .map_err(message)?;
        announce_start(&open, update.started);
        Ok(Response::Review(review_changed(&open, view, &update).await))
    }

    pub(crate) async fn toggle_flag(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        hunk: u32,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = open
            .session
            .toggle_flag(revision, view, path, hunk)
            .await
            .map_err(message)?;
        announce_start(&open, update.started);
        Ok(Response::Review(review_changed(&open, view, &update).await))
    }

    pub(crate) async fn toggle_item(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        kind: vet_protocol::NonTextKind,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = open
            .session
            .toggle_item(revision, view, path, mapping::core_non_text_kind(kind))
            .await
            .map_err(message)?;
        announce_start(&open, update.started);
        Ok(Response::Review(review_changed(&open, view, &update).await))
    }

    pub(crate) async fn toggle_file_seen(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = open
            .session
            .toggle_file_seen(revision, view, path)
            .await
            .map_err(message)?;
        announce_start(&open, update.started);
        Ok(Response::Review(review_changed(&open, view, &update).await))
    }

    pub(crate) async fn set_files_seen(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        paths: &[String],
        seen: bool,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = open
            .session
            .set_files_seen(revision, view, paths, seen)
            .await
            .map_err(message)?;
        announce_start(&open, update.started);
        let mut answers = Vec::with_capacity(update.len());
        for file in &update.value {
            answers.push(review_changed(&open, view, file).await);
        }
        Ok(Response::Reviews { updates: answers })
    }

    pub(crate) async fn reconfirm_file(
        &self,
        project: ProjectId,
        revision: u32,
        path: &str,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let update = open.session.reconfirm_file(revision, path).await.map_err(message)?;
        Ok(Response::Review(
            review_changed(&open, vet_app::session::View::Changeset, &update).await,
        ))
    }

    pub(crate) async fn base_diff(
        &self,
        project: ProjectId,
        revision: u32,
        path: &str,
        hunk: u32,
    ) -> Result<Response, String> {
        let diff = self
            .ready(project)
            .await?
            .session
            .base_diff(revision, path, hunk)
            .await
            .map_err(message)?;
        Ok(mapping::base_diff(&diff))
    }

    pub(crate) async fn item_blob(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        kind: vet_protocol::NonTextKind,
        side: vet_protocol::Side,
    ) -> Result<Response, String> {
        let blob = self
            .ready(project)
            .await?
            .session
            .item_blob(
                revision,
                mapping::core_view(view),
                path,
                mapping::core_non_text_kind(kind),
                mapping::core_side(side),
            )
            .await
            .map_err(message)?;
        Ok(mapping::item_blob(blob))
    }

    pub(crate) async fn set_project_filter(
        &self,
        project: ProjectId,
        filter: vet_protocol::ReviewFilter,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let revision = open
            .session
            .set_filter(mapping::core_filter(filter))
            .await
            .map_err(message)?;
        revision_changed(open, revision);
        Ok(Response::Done)
    }

    pub(crate) async fn exclude_path(
        &self,
        project: ProjectId,
        path: &str,
        directory: bool,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let revision = open.session.exclude_path(path, directory).await.map_err(message)?;
        settled(&open.subscribers, project, revision);
        Ok(Response::Done)
    }

    pub(crate) async fn list_directory(&self, project: ProjectId, path: &str) -> Result<Response, String> {
        let listing = self
            .ready(project)
            .await?
            .session
            .list_directory(path)
            .await
            .map_err(message)?;
        Ok(mapping::directory(&listing))
    }

    pub(crate) async fn target_line(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        side: vet_protocol::Side,
        path: &str,
        line: u32,
    ) -> Result<Response, String> {
        let line = self
            .ready(project)
            .await?
            .session
            .line_in_target(revision, mapping::core_view(view), mapping::core_side(side), path, line)
            .await
            .map_err(message)?;
        Ok(Response::TargetLine { revision, line })
    }
}

/// A request changed the revision for everyone: they refetch, and the language
/// servers start for it in the background.
fn revision_changed(ready: Ready, revision: u32) {
    settled(&ready.subscribers, ready.id, revision);
    start_servers_in_background(ready.open);
}
