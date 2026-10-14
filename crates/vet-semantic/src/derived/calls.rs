//! The calls around a symbol (§7.3): its callers followed through changed symbols only, its
//! changed callees, and its unchanged direct callers, which the change may affect.

use std::collections::{HashMap, HashSet};

use vet_core::content::{FileDiff, RepoPath};
use vet_core::review::Side;

use super::changed::{Change, ChangedSymbol, Compared, Texts, compared};
use super::{Bounds, Derived, Gap, Inputs, note_gap};
use crate::ports::{Sym, SymbolInfo};
use crate::query::Primitives;

/// A changed symbol that reaches the one asked about through calls of changed symbols alone.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Caller {
    pub symbol: SymbolInfo,
    /// Added or modified: the callers are those of the target side.
    pub change: Change,
    /// How many changed symbols lie between it and the one asked about.
    pub depth: u32,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Calls {
    /// In the order of a walk down the callers: each is followed by those that call it.
    pub callers: Vec<Caller>,
    /// What the symbol calls that was added, modified (target) or removed (base).
    pub callees: Vec<ChangedSymbol>,
    /// Unchanged symbols that call it directly; empty unless the symbol itself changed.
    pub affected: Vec<SymbolInfo>,
    /// Queries that failed or bounds that cut the walk; the matching list is incomplete.
    pub gaps: Vec<Gap>,
}

/// Which symbols of which changed files changed, asked of the texts once per file.
struct Changed<'a, P, T> {
    p: &'a P,
    texts: &'a T,
    files: &'a [FileDiff],
    seen: HashMap<(Side, RepoPath), Option<Compared>>,
}

impl<'a, P: Primitives, T: Texts> Changed<'a, P, T> {
    fn new(p: &'a P, texts: &'a T, files: &'a [FileDiff]) -> Self {
        Self {
            p,
            texts,
            files,
            seen: HashMap::new(),
        }
    }

    /// What became of `sym` as `side` has it, if the changeset changed it there: added or
    /// modified on the target, removed on the base.
    async fn change(&mut self, inputs: &mut Inputs, side: Side, sym: &Sym) -> Option<Change> {
        let key = (side, sym.file.clone());
        if !self.seen.contains_key(&key) {
            let file = self.files.iter().find(|f| {
                !f.hunks.is_empty()
                    && match side {
                        Side::Base => f.old_path.as_ref() == Some(&sym.file),
                        Side::Target => f.new_path.as_ref() == Some(&sym.file),
                    }
            });
            let found = match file {
                Some(file) => {
                    let Derived { value, inputs: used } = compared(self.p, self.texts, file).await;
                    inputs.primitives.extend(used.primitives);
                    value.ok()
                }
                None => None,
            };
            self.seen.insert(key.clone(), found);
        }
        let found = self.seen[&key].as_ref()?;
        let change = found.changes.change_of(sym)?.change;
        match (side, change) {
            (Side::Base, Change::Removed) => Some(change),
            (Side::Target, Change::Added | Change::Modified) => Some(change),
            _ => None,
        }
    }
}

/// The calls around `symbol`, a symbol of the target side. `base` is the symbol on the base side,
/// if it has one: what it called there and is gone now is among the callees. `changed` says the
/// symbol was changed, which makes its unchanged callers possibly affected.
pub async fn calls(
    p: &impl Primitives,
    texts: &impl Texts,
    files: &[FileDiff],
    symbol: &SymbolInfo,
    base: Option<&SymbolInfo>,
    changed: bool,
    bounds: Bounds,
) -> Derived<Calls> {
    let mut inputs = Inputs::new("calls");
    inputs.bounds = Some(bounds);
    let mut known = Changed::new(p, texts, files);
    let mut out = Calls::default();

    let mut visited: HashSet<Sym> = HashSet::from([symbol.sym.clone()]);
    let mut pending: Vec<(SymbolInfo, Change, u32)> = Vec::new();
    let mut next = Some((symbol.clone(), 0u32));
    let mut too_deep = false;
    loop {
        if let Some((node, depth)) = next.take() {
            if depth >= bounds.max_depth {
                too_deep = true;
            } else {
                let at = node.location();
                let found = p.incoming_calls(Side::Target, &at).await;
                match inputs.asked_at("incoming_calls", Side::Target, &at, found) {
                    Ok(found) => {
                        let mut changed_callers = Vec::new();
                        for caller in found {
                            if !visited.insert(caller.sym.clone()) {
                                continue;
                            }
                            match known.change(&mut inputs, Side::Target, &caller.sym).await {
                                Some(change) => changed_callers.push((caller, change, depth + 1)),
                                None if depth == 0 && changed => out.affected.push(caller),
                                None => {}
                            }
                        }
                        pending.extend(changed_callers.into_iter().rev());
                    }
                    Err(e) => note_gap(&mut out.gaps, &format!("callers of {}", node.sym), &e),
                }
            }
        }
        let Some((caller, change, depth)) = pending.pop() else {
            break;
        };
        if out.callers.len() >= bounds.max_nodes {
            out.gaps.push(Gap::TooManyCallers { max: bounds.max_nodes });
            break;
        }
        out.callers.push(Caller {
            symbol: caller.clone(),
            change,
            depth: depth - 1,
        });
        next = Some((caller, depth));
    }
    if too_deep {
        out.gaps.push(Gap::TooDeep {
            max_depth: bounds.max_depth,
        });
    }

    let at = symbol.location();
    let found = p.outgoing_calls(Side::Target, &at).await;
    match inputs.asked_at("outgoing_calls", Side::Target, &at, found) {
        Ok(found) => {
            for callee in found.into_iter().filter(|c| c.sym != symbol.sym) {
                if let Some(change) = known.change(&mut inputs, Side::Target, &callee.sym).await {
                    out.callees.push(ChangedSymbol { symbol: callee, change });
                }
            }
        }
        Err(e) => note_gap(&mut out.gaps, "callees", &e),
    }
    if let Some(base) = base {
        let at = base.location();
        let found = p.outgoing_calls(Side::Base, &at).await;
        match inputs.asked_at("outgoing_calls", Side::Base, &at, found) {
            Ok(found) => {
                for callee in found {
                    if known.change(&mut inputs, Side::Base, &callee.sym).await.is_some() {
                        out.callees.push(ChangedSymbol {
                            symbol: callee,
                            change: Change::Removed,
                        });
                    }
                }
            }
            Err(e) => note_gap(&mut out.gaps, "callees on the base side", &e),
        }
    }
    Derived { value: out, inputs }
}
