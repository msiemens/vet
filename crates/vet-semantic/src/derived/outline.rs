//! The whole outline of a changed file: the symbols of its target side, and those only its base
//! side has, each with what the change did to it.

use super::changed::{Change, Compared};
use crate::ports::SymbolInfo;

/// A symbol of a file's outline, and what the changeset did to it if it touched the file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutlineSymbol {
    /// The target side's symbol, or the base side's for a removed one.
    pub symbol: SymbolInfo,
    /// `None`: unchanged.
    pub change: Option<Change>,
}

/// The symbols of both sides of a changed file in the order of a reading: the target side's in
/// source order, and each removed symbol at the place it had in the base, after the sibling that
/// stood before it there (first in its holder if none did). A symbol only one side has was added
/// or removed, whether or not a hunk touched it itself; a modified one is as the hunks say.
pub fn outline(compared: &Compared) -> Vec<OutlineSymbol> {
    let named = |set: &[SymbolInfo], s: &SymbolInfo| set.iter().any(|o| o.sym.name == s.sym.name);
    let mut out: Vec<OutlineSymbol> = compared
        .target
        .iter()
        .map(|s| OutlineSymbol {
            symbol: s.clone(),
            change: compared
                .changes
                .change_of(&s.sym)
                .map(|c| c.change)
                .or_else(|| (!named(&compared.base, s)).then_some(Change::Added)),
        })
        .collect();

    for (at, gone) in compared.base.iter().enumerate() {
        if named(&compared.target, gone) {
            continue;
        }
        let name = &gone.sym.name;
        let holder = &name[..name.len() - 1];
        let before = compared.base[..at]
            .iter()
            .rev()
            .find(|s| s.sym.name.len() == name.len() && s.sym.name.starts_with(holder));
        let place = match before {
            Some(sibling) => after_subtree(&out, &sibling.sym.name),
            None if holder.is_empty() => Some(0),
            None => position(&out, holder).map(|i| i + 1),
        };
        out.insert(
            place.unwrap_or(out.len()),
            OutlineSymbol {
                symbol: gone.clone(),
                change: Some(Change::Removed),
            },
        );
    }
    out
}

fn position(out: &[OutlineSymbol], name: &[String]) -> Option<usize> {
    out.iter().position(|o| o.symbol.sym.name == name)
}

/// The index after the symbol `name` and everything nested in it.
fn after_subtree(out: &[OutlineSymbol], name: &[String]) -> Option<usize> {
    let at = position(out, name)?;
    let nested = out[at + 1..]
        .iter()
        .take_while(|o| o.symbol.sym.name.len() > name.len() && o.symbol.sym.name.starts_with(name))
        .count();
    Some(at + 1 + nested)
}
