//! A reference with the symbol it is in: the language servers answer `references` with
//! places only.

use std::collections::HashMap;

use vet_core::content::RepoPath;
use vet_core::review::Side;

use super::{Derived, Inputs};
use crate::ports::{Location, Sym, SymbolInfo};
use crate::query::Primitives;
use crate::symbols::enclosing;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reference {
    pub location: Location,
    /// The innermost symbol the reference is in. `None` at the top level of a file, outside the
    /// repository and where the outline of its file is unknown.
    pub within: Option<Sym>,
}

/// `locations` on `side` with the symbols around them (derived).
pub async fn located(p: &impl Primitives, side: Side, locations: Vec<Location>) -> Derived<Vec<Reference>> {
    let mut inputs = Inputs::new("located");
    let value = within(p, &mut inputs, side, locations).await;
    Derived { value, inputs }
}

/// `locations` with the symbols around them, from the outlines of their files on `side`. An
/// outline a server cannot give leaves its file's references without one.
pub(super) async fn within(
    p: &impl Primitives,
    inputs: &mut Inputs,
    side: Side,
    locations: Vec<Location>,
) -> Vec<Reference> {
    let mut outlines: HashMap<RepoPath, Vec<SymbolInfo>> = HashMap::new();
    let mut out = Vec::with_capacity(locations.len());
    for location in locations {
        if !location.external && !outlines.contains_key(&location.file) {
            let found = p.document_symbols(side, &location.file).await;
            inputs.query(
                "document_symbols",
                side,
                &location.file,
                None,
                found.as_ref().ok().map(|a| &a.provenance),
            );
            outlines.insert(location.file.clone(), found.map(|a| a.value).unwrap_or_default());
        }
        let within = outlines
            .get(&location.file)
            .and_then(|symbols| enclosing(symbols, location.span.start))
            .map(|s| s.sym.clone());
        out.push(Reference { location, within });
    }
    out
}
