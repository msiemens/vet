//! Derived analyses (I6): built over primitives, labeled as derived, and carrying
//! the inputs they were computed from so the UI can show them.

mod calls;
mod changed;
mod cursor;
mod origin;
mod outline;
mod reach;
mod reference;
mod removal;
mod usages;

#[cfg(test)]
mod tests;

use vet_core::content::{Hunk, RepoPath};
use vet_core::review::Side;

use crate::ports::{At, Pos, Provenance};
use crate::query::{Asked, SemanticError};

pub use calls::{Caller, Calls, calls};
pub use changed::{Change, ChangedSymbol, Compared, FileChanges, Texts, compared};
pub use cursor::{Located, symbol_at};
pub use origin::{Judged, Judgement, Origin, judged};
pub use outline::{OutlineSymbol, outline};
pub use reach::{EntryPoint, EntryPoints, Reach, reachability};
pub use reference::{Reference, located};
pub use removal::{CallSite, Fate, RemovalImpact, removal_impact};
pub use usages::{Usages, usages};

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
    /// The primitive queries consulted and who answered each, e.g.
    /// `incoming_calls(target:src/a.rs:3:4) [rust-analyzer 1.91.0]`.
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

    /// Records a primitive query and, if it was answered, the server that answered
    /// and whether the answer was reused (I6): `references(target:a.rs:3:4) [rust-analyzer 1.91, cached]`.
    fn query(&mut self, op: &str, side: Side, file: &RepoPath, pos: Option<Pos>, answered_by: Option<&Provenance>) {
        let side = match side {
            Side::Base => "base",
            Side::Target => "target",
        };
        let file = file.to_string_lossy();
        let mut query = match pos {
            Some(p) => format!("{op}({side}:{file}:{}:{})", p.line, p.col),
            None => format!("{op}({side}:{file})"),
        };
        if let Some(provenance) = answered_by {
            query.push_str(&format!(" [{provenance}]"));
        }
        self.primitives.push(query);
    }

    /// Records the hunks of the file `label` consulted.
    fn record_hunks(&mut self, label: &str, hunks: &[Hunk]) {
        let ranges: Vec<_> = (hunks.iter())
            .map(|h| format!("old {}..{} new {}..{}", h.old.start, h.old.end, h.new.start, h.new.end))
            .collect();
        self.hunks.push(format!("{label}: {}", ranges.join(", ")));
    }

    /// Records the query `op` at `at` with its outcome and hands the answer on.
    fn asked_at<T>(&mut self, op: &str, side: Side, at: &At, result: Asked<T>) -> Result<T, SemanticError> {
        self.query(
            op,
            side,
            &at.file,
            Some(at.pos),
            result.as_ref().ok().map(|a| &a.provenance),
        );
        result.map(|a| a.value)
    }
}

/// Adds to `gaps` that `what` is unknown because the query failed with `error`. A server that lacks
/// the operation lacks it for everything, which is said once and not per thing asked about.
fn note_gap(gaps: &mut Vec<String>, what: &str, error: &SemanticError) {
    let gap = match error {
        SemanticError::NotOffered { .. } => error.to_string(),
        _ => format!("{what}: {error}"),
    };
    if !gaps.contains(&gap) {
        gaps.push(gap);
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Derived<T> {
    pub value: T,
    pub inputs: Inputs,
}
