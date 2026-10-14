//! Semantic requests (§7.3) of a session. The use cases are in
//! `vet_app::semantics`; this maps requests to them and their results to the
//! protocol, turning a language without a usable server into `unavailable`.

use std::collections::HashMap;

use vet_app::semantics::Answer;
use vet_app::session::{SessionError, View};
use vet_core::content::RepoPath;
use vet_core::text::Lines;
use vet_protocol::{Response, Side};
use vet_semantic::derived::Reference;
use vet_semantic::ports::{At, BackendId, Location};
use vet_semantic::query::SemanticError;

use vet_mapping::semantic as map;
use vet_mapping::{core_side, core_view};

use super::highlight::Highlighter;
use super::open::Booted;
use super::{highlight, message};

/// The largest file outside the repository whose lines are shown.
const MAX_OUTSIDE_FILE: u64 = 8 * 1024 * 1024;

/// The text of a file outside the repository (a dependency or toolchain source), which is
/// named by its absolute path; `None` if it cannot be read.
async fn read_outside(file: &RepoPath) -> Option<String> {
    let path = std::path::PathBuf::from(file.to_string_lossy());
    if tokio::fs::metadata(&path).await.ok()?.len() > MAX_OUTSIDE_FILE {
        return None;
    }
    let bytes = tokio::fs::read(&path).await.ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
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
    fn language_of(&self, file: &RepoPath) -> Option<String> {
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
    pub(crate) async fn restart_server(&self, language: &BackendId) -> Result<(), String> {
        let revision = self.session.revision().await;
        (self.semantics)
            .restart_now(&self.view, revision, language)
            .await
            .map_err(message)?
            .map_err(|e| e.to_string())
    }

    pub(crate) async fn hover(
        &self,
        revision: u32,
        side: Side,
        path: &str,
        line: u32,
        col: u32,
    ) -> Result<Response, String> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .hover(&self.view, revision, core_side(side), &at)
            .await
            .map_err(message)?;
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
    ) -> Result<Response, String> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .definition(&self.view, revision, core_side(side), &at)
            .await
            .map_err(message)?;
        Ok(Response::Definition(self.locations(answer, side, &at)))
    }

    pub(crate) async fn workspace_symbols(&self, revision: u32, query: &str) -> Result<Response, String> {
        let found = (self.semantics)
            .workspace_symbols(&self.view, revision, query)
            .await
            .map_err(message)?;
        Ok(Response::WorkspaceSymbols(map::workspace_symbols(&found)))
    }

    pub(crate) async fn references(
        &self,
        highlighter: &Highlighter,
        revision: u32,
        side: Side,
        (path, line, col): (&str, u32, u32),
        limit: Option<u32>,
    ) -> Result<Response, String> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .references(&self.view, revision, core_side(side), &at)
            .await
            .map_err(message)?;
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
    ) -> Result<Response, String> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .implementations(&self.view, revision, core_side(side), &at)
            .await
            .map_err(message)?;
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
    ) -> Result<vet_protocol::LocationRows, String> {
        let revision = answer.revision;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&at.file));
        let files = self.view.changed_files(revision).await.map_err(message)?;
        let mut texts: HashMap<RepoPath, Option<FileLines>> = HashMap::new();
        let mut rows = Vec::with_capacity(found.len());
        for r in &found {
            let file = &r.location.file;
            if !texts.contains_key(file) {
                let text = if r.location.external {
                    read_outside(file).await
                } else {
                    self.view
                        .side_text(revision, core_side(side), file)
                        .await
                        .map_err(message)?
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

    pub(crate) async fn diagnostics(&self, revision: u32, path: &str, against_base: bool) -> Result<Response, String> {
        let file = RepoPath::from(path);
        let language = self.language_of(&file);
        if !against_base {
            let answer = (self.semantics)
                .diagnostics(&self.view, revision, &file)
                .await
                .map_err(message)?;
            let (found, unavailable) = map::answer(answer.result, language);
            return Ok(Response::Diagnostics {
                revision,
                path: path.to_owned(),
                diagnostics: map::pending_diagnostics(&found),
                count: vet_app::semantics::countable(&found),
                unavailable,
                origin_unavailable: None,
            });
        }
        let answer = (self.semantics)
            .judged_diagnostics(&self.view, revision, &file)
            .await
            .map_err(message)?;
        let (diagnostics, count, unavailable, origin_unavailable) = match answer.result {
            Ok(judgement) => {
                let found: Vec<_> = judgement.diagnostics.iter().map(|j| j.diagnostic.clone()).collect();
                (
                    map::judged_diagnostics(&judgement.diagnostics),
                    vet_app::semantics::countable(&found),
                    None,
                    judgement.base.map(|e| map::unavailable(&e, language)),
                )
            }
            Err(e) => (Vec::new(), 0, Some(map::unavailable(&e, language)), None),
        };
        Ok(Response::Diagnostics {
            revision,
            path: path.to_owned(),
            diagnostics,
            count,
            unavailable,
            origin_unavailable,
        })
    }

    pub(crate) async fn file_symbols(&self, revision: u32, path: &str) -> Result<Response, String> {
        let file = RepoPath::from(path);
        let found = (self.semantics)
            .file_symbols(&self.view, revision, &file)
            .await
            .map_err(message)?;
        Ok(Response::FileSymbols(map::file_symbols(
            revision,
            path,
            &found.changes,
            found.gap,
        )))
    }

    pub(crate) async fn file_outline(
        &self,
        view: vet_protocol::View,
        revision: u32,
        path: &str,
    ) -> Result<Response, String> {
        let file = RepoPath::from(path);
        let answer = match core_view(view) {
            View::Changeset => (self.semantics)
                .file_outline(&self.view, revision, &file)
                .await
                .map_err(message)?,
            // The semantic layer holds the changeset's snapshots, not a commit's: such a view has
            // the outline of its text, with no marks.
            other => {
                if self.semantics.language_of(&file).is_none() {
                    Answer {
                        revision,
                        result: Err(SemanticError::NoBackend(file.clone())),
                        provenance: None,
                    }
                } else {
                    let text = self.session.target_text(other, path).await.map_err(message)?;
                    if text.revision != revision {
                        return Err(message(SessionError::Stale { current: text.revision }));
                    }
                    match text.content {
                        Ok(text) => (self.semantics)
                            .text_outline(&self.view, revision, &file, &text)
                            .await
                            .map_err(message)?,
                        // No text to read symbols from: the file is not there at this commit.
                        Err(_) => Answer {
                            revision,
                            result: Ok(Vec::new()),
                            provenance: None,
                        },
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
    ) -> Result<Response, String> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .symbol_at(&self.view, revision, core_side(side), &at)
            .await
            .map_err(message)?;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&at.file));
        Ok(Response::SymbolAt(map::symbol_at(
            revision,
            found.as_ref(),
            unavailable,
        )))
    }

    pub(crate) async fn calls(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let (info, calls) = (self.semantics)
            .calls(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::Calls(map::calls(revision, &info, &calls)))
    }

    pub(crate) async fn usages(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let (info, usages) = (self.semantics)
            .usages(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::Usages(map::usages(revision, &info, &usages)))
    }

    pub(crate) async fn signature(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let answer = (self.semantics)
            .signature(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&RepoPath::from(symbol.path.as_str())));
        Ok(highlight::signature(&symbol.path, revision, found, unavailable))
    }

    pub(crate) async fn removal_impact(
        &self,
        revision: u32,
        symbol: &vet_protocol::SymbolId,
    ) -> Result<Response, String> {
        let impact = (self.semantics)
            .removal_impact(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::RemovalImpact(map::removal_impact(revision, &impact)))
    }

    pub(crate) async fn reachability(
        &self,
        revision: u32,
        symbol: &vet_protocol::SymbolId,
    ) -> Result<Response, String> {
        let (info, reach) = (self.semantics)
            .reachability(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::Reachability(map::reachability(revision, &info, &reach)))
    }
}
