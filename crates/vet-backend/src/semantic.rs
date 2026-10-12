//! Semantic requests (§7.3) of a session. The use cases are in
//! `vet_app::semantics`; this maps requests to them and their results to the
//! protocol, turning a language without a usable server into `unavailable`.

use std::collections::HashMap;

use vet_core::content::RepoPath;
use vet_protocol::{Response, Side};
use vet_semantic::derived::Reference;
use vet_semantic::ports::{At, Location};

use vet_mapping::core_side;
use vet_mapping::semantic as map;

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

impl Booted {
    /// The language that serves `file`, as `ListServers` names it.
    fn language_of(&self, file: &RepoPath) -> Option<String> {
        self.semantics.language_of(file).map(|language| language.0.clone())
    }

    /// Starts the servers the current revision's changed files and the
    /// repository's languages need. A server that fails to start is reported
    /// by the query that needs it.
    pub async fn start_servers(&self) {
        let revision = self.session.revision().await;
        let _ = self.semantics.start_servers(&self.view, revision).await;
    }

    pub async fn hover(&self, revision: u32, side: Side, path: &str, line: u32, col: u32) -> Result<Response, String> {
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

    pub async fn definition(
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

    pub async fn references(
        &self,
        revision: u32,
        side: Side,
        path: &str,
        line: u32,
        col: u32,
    ) -> Result<Response, String> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .references(&self.view, revision, core_side(side), &at)
            .await
            .map_err(message)?;
        Ok(Response::References(self.rows(answer, side, &at).await?))
    }

    pub async fn implementations(
        &self,
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
        Ok(Response::Implementations(self.rows(answer, side, &at).await?))
    }

    /// The places of `answer` as rows to list: with their line, and where they stand against
    /// the changeset on `side`.
    async fn rows(
        &self,
        answer: vet_app::semantics::Answer<Vec<Reference>>,
        side: Side,
        at: &At,
    ) -> Result<vet_protocol::LocationRows, String> {
        let revision = answer.revision;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&at.file));
        let files = self.view.changed_files(revision).await.map_err(message)?;
        let mut texts: HashMap<RepoPath, Option<String>> = HashMap::new();
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
                texts.insert(file.clone(), text);
            }
            let line = r.location.span.start.line;
            let text = texts[file].as_deref().map_or("", |t| map::line_at(t, line));
            let change = if r.location.external {
                vet_protocol::LocationChange::Unchanged
            } else {
                map::change_at(&files, core_side(side), file, line)
            };
            rows.push(map::row(r, text, change));
        }
        let (rows, total) = map::finish(rows);
        Ok(vet_protocol::LocationRows {
            revision,
            side,
            rows,
            total,
            unavailable,
        })
    }

    pub async fn diagnostics(&self, revision: u32, path: &str) -> Result<Response, String> {
        let file = RepoPath::from(path);
        let answer = (self.semantics)
            .diagnostics(&self.view, revision, &file)
            .await
            .map_err(message)?;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&file));
        Ok(Response::Diagnostics {
            revision,
            path: path.to_owned(),
            diagnostics: map::diagnostics(&found),
            count: vet_app::semantics::countable(&found),
            unavailable,
        })
    }

    pub async fn changed_symbols(&self, revision: u32) -> Result<Response, String> {
        let changes = self
            .semantics
            .changed_symbols(&self.view, revision)
            .await
            .map_err(message)?;
        Ok(Response::ChangedSymbols(map::changed_symbols(revision, &changes)))
    }

    pub async fn signature(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let answer = (self.semantics)
            .signature(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&RepoPath::from(symbol.path.as_str())));
        Ok(highlight::signature(&symbol.path, revision, found, unavailable))
    }

    pub async fn impact(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let impact = (self.semantics)
            .impact(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::Impact(map::impact(revision, &impact)))
    }

    pub async fn removal_impact(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let impact = (self.semantics)
            .removal_impact(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::RemovalImpact(map::removal_impact(revision, &impact)))
    }

    pub async fn reachability(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let (info, reach) = (self.semantics)
            .reachability(&self.view, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::Reachability(map::reachability(revision, &info, &reach)))
    }
}
