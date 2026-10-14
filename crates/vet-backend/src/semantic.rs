//! Semantic requests (§7.3) of a session. The use cases are in
//! `vet_app::semantics`; this maps requests to them and their results to the
//! protocol, turning a language without a usable server into `unavailable`.

use std::collections::HashMap;

use vet_app::semantics::Answer;
use vet_app::session::{SessionError, View};
use vet_core::content::RepoPath;
use vet_core::text::Lines;
use vet_infra::language;
use vet_protocol::{Response, Side};
use vet_semantic::derived::Reference;
use vet_semantic::ports::{At, BackendId, Location};
use vet_semantic::query::SemanticError;

use vet_mapping::semantic as map;
use vet_mapping::{core_side, core_view};

use super::error::{Error, Result};
use super::highlight;
use super::highlight::Highlighter;
use super::open::Booted;

/// The largest file outside the repository whose lines are shown.
const MAX_OUTSIDE_FILE: u64 = 8 * 1024 * 1024;

/// The text of a file outside the repository (a dependency or toolchain source), which is
/// named by its absolute path; `None` if it cannot be read. A language server names the place,
/// and a repository can steer it, so only a source file of the language `answerer` that named
/// it is read: library and toolchain sources have the extension of their language, a key, a
/// dotfile or another tool's configuration does not.
async fn read_outside(answerer: Option<&str>, file: &RepoPath) -> Option<String> {
    if !answerer.is_some_and(|language| language::is_source_of(language, file)) {
        return None;
    }
    let path = std::path::PathBuf::from(file.to_string_lossy());
    if tokio::fs::metadata(&path).await.ok()?.len() > MAX_OUTSIDE_FILE {
        return None;
    }
    let bytes = tokio::fs::read(&path).await.ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

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

/// A file's text as the lines of a list of places need it.
struct FileLines {
    text: String,
    /// The highlighting of the whole text.
    highlights: Vec<u32>,
    /// The offset of each line's start, in UTF-16 units.
    starts: Vec<u32>,
}

impl FileLines {
    fn line_start(&self, line: u32) -> u32 {
        self.starts
            .get(line as usize)
            .copied()
            .unwrap_or_else(|| self.starts.last().copied().unwrap_or(0))
    }
}

impl Booted {
    /// The language that serves `file`, as `ListServers` names it.
    pub(crate) fn language_of(&self, file: &RepoPath) -> Option<String> {
        self.semantics.language_of(file).map(|language| language.0.clone())
    }

    /// Starts the servers the current revision's changed files and the
    /// repository's languages need. A server that fails to start is reported
    /// by the query that needs it.
    pub(crate) async fn start_servers(&self) {
        let revision = self.session.revision().await;
        let _ = self.semantics.start_servers(&self.view, revision).await;
    }

    /// Stops the servers that run under a policy the current revision no longer has.
    pub(crate) async fn retire_disallowed_servers(&self) {
        let revision = self.session.revision().await;
        let _ = self.semantics.retire_disallowed(&self.view, revision).await;
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
        highlighter: &Highlighter,
        revision: u32,
        side: Side,
        (path, line, col): (&str, u32, u32),
        limit: Option<u32>,
    ) -> Result<Response> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .references(&self.view, revision, core_side(side), &at)
            .await?;
        let limit = limit.map_or(map::MAX_ROWS, |l| l as usize);
        Ok(Response::References(
            self.rows(highlighter, answer, side, &at, limit).await?,
        ))
    }

    pub(crate) async fn implementations(
        &self,
        highlighter: &Highlighter,
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
        Ok(Response::Implementations(
            self.rows(highlighter, answer, side, &at, map::MAX_ROWS).await?,
        ))
    }

    /// The places of `answer` as rows to list, at most `limit` of them: with their line, and
    /// where they stand against the changeset on `side`.
    async fn rows(
        &self,
        highlighter: &Highlighter,
        answer: vet_app::semantics::Answer<Vec<Reference>>,
        side: Side,
        at: &At,
        limit: usize,
    ) -> Result<vet_protocol::LocationRows> {
        let revision = answer.revision;
        let answerer = self.language_of(&at.file);
        let (found, unavailable) = map::answer(answer.result, answerer.clone());
        let files = self.view.changed_files(revision).await?;
        let mut texts: HashMap<RepoPath, Option<FileLines>> = HashMap::new();
        let mut rows = Vec::with_capacity(found.len());
        for r in &found {
            let file = &r.location.file;
            if !texts.contains_key(file) {
                let text = if r.location.external {
                    read_outside(answerer.as_deref(), file).await
                } else {
                    self.view.side_text(revision, core_side(side), file).await?
                };
                let lines = match text {
                    Some(text) => {
                        let highlights = highlighter.text(&file.to_string_lossy(), Some(&text)).await;
                        let starts = Lines::new(&text).starts().to_vec();
                        Some(FileLines {
                            text,
                            highlights,
                            starts,
                        })
                    }
                    None => None,
                };
                texts.insert(file.clone(), lines);
            }
            let line = r.location.span.start.line;
            let source = texts[file].as_ref();
            let text = source.map_or("", |s| map::line_at(&s.text, line));
            let standing = if r.location.external {
                (vet_protocol::LocationChange::Unchanged, None)
            } else {
                map::change_at(&files, core_side(side), file, line)
            };
            rows.push(map::row(r, text, standing, |from, to| {
                source.map_or_else(Vec::new, |s| {
                    let start = s.line_start(line);
                    vet_highlight::slice(&s.highlights, start + from, start + to)
                })
            }));
        }
        let (rows, total) = map::finish(rows, limit);
        Ok(vet_protocol::LocationRows {
            revision,
            side,
            rows,
            total,
            unavailable,
        })
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
    use super::*;

    #[tokio::test]
    async fn only_sources_of_the_answering_servers_language_are_read_from_outside_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, "secret = 1\n").unwrap();
            RepoPath::from(path.to_string_lossy().as_ref())
        };
        for (language, source) in [
            ("rust", "lib.rs"),
            ("typescript", "index.d.ts"),
            ("typescript", "index.js"),
            ("svelte", "index.d.ts"),
            ("python", "typing.pyi"),
            ("dockerfile", "Dockerfile"),
        ] {
            assert!(read_outside(Some(language), &file(source)).await.is_some(), "{source}");
        }
        for (language, other) in [
            ("rust", "config"),
            ("rust", "hosts.yml"),
            ("typescript", "secrets.json"),
            ("typescript", "lib.rs"),
            ("python", ".env"),
            ("python", "id_rsa"),
        ] {
            assert!(read_outside(Some(language), &file(other)).await.is_none(), "{other}");
        }
        assert!(read_outside(None, &file("lib.rs")).await.is_none());
    }
}
