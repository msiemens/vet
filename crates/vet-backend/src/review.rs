//! Requests about a project's review: its changeset, files, marks, base and
//! filter. The rules are in `vet_app::session`; this maps requests to them and
//! tells the other subscribers what changed.

use vet_mapping as mapping;
use vet_protocol::{ProjectId, Response};

use crate::events::{Connection, announce_start, review_changed, settled};
use crate::open::Ready;
use crate::{Orchestrator, message, start_servers_in_background};

impl Orchestrator {
    pub(crate) async fn finish_review(&self, project: ProjectId) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let complete = open.session.finish_review().await.map_err(message)?;
        let revision = open.session.revision().await;
        settled(&open.subscribers, project, revision);
        Ok(Response::ReviewFinished { complete })
    }

    pub(crate) async fn versions(&self, project: ProjectId) -> Result<Response, String> {
        let versions = self.ready(project).await?.session.versions().await.map_err(message)?;
        Ok(mapping::versions(&versions))
    }

    pub(crate) async fn manifest(&self, project: ProjectId, view: vet_protocol::View) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let manifest = open.session.manifest(mapping::core_view(view)).await.map_err(message)?;
        let failures = open.semantics.restore_failures();
        Ok(Response::Manifest(Box::new(mapping::manifest(
            project, &manifest, &failures,
        ))))
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

    pub(crate) async fn file_text(&self, project: ProjectId, path: &str) -> Result<Response, String> {
        let text = self
            .ready(project)
            .await?
            .session
            .target_text(path)
            .await
            .map_err(message)?;
        Ok(self.highlighter.file_text(path, &text).await)
    }

    pub(crate) async fn toggle_hunk(
        &self,
        project: ProjectId,
        from: &Connection,
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
        Ok(Response::Review(review_changed(&open, from, view, &update).await))
    }

    pub(crate) async fn toggle_item(
        &self,
        project: ProjectId,
        from: &Connection,
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
        Ok(Response::Review(review_changed(&open, from, view, &update).await))
    }

    pub(crate) async fn toggle_file_seen(
        &self,
        project: ProjectId,
        from: &Connection,
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
        Ok(Response::Review(review_changed(&open, from, view, &update).await))
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

    pub(crate) async fn set_base(
        &self,
        project: ProjectId,
        base: vet_protocol::BaseChoice,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let (base, revision) = open
            .session
            .set_base(mapping::core_base_choice(base))
            .await
            .map_err(message)?;
        revision_changed(open, revision);
        Ok(Response::Base(mapping::base(&base)))
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

    pub(crate) async fn list_base_candidates(&self, project: ProjectId) -> Result<Response, String> {
        let candidates = self
            .ready(project)
            .await?
            .session
            .base_candidates()
            .await
            .map_err(message)?;
        Ok(mapping::base_candidates(&candidates))
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
