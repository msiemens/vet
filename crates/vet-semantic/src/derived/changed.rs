//! Which symbols changed (§7.3): `documentSymbol` on both sides of each changed
//! file, intersected with the diff hunks.

use std::sync::atomic::{AtomicUsize, Ordering};

use vet_core::content::{FileDiff, LineRange, RepoPath};
use vet_core::review::Side;

use super::{Derived, Inputs};
use crate::join::{bounded, join};
use crate::ports::{Provenance, SymbolInfo};
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

/// A file whose symbols could not be determined.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Gap {
    pub file: RepoPath,
    pub reason: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Changes {
    pub symbols: Vec<ChangedSymbol>,
    /// Files a backend failed on; their symbols are unknown, not unchanged. Files no configured
    /// backend handles are not here: they have no symbols to know, and the footer reports them.
    pub gaps: Vec<Gap>,
}

/// Files whose texts are read at once, both sides each. It only bounds the texts held
/// while their questions wait their turn: how many requests run on a server is the
/// pool's to decide (`pool::REQUESTS_PER_SERVER`), over all that ask it.
const FILES_READ_AHEAD: usize = 16;

/// The file contents of both sides, so their symbols can be had from the texts alone:
/// without a server on the base snapshot, and without the target file as the live
/// tree has it now, which may have moved on from the revision.
pub trait Texts: Sync {
    /// The base content of `file`; `None` if it is not available.
    fn base_text(&self, file: &FileDiff) -> impl Future<Output = Option<String>> + Send;
    /// The target content of `file`; `None` if it is not available.
    fn target_text(&self, file: &FileDiff) -> impl Future<Output = Option<String>> + Send;
}

/// `on_file(done, total)` is called each time a file's symbols are in, `total` being
/// the files with changes to look at.
pub async fn changed_symbols(
    p: &impl Primitives,
    texts: &impl Texts,
    files: &[FileDiff],
    on_file: &(impl Fn(usize, usize) + Sync),
) -> Derived<Changes> {
    let mut inputs = Inputs::new("changed symbols");
    let mut out = Changes::default();
    let files: Vec<_> = files.iter().filter(|f| !f.hunks.is_empty()).collect();
    for file in &files {
        inputs.record_hunks(&file.path().to_string_lossy(), &file.hunks);
    }
    let total = files.len();
    let done = AtomicUsize::new(0);
    let answers = bounded(FILES_READ_AHEAD, 0..total, |at| {
        let file = files[at];
        let done = &done;
        async move {
            let answer = join(base_symbols(p, texts, file), target_symbols(p, texts, file)).await;
            on_file(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            answer
        }
    })
    .await;
    for (file, (base, target)) in files.into_iter().zip(answers) {
        let shown = file.path();
        let old: Vec<_> = file.hunks.iter().map(|h| h.old).collect();
        let new: Vec<_> = file.hunks.iter().map(|h| h.new).collect();
        for (side, path, answer) in [
            (Side::Base, &file.old_path, &base),
            (Side::Target, &file.new_path, &target),
        ] {
            if let Some(path) = path {
                let by = answer.as_ref().ok().and_then(|(_, by)| by.as_ref());
                inputs.query("document_symbols", side, path, None, by);
            }
        }
        let (base, target) = match (base, target) {
            (Ok((b, _)), Ok((t, _))) => (b, t),
            (Err(e), _) | (_, Err(e)) => {
                if !matches!(e, SemanticError::NoBackend(_)) {
                    out.gaps.push(Gap {
                        file: shown.clone(),
                        reason: e.to_string(),
                    });
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
