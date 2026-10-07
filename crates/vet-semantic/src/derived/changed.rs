//! Which symbols changed (§7.3): `documentSymbol` on both sides of each changed
//! file, intersected with the diff hunks.

use vet_core::content::{FileDiff, LineRange, RepoPath};

use super::{Derived, Inputs};
use crate::ports::SymbolInfo;
use crate::query::{Primitives, SemanticError, Which};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Added,
    Modified,
    /// Present on the base side only.
    Removed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangedSymbol {
    /// The target-side symbol, or the base-side one for `Removed`.
    pub symbol: SymbolInfo,
    pub change: Change,
}

/// A file whose symbols could not be determined.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Gap {
    pub file: RepoPath,
    pub reason: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Changes {
    pub symbols: Vec<ChangedSymbol>,
    /// Files no configured backend handles.
    pub unsupported: Vec<RepoPath>,
    /// Files a backend failed on; their symbols are unknown, not unchanged.
    pub gaps: Vec<Gap>,
}

pub async fn changed_symbols(p: &impl Primitives, files: &[FileDiff]) -> Derived<Changes> {
    let mut inputs = Inputs::new("changed symbols");
    let mut out = Changes::default();
    for file in files.iter().filter(|f| !f.hunks.is_empty()) {
        let shown = file.path();
        let old: Vec<_> = file.hunks.iter().map(|h| h.old).collect();
        let new: Vec<_> = file.hunks.iter().map(|h| h.new).collect();
        inputs.record_hunks(&shown.to_string_lossy(), &file.hunks);

        let base = symbols(p, &mut inputs, Which::Base, file.old_path.as_ref()).await;
        let target = symbols(p, &mut inputs, Which::Target, file.new_path.as_ref()).await;
        let (base, target) = match (base, target) {
            (Ok(b), Ok(t)) => (b, t),
            (Err(e), _) | (_, Err(e)) => {
                match e {
                    SemanticError::NoBackend(_) => out.unsupported.push(shown.clone()),
                    e => out.gaps.push(Gap {
                        file: shown.clone(),
                        reason: e.to_string(),
                    }),
                }
                continue;
            }
        };

        let has = |set: &[SymbolInfo], s: &SymbolInfo| set.iter().any(|o| o.sym.name == s.sym.name);
        let added_file = file.old_path.is_none();
        let deleted_file = file.new_path.is_none();

        let touched_target: Vec<_> = if added_file {
            target.iter().collect()
        } else {
            touched(&target, &new)
        };
        for s in touched_target {
            let change = if has(&base, s) { Change::Modified } else { Change::Added };
            out.symbols.push(ChangedSymbol {
                symbol: s.clone(),
                change,
            });
        }

        let touched_base: Vec<_> = if deleted_file {
            base.iter().collect()
        } else {
            touched(&base, &old)
        };
        for s in touched_base.into_iter().filter(|s| !has(&target, s)) {
            out.symbols.push(ChangedSymbol {
                symbol: s.clone(),
                change: Change::Removed,
            });
        }
    }
    Derived { value: out, inputs }
}

async fn symbols(
    p: &impl Primitives,
    inputs: &mut Inputs,
    which: Which,
    file: Option<&RepoPath>,
) -> Result<Vec<SymbolInfo>, SemanticError> {
    let Some(file) = file else { return Ok(Vec::new()) };
    inputs.query("document_symbols", which, file, None);
    p.document_symbols(which, file).await
}

/// The innermost symbols each hunk touches. An empty range is a deletion or
/// insertion point and touches only a symbol it lies strictly inside.
fn touched<'a>(symbols: &'a [SymbolInfo], hunks: &[LineRange]) -> Vec<&'a SymbolInfo> {
    let mut out: Vec<&SymbolInfo> = Vec::new();
    for hunk in hunks {
        let hit = |s: &SymbolInfo| {
            let lines = s.span.lines();
            if hunk.is_empty() {
                lines.start < hunk.start && hunk.start < lines.end
            } else {
                lines.intersects(*hunk)
            }
        };
        let candidates: Vec<_> = symbols.iter().filter(|s| hit(s)).collect();
        for s in &candidates {
            let contains_another = candidates
                .iter()
                .any(|o| o.sym != s.sym && s.span.lines().contains(o.span.lines()));
            if !contains_another && !out.iter().any(|o| o.sym == s.sym) {
                out.push(s);
            }
        }
    }
    out
}
