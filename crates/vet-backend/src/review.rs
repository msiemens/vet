//! Requests about a project's review: its changeset, files, marks, base and
//! filter. The rules are in `vet_app::session`; this maps requests to them and
//! tells every subscriber what changed.

use std::sync::Arc;

use vet_app::search::SearchFilters;
use vet_app::session::{Outcome, ReviewUpdateView, SearchQuery, SessionError, View};
use vet_core::filter::FileMask;
use vet_infra::search::RegexMatcher;
use vet_mapping as mapping;
use vet_protocol::{Event, ProjectId, Response};

use crate::announce::{announce_start, review_changed};
use crate::error::{Error, Result};
use crate::events::{filter_changed, review_finished};
use crate::{Live, Orchestrator};

impl Orchestrator {
    pub(crate) async fn finish_review(&self, project: ProjectId) -> Result<Response> {
        let open = self.ready(project).await?;
        let complete = open.session.finish_review().await?;
        let revision = open.session.revision().await;
        review_finished(&open.open, revision);
        // The trees only the finished review needed.
        crate::open::collect_trees(&open.open.trees).await;
        Ok(Response::ReviewFinished { complete })
    }

    pub(crate) async fn versions(&self, project: ProjectId) -> Result<Response> {
        let versions = self.asking(project, async |session| session.versions().await).await?;
        Ok(mapping::versions(&versions))
    }

    pub(crate) async fn changed_since(&self, project: ProjectId, from: u32) -> Result<Response> {
        let changed = self
            .asking(project, async |session| session.changed_since(from).await)
            .await?;
        Ok(mapping::changed_files(&changed))
    }

    pub(crate) async fn commits(&self, project: ProjectId) -> Result<Response> {
        let list = self.asking(project, async |session| session.commits().await).await?;
        Ok(mapping::commits(&list))
    }

    pub(crate) async fn set_lens(&self, project: ProjectId, lens: vet_protocol::Lens) -> Result<Response> {
        let open = self.ready(project).await?;
        open.session.set_lens(mapping::core_lens(lens)).await?;
        open.subscribers.broadcast(Event::LensChanged { project, lens });
        Ok(Response::Done)
    }

    pub(crate) async fn manifest(&self, project: ProjectId, view: vet_protocol::View) -> Result<Response> {
        let open = self.ready(project).await?;
        let view = open.session.manifest(mapping::core_view(view)).await?;
        let failures = open.semantics.restore_failures();
        let (trust, head) = open.session.trust().await?;
        let manifest = mapping::manifest(project, &view, &failures, mapping::analysis_trust(&trust, &head));
        Ok(Response::Manifest(Box::new(manifest)))
    }

    pub(crate) async fn file_contents(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        path: &str,
    ) -> Result<Response> {
        let text = self
            .asking(project, async |session| {
                session.file_text(mapping::core_view(view), path).await
            })
            .await?;
        Ok(self.highlighter.file_contents(path, text).await)
    }

    pub(crate) async fn file_text(&self, project: ProjectId, view: vet_protocol::View, path: &str) -> Result<Response> {
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
    ) -> Result<Response> {
        let matcher = RegexMatcher::compile(&mapping::core_search_pattern(pattern)).map_err(Error::Invalid)?;
        let mask = FileMask::parse(mask).map_err(Error::Invalid)?;
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
        mark: impl AsyncFnOnce(&Live, View) -> std::result::Result<Outcome<ReviewUpdateView>, SessionError>,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = mark(&open.session, view).await?;
        announce_start(&open, update.started);
        Ok(Response::Review(
            review_changed(&open, view, &update, update.reviewed_at).await,
        ))
    }

    pub(crate) async fn toggle_hunk(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        hunk: u32,
    ) -> Result<Response> {
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
    ) -> Result<Response> {
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
    ) -> Result<Response> {
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
    ) -> Result<Response> {
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
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let view = mapping::core_view(view);
        let update = open.session.set_files_seen(revision, view, paths, seen).await?;
        announce_start(&open, update.started);
        let mut answers = Vec::with_capacity(update.len());
        for file in &update.value {
            answers.push(review_changed(&open, view, file, update.reviewed_at).await);
        }
        Ok(Response::Reviews { updates: answers })
    }

    pub(crate) async fn reconfirm_file(&self, project: ProjectId, revision: u32, path: &str) -> Result<Response> {
        let open = self.ready(project).await?;
        let update = open.session.reconfirm_file(revision, path).await?;
        Ok(Response::Review(
            review_changed(&open, vet_app::session::View::Changeset, &update, update.reviewed_at).await,
        ))
    }

    pub(crate) async fn base_diff(&self, project: ProjectId, revision: u32, path: &str, hunk: u32) -> Result<Response> {
        let diff = self
            .asking(project, async |session| session.base_diff(revision, path, hunk).await)
            .await?;
        Ok(mapping::base_diff(revision, &diff))
    }

    pub(crate) async fn item_blob(
        &self,
        project: ProjectId,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
        kind: vet_protocol::NonTextKind,
        side: vet_protocol::Side,
    ) -> Result<Response> {
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
        Ok(mapping::item_blob(revision, blob))
    }

    pub(crate) async fn set_project_filter(
        &self,
        project: ProjectId,
        filter: vet_protocol::ReviewFilter,
    ) -> Result<Response> {
        let open = self.ready(project).await?;
        let revision = open.session.set_filter(mapping::core_filter(filter)).await?;
        filter_changed(&open.open, revision);
        Ok(Response::Done)
    }

    pub(crate) async fn pattern_matches(
        &self,
        project: ProjectId,
        filter: vet_protocol::ReviewFilter,
    ) -> Result<Response> {
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
    ) -> Result<Response> {
        let source = mapping::core_changeset_key(changeset).map_err(Error::Invalid)?;
        let open = self.ready(project).await?;
        let revision = open.session.narrow(&source, filter.map(mapping::core_filter)).await?;
        if let Some(revision) = revision {
            filter_changed(&open.open, revision);
        }
        Ok(Response::Done)
    }

    pub(crate) async fn exclude_path(&self, project: ProjectId, path: &str, directory: bool) -> Result<Response> {
        let open = self.ready(project).await?;
        let revision = open.session.exclude_path(path, directory).await?;
        filter_changed(&open.open, revision);
        Ok(Response::Done)
    }

    pub(crate) async fn list_directory(&self, project: ProjectId, path: &str) -> Result<Response> {
        let listing = self
            .asking(project, async |session| session.list_directory(path).await)
            .await?;
        Ok(mapping::directory(&listing))
    }

    pub(crate) async fn list_files(&self, project: ProjectId) -> Result<Response> {
        let listing = self.asking(project, async |session| session.list_files().await).await?;
        Ok(mapping::files(&listing))
    }

    pub(crate) async fn map_lines(
        &self,
        project: ProjectId,
        revision: u32,
        from: u32,
        path: &str,
        lines: &[u32],
    ) -> Result<Response> {
        let lines = self
            .asking(project, async |session| {
                session.map_lines(revision, from, path, lines).await
            })
            .await?;
        Ok(Response::MappedLines {
            revision,
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
    ) -> Result<Response> {
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
