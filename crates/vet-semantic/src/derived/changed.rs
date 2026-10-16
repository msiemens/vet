//! Which symbols changed (§6.3): `documentSymbol` on both sides of each changed
//! file, intersected with the diff hunks.

use vet_core::content::{FileDiff, LineRange};
use vet_core::review::Side;

use super::{Derived, Inputs};
use crate::join::join;
use crate::ports::{Provenance, Sym, SymbolInfo};
use crate::query::{Asked, Primitives, SemanticError};

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

/// What the change of one file did to its symbols.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FileChanges {
    pub symbols: Vec<ChangedSymbol>,
    /// The symbols that hold a changed one without being changed themselves, in source order:
    /// where the changed ones are. A removed symbol's are those of the target side if it has
    /// them, else of the base.
    pub context: Vec<SymbolInfo>,
}

impl FileChanges {
    /// The change `sym` underwent, if it is one of the changed symbols.
    pub fn change_of(&self, sym: &Sym) -> Option<&ChangedSymbol> {
        self.symbols.iter().find(|c| &c.symbol.sym == sym)
    }
}

/// The symbols of a changed file on both sides, and what the change did to them.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Compared {
    pub base: Vec<SymbolInfo>,
    pub target: Vec<SymbolInfo>,
    pub changes: FileChanges,
}

/// The file contents of both sides, so their symbols can be had from the texts alone:
/// without a server on the base snapshot, and without the target file as the live
/// tree has it now, which may have moved on from the revision.
pub trait Texts: Sync {
    /// The base content of `file`; `None` if it is not available.
    fn base_text(&self, file: &FileDiff) -> impl Future<Output = Option<String>> + Send;
    /// The target content of `file`; `None` if it is not available.
    fn target_text(&self, file: &FileDiff) -> impl Future<Output = Option<String>> + Send;
}

/// The symbols of both sides of `file`, from the texts, and what the change did to them.
pub async fn compared(
    p: &impl Primitives,
    texts: &impl Texts,
    file: &FileDiff,
) -> Derived<Result<Compared, SemanticError>> {
    let mut inputs = Inputs::new("changed symbols");
    inputs.record_hunks(&file.path().to_string_lossy(), &file.hunks);
    let (base, target) = join(base_symbols(p, texts, file), target_symbols(p, texts, file)).await;
    for (side, path, answer) in [
        (Side::Base, &file.old_path, &base),
        (Side::Target, &file.new_path, &target),
    ] {
        if let Some(path) = path {
            let by = answer.as_ref().ok().and_then(|(_, by)| by.as_ref());
            inputs.query("document_symbols", side, path, None, by);
        }
    }
    let value = match (base, target) {
        (Ok((base, _)), Ok((target, _))) => {
            let changes = changes_between(file, &base, &target);
            Ok(Compared { base, target, changes })
        }
        (Err(e), _) | (_, Err(e)) => Err(e),
    };
    Derived { value, inputs }
}

/// What the hunks of `file` did to the symbols of its sides.
fn changes_between(file: &FileDiff, base: &[SymbolInfo], target: &[SymbolInfo]) -> FileChanges {
    let old: Vec<_> = file.hunks.iter().map(|h| h.old).collect();
    let new: Vec<_> = file.hunks.iter().map(|h| h.new).collect();
    let has = |set: &[SymbolInfo], s: &SymbolInfo| set.iter().any(|o| o.sym.name == s.sym.name);
    let mut symbols = Vec::new();

    let touched_target: Vec<_> = if file.old_path.is_none() {
        target.iter().collect()
    } else {
        touched(target, &new)
    };
    for s in touched_target {
        let change = if has(base, s) { Change::Modified } else { Change::Added };
        symbols.push(ChangedSymbol {
            symbol: s.clone(),
            change,
        });
    }

    let touched_base: Vec<_> = if file.new_path.is_none() {
        base.iter().collect()
    } else {
        touched(base, &old)
    };
    for s in touched_base.into_iter().filter(|s| !has(target, s)) {
        symbols.push(ChangedSymbol {
            symbol: s.clone(),
            change: Change::Removed,
        });
    }

    let mut context: Vec<SymbolInfo> = Vec::new();
    for changed in &symbols {
        let name = &changed.symbol.sym.name;
        let sides: &[&[SymbolInfo]] = if changed.change == Change::Removed {
            &[target, base]
        } else {
            &[target]
        };
        for depth in 1..name.len() {
            let holder = &name[..depth];
            let listed =
                symbols.iter().any(|o| o.symbol.sym.name == holder) || context.iter().any(|o| o.sym.name == holder);
            if listed {
                continue;
            }
            if let Some(found) = sides.iter().find_map(|side| side.iter().find(|s| s.sym.name == holder)) {
                context.push(found.clone());
            }
        }
    }
    context.sort_by_key(|s| (s.span.start.line, s.span.start.col));
    FileChanges { symbols, context }
}

/// A file's symbols and the server that gave them; none for a side the file is not on.
type Symbols = Result<(Vec<SymbolInfo>, Option<Provenance>), SemanticError>;

fn attributed(answer: Asked<Vec<SymbolInfo>>) -> Symbols {
    answer.map(|a| (a.value, Some(a.provenance)))
}

/// The symbols a document declares depend on its text alone, so the base side
/// is asked of the target side's server, which is running anyway; the base
/// side's own server, which has to restore the base's dependencies first, is
/// asked only when the text cannot be had or placed.
async fn base_symbols(p: &impl Primitives, texts: &impl Texts, file: &FileDiff) -> Symbols {
    let Some(path) = &file.old_path else {
        return Ok((Vec::new(), None));
    };
    if let Some(text) = texts.base_text(file).await
        && let Ok(found) = p.text_symbols(path, &text).await
    {
        return attributed(Ok(found));
    }
    attributed(p.document_symbols(Side::Base, path).await)
}

/// The target side's symbols from the revision's text, which the positions of its
/// hunks belong to; the file as its side's server has it is asked only when the text
/// cannot be had or placed.
async fn target_symbols(p: &impl Primitives, texts: &impl Texts, file: &FileDiff) -> Symbols {
    let Some(path) = &file.new_path else {
        return Ok((Vec::new(), None));
    };
    if let Some(text) = texts.target_text(file).await
        && let Ok(found) = p.text_symbols(path, &text).await
    {
        return attributed(Ok(found));
    }
    attributed(p.document_symbols(Side::Target, path).await)
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
