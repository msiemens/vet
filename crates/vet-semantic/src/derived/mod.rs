//! Derived analyses (I6): built over primitives, labeled as derived, and carrying
//! the inputs they were computed from so the UI can show them.

mod changed;
mod impact;
mod reach;
mod removal;

#[cfg(test)]
mod tests;

use vet_core::content::{LineRange, RepoPath};

use crate::ports::{At, Pos};
use crate::query::Which;

pub use changed::{Change, ChangedSymbol, Changes, Gap, changed_symbols};
pub use impact::{SymbolImpact, impact};
pub use reach::{EntryPoint, EntryPoints, Reach, reachability};
pub use removal::{CallSite, Fate, RemovalImpact, removal_impact};

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
    /// The primitive queries consulted, e.g. `incoming_calls(target:src/a.rs:3:4)`.
    pub primitives: Vec<String>,
    /// The diff hunks consulted, e.g. `src/a.rs: old 3..5, new 3..6`.
    pub hunks: Vec<String>,
    /// Configuration that shaped the result, e.g. the entry points.
    pub config: Vec<String>,
}

impl Inputs {
    fn new(analysis: &'static str) -> Self {
        Self {
            analysis,
            ..Self::default()
        }
    }

    fn query(&mut self, op: &str, which: Which, file: &RepoPath, pos: Option<Pos>) {
        let side = match which {
            Which::Base => "base",
            Which::Target => "target",
        };
        let file = path(file);
        self.primitives.push(match pos {
            Some(p) => format!("{op}({side}:{file}:{}:{})", p.line, p.col),
            None => format!("{op}({side}:{file})"),
        });
    }

    fn query_at(&mut self, op: &str, which: Which, at: &At) {
        self.query(op, which, &at.file, Some(at.pos));
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Derived<T> {
    pub value: T,
    pub inputs: Inputs,
}

fn path(p: &RepoPath) -> String {
    String::from_utf8_lossy(&p.0).into_owned()
}

fn lines_intersect(a: LineRange, b: LineRange) -> bool {
    a.start < b.end && b.start < a.end
}
