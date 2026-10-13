//! Requests about a project's review: its changeset, files, marks, base and
//! filter. The rules are in `vet_app::session`; this maps requests to them and
//! tells every subscriber what changed.

use std::sync::Arc;

use vet_app::changeset::Source;
use vet_app::search::SearchFilters;
use vet_app::session::{Outcome, ReviewUpdateView, SearchQuery, SessionError, View};
use vet_core::filter::FileMask;
use vet_infra::search::RegexMatcher;
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};

use crate::announce::{announce_start, review_changed};
use crate::events::settled;
use crate::open::Ready;
use crate::{Live, Orchestrator, message, start_servers_in_background};

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
        let versions = self.asking(project, async |session| session.versions().await).await?;
        Ok(mapping::versions(&versions))
    }

    pub(crate) async fn commits(&self, project: ProjectId) -> Result<Response, String> {
        let list = self.asking(project, async |session| session.commits().await).await?;
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
            .asking(project, async |session| {
                session.file_text(mapping::core_view(view), path).await
            })
            .await?;
        Ok(self.highlighter.file_contents(path, text).await)
    }

    pub(crate) async fn file_text(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        path: &str,
    ) -> Result<Response, String> {
        let text = self
            .asking(project, async |session| {
                session.target_text(mapping::core_view(view), path).await
            })
            .await?;
        Ok(self.highlighter.file_text(path, &text).await)
    }

    pub(crate) async fn search(
        &self,
        project: ProjectId,
        scope: vet_protocol::SearchScope,
        pattern: vet_protocol::SearchPattern,
        mask: &str,
        filters: SearchFilters,
        limit: u32,
    ) -> Result<Response, String> {
        let matcher = RegexMatcher::compile(&mapping::core_search_pattern(pattern))?;
        let mask = FileMask::parse(mask)?;
        let query = SearchQuery {
            scope: mapping::core_search_scope(scope),
            matcher: Arc::new(matcher),
            mask: &mask,
            filters,
            limit: limit as usize,
        };
        let result = self
            .asking(project, async |session| session.search(&query).await)
            .await?;
        Ok(mapping::search_results(&result))
    }

    /// Makes a mark in `view` with `mark`, and tells the subscribers what it changed.
    async fn marked(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        mark: impl AsyncFnOnce(&Live, View) -> Result<Outcome<ReviewUpdateView>, SessionError>,
    ) -> Result<Response, String> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = mark(&open.session, view).await.map_err(message)?;
        announce_start(&open, update.started);
        Ok(Response::Review(review_changed(&open, view, &update).await))
    }

    pub(crate) async fn toggle_hunk(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        hunk: u32,
    ) -> Result<Response, String> {
        self.marked(project, view, async |session, view| {
            session.toggle_hunk(revision, view, path, hunk).await
        })
        .await
    }

    pub(crate) async fn toggle_flag(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        hunk: u32,
    ) -> Result<Response, String> {
        self.marked(project, view, async |session, view| {
            session.toggle_flag(revision, view, path, hunk).await
        })
        .await
    }

    pub(crate) async fn toggle_item(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        kind: vet_protocol::NonTextKind,
    ) -> Result<Response, String> {
        self.marked(project, view, async |session, view| {
            session
                .toggle_item(revision, view, path, mapping::core_non_text_kind(kind))
                .await
        })
        .await
    }

    pub(crate) async fn toggle_file_seen(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
    ) -> Result<Response, String> {
        self.marked(project, view, async |session, view| {
            session.toggle_file_seen(revision, view, path).await
        })
        .await
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
            .asking(project, async |session| session.base_diff(revision, path, hunk).await)
            .await?;
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
            .asking(project, async |session| {
                session
                    .item_blob(
                        revision,
                        mapping::core_view(view),
                        path,
                        mapping::core_non_text_kind(kind),
                        mapping::core_side(side),
                    )
                    .await
            })
            .await?;
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

    pub(crate) async fn pattern_matches(
        &self,
        project: ProjectId,
        filter: vet_protocol::ReviewFilter,
    ) -> Result<Response, String> {
        let filter = mapping::core_filter(filter);
        let matches = self
            .asking(project, async |session| session.pattern_matches(&filter).await)
            .await?;
        Ok(mapping::pattern_matches(&matches))
    }

    pub(crate) async fn narrow_review(
        &self,
        project: ProjectId,
        changeset: vet_protocol::ChangesetKey,
        filter: Option<vet_protocol::ReviewFilter>,
    ) -> Result<Response, String> {
        let source = mapping::core_changeset_key(changeset)?;
        let open = self.ready(project).await?;
        let revision = open
            .session
            .narrow(&source, filter.map(mapping::core_filter))
            .await
            .map_err(message)?;
        if let Some(revision) = revision {
            settled(&open.subscribers, project, revision);
        }
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
            .asking(project, async |session| session.list_directory(path).await)
            .await?;
        Ok(mapping::directory(&listing))
    }

    pub(crate) async fn list_files(&self, project: ProjectId) -> Result<Response, String> {
        let listing = self.asking(project, async |session| session.list_files().await).await?;
        Ok(mapping::files(&listing))
    }

    pub(crate) async fn map_lines(
        &self,
        project: ProjectId,
        from: u32,
        path: &str,
        lines: &[u32],
    ) -> Result<Response, String> {
        let lines = self
            .asking(project, async |session| session.map_lines(from, path, lines).await)
            .await?;
        Ok(Response::MappedLines {
            lines: lines
                .into_iter()
                .map(|m| vet_protocol::MappedLine {
                    line: m.line,
                    kept: m.kept,
                })
                .collect(),
        })
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
            .asking(project, async |session| {
                session
                    .line_in_target(revision, mapping::core_view(view), mapping::core_side(side), path, line)
                    .await
            })
            .await?;
        Ok(Response::TargetLine { revision, line })
    }
}

/// A request changed the revision for everyone: they refetch, and the language
/// servers start for it in the background.
fn revision_changed(ready: Ready, revision: u32) {
    settled(&ready.subscribers, ready.id, revision);
    start_servers_in_background(ready.open);
}
