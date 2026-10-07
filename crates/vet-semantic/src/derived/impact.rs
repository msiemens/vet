//! Impact of a change (§7.3): `references` and incoming calls of each changed
//! or added symbol, on the target.

use vet_core::review::Side;

use super::changed::{Change, ChangedSymbol};
use super::{Derived, Inputs};
use crate::ports::{Location, SymbolInfo};
use crate::query::Primitives;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SymbolImpact {
    pub symbol: SymbolInfo,
    pub references: Vec<Location>,
    pub callers: Vec<SymbolInfo>,
    /// Queries that failed; the matching list is unknown, not empty.
    pub gaps: Vec<String>,
}

/// Removed symbols are skipped: see [`super::removal_impact`].
pub async fn impact(p: &impl Primitives, changed: &[ChangedSymbol]) -> Derived<Vec<SymbolImpact>> {
    let mut inputs = Inputs::new("impact");
    let mut out = Vec::new();
    for c in changed.iter().filter(|c| c.change != Change::Removed) {
        let at = c.symbol.location();
        let mut gaps = Vec::new();
        let found = p.references(Side::Target, &at).await;
        let references = inputs
            .asked_at("references", Side::Target, &at, found)
            .unwrap_or_else(|e| {
                gaps.push(format!("references: {e}"));
                Vec::new()
            });
        let found = p.incoming_calls(Side::Target, &at).await;
        let callers = inputs
            .asked_at("incoming_calls", Side::Target, &at, found)
            .unwrap_or_else(|e| {
                gaps.push(format!("incoming calls: {e}"));
                Vec::new()
            });
        out.push(SymbolImpact {
            symbol: c.symbol.clone(),
            references,
            callers,
            gaps,
        });
    }
    Derived { value: out, inputs }
}
