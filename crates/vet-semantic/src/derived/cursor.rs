//! The symbol a position is in, and what became of it.

use vet_core::review::Side;

use super::changed::{Change, Compared};
use crate::ports::{Pos, SymbolInfo};
use crate::symbols::enclosing;

/// A symbol, and what became of it if the changeset touched its file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Located {
    /// The target side's symbol, or the base side's if it exists there only.
    pub symbol: SymbolInfo,
    /// `None`: unchanged.
    pub change: Option<Change>,
}

/// The innermost symbol of a changed file that holds `pos` on `side`: the position of the cursor
/// in a base-side pane is placed in the base text, and the symbol answered is the target side's
/// where there is one. A symbol only one side has was added or removed, whether or not a hunk
/// touched it itself.
pub fn symbol_at(compared: &Compared, side: Side, pos: Pos) -> Option<Located> {
    let (here, other) = match side {
        Side::Base => (&compared.base, &compared.target),
        Side::Target => (&compared.target, &compared.base),
    };
    let found = enclosing(here, pos)?;
    let Some(counterpart) = other.iter().find(|s| s.sym.name == found.sym.name) else {
        let change = match side {
            Side::Base => Change::Removed,
            Side::Target => Change::Added,
        };
        return Some(Located {
            symbol: found.clone(),
            change: Some(change),
        });
    };
    let target = match side {
        Side::Base => counterpart,
        Side::Target => found,
    };
    Some(Located {
        symbol: target.clone(),
        change: compared.changes.change_of(&target.sym).map(|c| c.change),
    })
}
