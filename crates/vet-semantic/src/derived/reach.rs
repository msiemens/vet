//! Reachability (§7.4): a bounded incoming-call search from a changed or added
//! symbol, stopping at the first configured entry point.

use std::collections::{HashMap, VecDeque};

use vet_core::review::Side;

use super::{Bounds, Derived, Inputs};
use crate::ports::{Sym, SymbolInfo};
use crate::query::Primitives;

/// A symbol whose last name segment is `name`, in a file under `path_prefix`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EntryPoint {
    /// Empty matches every file.
    pub path_prefix: String,
    pub name: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EntryPoints(pub Vec<EntryPoint>);

impl EntryPoints {
    pub fn matches(&self, sym: &Sym) -> bool {
        let file = sym.file.to_string_lossy();
        self.0
            .iter()
            .any(|e| file.starts_with(&e.path_prefix) && sym.name.last() == Some(&e.name))
    }
}

impl Default for EntryPoints {
    /// `main` in any file.
    fn default() -> Self {
        Self(vec![EntryPoint {
            path_prefix: String::new(),
            name: "main".into(),
        }])
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reach {
    /// Path from the changed symbol to the entry point.
    Reached {
        path: Vec<Sym>,
    },
    UnreachedWithinBound,
    /// Backend gaps (DI, reflection, serialization).
    Unknown {
        gaps: Vec<String>,
    },
}

pub async fn reachability(
    p: &impl Primitives,
    start: &SymbolInfo,
    entries: &EntryPoints,
    bounds: Bounds,
) -> Derived<Reach> {
    let mut inputs = Inputs::new("reachability");
    inputs.bounds = Some(bounds);
    inputs.config = entries
        .0
        .iter()
        .map(|e| format!("entry point {}::{}", e.path_prefix, e.name))
        .collect();
    let mut gaps = Vec::new();
    let mut parent: HashMap<Sym, Option<Sym>> = HashMap::from([(start.sym.clone(), None)]);
    let mut frontier = VecDeque::from([(start.clone(), 0u32)]);

    while let Some((node, depth)) = frontier.pop_front() {
        if entries.matches(&node.sym) {
            let mut path = vec![node.sym.clone()];
            let mut at = &node.sym;
            while let Some(Some(up)) = parent.get(at) {
                path.push(up.clone());
                at = up;
            }
            path.reverse();
            return Derived {
                value: Reach::Reached { path },
                inputs,
            };
        }
        if depth >= bounds.max_depth {
            continue;
        }
        let at = node.location();
        inputs.query_at("incoming_calls", Side::Target, &at);
        match p.incoming_calls(Side::Target, &at).await {
            Ok(callers) => {
                for caller in callers {
                    if parent.len() < bounds.max_nodes && !parent.contains_key(&caller.sym) {
                        parent.insert(caller.sym.clone(), Some(node.sym.clone()));
                        frontier.push_back((caller, depth + 1));
                    }
                }
            }
            Err(e) => gaps.push(format!("{}: {e}", node.sym)),
        }
    }

    let value = if gaps.is_empty() {
        Reach::UnreachedWithinBound
    } else {
        Reach::Unknown { gaps }
    };
    Derived { value, inputs }
}
