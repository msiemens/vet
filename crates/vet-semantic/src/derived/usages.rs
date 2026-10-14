//! Usages of a symbol: its `references` on the target, each with the symbol it is in.

use vet_core::review::Side;

use super::reference::{Reference, within};
use super::{Derived, Inputs};
use crate::ports::SymbolInfo;
use crate::query::{Primitives, SemanticError};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Usages {
    pub references: Vec<Reference>,
    /// Set when the references could not be listed: they are unknown, not absent.
    pub gap: Option<SemanticError>,
}

pub async fn usages(p: &impl Primitives, symbol: &SymbolInfo) -> Derived<Usages> {
    let mut inputs = Inputs::new("usages");
    let at = symbol.location();
    let found = p.references(Side::Target, &at).await;
    let value = match inputs.asked_at("references", Side::Target, &at, found) {
        Ok(found) => Usages {
            references: within(p, &mut inputs, Side::Target, found).await,
            gap: None,
        },
        Err(e) => Usages {
            references: Vec::new(),
            gap: Some(e),
        },
    };
    Derived { value, inputs }
}
