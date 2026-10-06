//! Derived analyses (I6): built over primitives, labeled as derived, and carrying
//! the inputs they were computed from so the UI can show them.

use std::collections::{HashMap, VecDeque};

use crate::ports::Sym;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bounds {
    pub max_depth: u32,
    pub max_nodes: usize,
}

/// What a derived result was computed from.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Inputs {
    pub analysis: &'static str,
    pub bounds: Option<Bounds>,
    /// The primitive queries consulted, e.g. `incoming_calls(foo::bar)`.
    pub primitives: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Derived<T> {
    pub value: T,
    pub inputs: Inputs,
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

/// Bounded incoming-call search from `start`, stopping at the first entry point
/// (§7.4). `callers` is the `incoming_calls` primitive; a closure rather than a
/// trait, so tests pass a map.
pub fn reachability(
    start: &Sym,
    mut callers: impl FnMut(&Sym) -> Result<Vec<Sym>, String>,
    is_entry: impl Fn(&Sym) -> bool,
    bounds: Bounds,
) -> Derived<Reach> {
    let mut inputs = Inputs {
        analysis: "reachability",
        bounds: Some(bounds),
        primitives: Vec::new(),
    };
    let mut gaps = Vec::new();
    let mut parent: HashMap<Sym, Option<Sym>> = HashMap::from([(start.clone(), None)]);
    let mut frontier = VecDeque::from([(start.clone(), 0u32)]);

    while let Some((sym, depth)) = frontier.pop_front() {
        if is_entry(&sym) {
            let mut path = vec![sym.clone()];
            let mut at = &sym;
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
        inputs.primitives.push(format!("incoming_calls({})", sym.0));
        match callers(&sym) {
            Ok(found) => {
                for caller in found {
                    if parent.len() < bounds.max_nodes && !parent.contains_key(&caller) {
                        parent.insert(caller.clone(), Some(sym.clone()));
                        frontier.push_back((caller, depth + 1));
                    }
                }
            }
            Err(gap) => gaps.push(gap),
        }
    }

    let value = if gaps.is_empty() {
        Reach::UnreachedWithinBound
    } else {
        Reach::Unknown { gaps }
    };
    Derived { value, inputs }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str) -> Sym {
        Sym(name.into())
    }

    fn graph(edges: &[(&str, &[&str])]) -> impl FnMut(&Sym) -> Result<Vec<Sym>, String> {
        let map: HashMap<String, Vec<Sym>> = edges
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| sym(s)).collect()))
            .collect();
        move |s| Ok(map.get(&s.0).cloned().unwrap_or_default())
    }

    const BOUNDS: Bounds = Bounds {
        max_depth: 4,
        max_nodes: 100,
    };

    #[test]
    fn reached_reports_the_path_and_the_queries_it_used() {
        let result = reachability(
            &sym("leaf"),
            graph(&[("leaf", &["mid"]), ("mid", &["main"])]),
            |s| s.0 == "main",
            BOUNDS,
        );
        assert_eq!(
            result.value,
            Reach::Reached {
                path: vec![sym("leaf"), sym("mid"), sym("main")]
            }
        );
        assert_eq!(
            result.inputs.primitives,
            ["incoming_calls(leaf)", "incoming_calls(mid)"]
        );
    }

    #[test]
    fn depth_bound_gives_unreached_not_reached() {
        let bounds = Bounds { max_depth: 1, ..BOUNDS };
        let result = reachability(
            &sym("leaf"),
            graph(&[("leaf", &["mid"]), ("mid", &["main"])]),
            |s| s.0 == "main",
            bounds,
        );
        assert_eq!(result.value, Reach::UnreachedWithinBound);
        assert_eq!(result.inputs.bounds, Some(bounds));
    }

    #[test]
    fn backend_errors_surface_as_unknown() {
        let result = reachability(
            &sym("leaf"),
            |_| Err("no call hierarchy".to_string()),
            |_| false,
            BOUNDS,
        );
        assert_eq!(
            result.value,
            Reach::Unknown {
                gaps: vec!["no call hierarchy".into()]
            }
        );
    }
}
