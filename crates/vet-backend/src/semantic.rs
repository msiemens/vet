//! Semantic requests (§6.3) of a session. The use cases are in
//! `vet_app::semantics`; this maps requests to them and their results to the
//! protocol, turning a language without a usable server into `unavailable`.

use vet_app::semantics::Answer;
use vet_app::session::{SessionError, View};
use vet_core::content::RepoPath;
use vet_infra::language;
use vet_protocol::{Response, Side};
use vet_semantic::ports::{At, BackendId, Location};
use vet_semantic::query::SemanticError;

use vet_mapping::semantic as map;
use vet_mapping::{core_side, core_view};

use super::error::{Error, Result};
use super::highlight;
use super::highlight::Sources;
use super::open::Booted;

/// Why an answer of the server of `language` may lack what the server had not got to yet.
fn incomplete(language: &Option<String>) -> vet_protocol::Unavailable {
    let server = language
        .as_deref()
        .and_then(language::by_id)
        .map_or("The language server", |l| l.display_name);
    map::unavailable(
        &SemanticError::Incomplete {
            server: server.to_owned(),
        },
        language.clone(),
    )
}

impl Booted {
    /// The language that serves `file`, as `ListServers` names it.
    pub(crate) fn language_of(&self, file: &RepoPath) -> Option<String> {
        self.semantics.language_of(file).map(|language| language.0.clone())
    }

    /// Starts the servers the current revision's changed files and the
    /// repository's languages need. A server that fails to start is reported
    /// by the query that needs it; a start that can't be tried is logged.
    pub(crate) async fn start_servers(&self) {
        let revision = self.session.revision().await;
        match self.semantics.start_servers(&self.view, revision).await {
            Err(SessionError::Stale { .. }) | Ok(()) => {}
            Err(error) => {
                tracing::warn!(%error, revision, "starting the servers of the revision on view failed");
            }
        }
    }

    /// Stops the servers that run under a policy the current revision no longer has. If that
    /// can't be told, every server that may execute project code is stopped and the error logged.
    pub(crate) async fn retire_disallowed_servers(&self) {
        let revision = self.session.revision().await;
        match self.semantics.retire_disallowed(&self.view, revision).await {
            Err(SessionError::Stale { .. }) | Ok(()) => {}
            Err(error) => {
                tracing::warn!(
                    %error, revision,
                    "retiring the servers a policy no longer allows failed; \
                     every server that may execute project code was stopped"
                );
            }
        }
    }

    /// Replaces the servers of `language` and waits until the ones the revision needs run.
    pub(crate) async fn restart_server(&self, language: &BackendId) -> Result<()> {
        let revision = self.session.revision().await;
        (self.semantics)
            .restart_now(&self.view, revision, language)
            .await?
            .map_err(|e| Error::Unavailable(e.to_string()))
    }

    pub(crate) async fn hover(&self, revision: u32, side: Side, path: &str, line: u32, col: u32) -> Result<Response> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .hover(&self.view, revision, core_side(side), &at)
            .await?;
        let (text, unavailable) = map::answer(answer.result, self.language_of(&at.file));
        Ok(highlight::hover(path, revision, side, text, unavailable))
    }

    fn locations(
        &self,
        answer: vet_app::semantics::Answer<Vec<Location>>,
        side: Side,
        at: &At,
    ) -> vet_protocol::Locations {
        let (found, unavailable) = map::answer(answer.result, self.language_of(&at.file));
        vet_protocol::Locations {
            revision: answer.revision,
            side,
            locations: map::locations(&found),
            unavailable,
        }
    }

    pub(crate) async fn definition(
        &self,
        revision: u32,
        side: Side,
        path: &str,
        line: u32,
        col: u32,
    ) -> Result<Response> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .definition(&self.view, revision, core_side(side), &at)
            .await?;
        Ok(Response::Definition(self.locations(answer, side, &at)))
    }

    pub(crate) async fn workspace_symbols(&self, revision: u32, query: &str) -> Result<Response> {
        let found = (self.semantics).workspace_symbols(&self.view, revision, query).await?;
        Ok(Response::WorkspaceSymbols(map::workspace_symbols(&found)))
    }

    pub(crate) async fn references(
        &self,
        sources: &Sources,
        revision: u32,
        side: Side,
        (path, line, col): (&str, u32, u32),
        limit: Option<u32>,
    ) -> Result<Response> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .references(&self.view, revision, core_side(side), &at)
            .await?;
        Ok(Response::References(map::location_rows(
            &(self.semantics)
                .rows(&self.view, core_side(side), &answer, &at, limit, sources)
                .await?,
            side,
            answer
                .result
                .as_ref()
                .err()
                .map(|e| map::unavailable(e, self.language_of(&at.file))),
        )))
    }

    pub(crate) async fn implementations(
        &self,
        sources: &Sources,
        revision: u32,
        side: Side,
        path: &str,
        line: u32,
        col: u32,
    ) -> Result<Response> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .implementations(&self.view, revision, core_side(side), &at)
            .await?;
        Ok(Response::Implementations(map::location_rows(
            &(self.semantics)
                .rows(&self.view, core_side(side), &answer, &at, None, sources)
                .await?,
            side,
            answer
                .result
                .as_ref()
                .err()
                .map(|e| map::unavailable(e, self.language_of(&at.file))),
        )))
    }

    pub(crate) async fn diagnostics(&self, revision: u32, path: &str, against_base: bool) -> Result<Response> {
        let file = RepoPath::from(path);
        let language = self.language_of(&file);
        if !against_base {
            let answer = (self.semantics).diagnostics(&self.view, revision, &file).await?;
            let incomplete = (answer.result.is_ok() && !answer.complete).then(|| incomplete(&language));
            let (found, unavailable) = map::answer(answer.result, language);
            return Ok(Response::Diagnostics {
                revision,
                path: path.to_owned(),
                diagnostics: map::pending_diagnostics(&found),
                count: vet_app::semantics::countable(&found),
                unavailable,
                origin_unavailable: None,
                incomplete,
            });
        }
        let answer = (self.semantics).judged_diagnostics(&self.view, revision, &file).await?;
        let (diagnostics, count, unavailable, origin_unavailable, incomplete) = match answer.result {
            Ok(judgement) => {
                let found: Vec<_> = judgement.diagnostics.iter().map(|j| j.diagnostic.clone()).collect();
                let waiting = !judgement.complete || matches!(judgement.base, Some(SemanticError::Incomplete { .. }));
                (
                    map::judged_diagnostics(&judgement.diagnostics),
                    vet_app::semantics::countable(&found),
                    None,
                    judgement.base.map(|e| map::unavailable(&e, language.clone())),
                    waiting.then(|| incomplete(&language)),
                )
            }
            Err(e) => (Vec::new(), 0, Some(map::unavailable(&e, language)), None, None),
        };
        Ok(Response::Diagnostics {
            revision,
            path: path.to_owned(),
            diagnostics,
            count,
            unavailable,
            origin_unavailable,
            incomplete,
        })
    }

    pub(crate) async fn file_symbols(&self, revision: u32, path: &str) -> Result<Response> {
        let file = RepoPath::from(path);
        let found = (self.semantics).file_symbols(&self.view, revision, &file).await?;
        Ok(Response::FileSymbols(map::file_symbols(
            revision,
            path,
            &found.changes,
            found.gap.as_ref(),
        )))
    }

    pub(crate) async fn file_outline(&self, view: vet_protocol::View, revision: u32, path: &str) -> Result<Response> {
        let file = RepoPath::from(path);
        let answer = match core_view(view) {
            View::Changeset => (self.semantics).file_outline(&self.view, revision, &file).await?,
            // The semantic layer holds the changeset's snapshots, not a commit's: such a view has
            // the outline of its text, with no marks.
            other => {
                if self.semantics.language_of(&file).is_none() {
                    Answer::derived(revision, Err(SemanticError::NoBackend(file.clone())))
                } else {
                    let text = self.session.target_text(other, path).await?;
                    if text.revision != revision {
                        return Err(SessionError::Stale { current: text.revision }.into());
                    }
                    match text.content {
                        Ok(text) => {
                            (self.semantics)
                                .text_outline(&self.view, revision, &file, &text)
                                .await?
                        }
                        // No text to read symbols from: the file is not there at this commit.
                        Err(_) => Answer::derived(revision, Ok(Vec::new())),
                    }
                }
            }
        };
        let (found, unavailable) = map::answer(answer.result, self.language_of(&file));
        Ok(Response::FileOutline(map::file_outline(
            revision,
            path,
            &found,
            unavailable,
        )))
    }

    pub(crate) async fn symbol_at(
        &self,
        revision: u32,
        side: Side,
        path: &str,
        line: u32,
        col: u32,
    ) -> Result<Response> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .symbol_at(&self.view, revision, core_side(side), &at)
            .await?;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&at.file));
        Ok(Response::SymbolAt(map::symbol_at(
            revision,
            found.as_ref(),
            unavailable,
        )))
    }

    pub(crate) async fn calls(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response> {
        let (info, calls) = (self.semantics)
            .calls(&self.view, revision, &map::core_sym(symbol))
            .await?;
        Ok(Response::Calls(map::calls(revision, &info, &calls)))
    }

    pub(crate) async fn usages(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response> {
        let (info, usages) = (self.semantics)
            .usages(&self.view, revision, &map::core_sym(symbol))
            .await?;
        Ok(Response::Usages(map::usages(revision, &info, &usages)))
    }

    pub(crate) async fn signature(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response> {
        let answer = (self.semantics)
            .signature(&self.view, revision, &map::core_sym(symbol))
            .await?;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&RepoPath::from(symbol.path.as_str())));
        Ok(highlight::signature(&symbol.path, revision, found, unavailable))
    }

    pub(crate) async fn removal_impact(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response> {
        let impact = (self.semantics)
            .removal_impact(&self.view, revision, &map::core_sym(symbol))
            .await?;
        Ok(Response::RemovalImpact(map::removal_impact(revision, &impact)))
    }

    pub(crate) async fn reachability(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response> {
        let (info, reach) = (self.semantics)
            .reachability(&self.view, revision, &map::core_sym(symbol))
            .await?;
        Ok(Response::Reachability(map::reachability(revision, &info, &reach)))
    }
}

#[cfg(test)]
mod tests {
    use vet_app::ports::LineSources;
    use vet_core::content::RepoPath;
    use vet_infra::budget::Budget;
    use vet_infra::language;

    use super::highlight::Sources;

    /// A file a language server named outside the repository is read only where it is a source
    /// of the answerer's language, and never one too large to show.
    #[tokio::test]
    async fn a_source_of_the_answerers_language_is_read_from_outside_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, "secret = 1\n").unwrap();
            RepoPath::from(path.to_string_lossy().as_ref())
        };
        let reads = Sources::new(super::highlight::Highlighter::new(Budget::new(1)));
        for (answerer, source) in [
            ("rust", "lib.rs"),
            ("typescript", "index.d.ts"),
            ("typescript", "index.js"),
            ("svelte", "index.d.ts"),
            ("python", "typing.pyi"),
            ("dockerfile", "Dockerfile"),
        ] {
            assert!(language::is_source_of(answerer, &file(source)), "{source}");
            assert_eq!(
                reads.outside(&file(source), u64::MAX).await.as_deref(),
                Some("secret = 1\n"),
                "{source}"
            );
        }
        for (answerer, other) in [
            ("rust", "config"),
            ("rust", "hosts.yml"),
            ("typescript", "secrets.json"),
            ("typescript", "lib.rs"),
            ("python", ".env"),
            ("python", "id_rsa"),
        ] {
            assert!(!language::is_source_of(answerer, &file(other)), "{other}");
        }
        // Nothing over the byte cap asked for is read.
        assert_eq!(reads.outside(&file("lib.rs"), 5).await.as_deref(), None);
    }
}
