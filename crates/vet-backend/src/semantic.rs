//! Semantic requests (§7.3) of a session. The use cases are in
//! `vet_app::semantics`; this maps requests to them and their results to the
//! protocol, turning a language without a usable server into `unavailable`.

use vet_core::content::RepoPath;
use vet_protocol::{Response, Side};
use vet_semantic::ports::{At, Location};

use super::highlight;
use super::mapping::core_side;
use super::mapping::semantic as map;
use super::{Open, message};

impl Open {
    /// The language that serves `file`, as `ListServers` names it.
    fn language_of(&self, file: &RepoPath) -> Option<String> {
        self.semantics.language_of(file).map(|language| language.0.clone())
    }

    /// Starts the servers the current revision's changed files and the
    /// repository's languages need. A server that fails to start is reported
    /// by the query that needs it.
    pub async fn start_servers(&self) {
        let revision = self.session.revision().await;
        let _ = self.semantics.start_servers(&self.session, revision).await;
    }

    pub async fn hover(&self, revision: u32, side: Side, path: &str, line: u32, col: u32) -> Result<Response, String> {
        let at = map::at(path, line, col);
        let answer = (self.semantics)
            .hover(&self.session, revision, core_side(side), &at)
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
            .definition(&self.session, revision, core_side(side), &at)
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
            .references(&self.session, revision, core_side(side), &at)
            .await
            .map_err(message)?;
        Ok(Response::References(self.locations(answer, side, &at)))
    }

    pub async fn diagnostics(&self, revision: u32, path: &str) -> Result<Response, String> {
        let file = RepoPath::from(path);
        let answer = (self.semantics)
            .diagnostics(&self.session, revision, &file)
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
            .changed_symbols(&self.session, revision)
            .await
            .map_err(message)?;
        Ok(Response::ChangedSymbols(map::changed_symbols(revision, &changes)))
    }

    pub async fn signature(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let answer = (self.semantics)
            .signature(&self.session, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        let (found, unavailable) = map::answer(answer.result, self.language_of(&RepoPath::from(symbol.path.as_str())));
        Ok(highlight::signature(&symbol.path, revision, found, unavailable))
    }

    pub async fn impact(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let impact = (self.semantics)
            .impact(&self.session, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::Impact(map::impact(revision, &impact)))
    }

    pub async fn removal_impact(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let impact = (self.semantics)
            .removal_impact(&self.session, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::RemovalImpact(map::removal_impact(revision, &impact)))
    }

    pub async fn reachability(&self, revision: u32, symbol: &vet_protocol::SymbolId) -> Result<Response, String> {
        let (info, reach) = (self.semantics)
            .reachability(&self.session, revision, &map::core_sym(symbol))
            .await
            .map_err(message)?;
        Ok(Response::Reachability(map::reachability(revision, &info, &reach)))
    }
}
