//! Impact of a removal (§7.3): the remaining references of each removed symbol
//! on the base, each classified against the target.

use vet_core::content::FileDiff;

use super::changed::{Change, ChangedSymbol};
use super::{Derived, Inputs, path};
use crate::ports::{At, Location, Pos, SymbolInfo};
use crate::query::{Primitives, Which};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fate {
    /// The call site's lines were deleted.
    Gone,
    /// The call site's lines were edited.
    Modified,
    /// Unchanged text that no longer resolves.
    LikelyBroken,
    /// Unchanged text that now resolves to something else.
    BindingChanged { now: Vec<Location> },
    Unknown(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallSite {
    /// Where the reference is on the base side.
    pub base: Location,
    pub fate: Fate,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RemovalImpact {
    pub symbol: SymbolInfo,
    pub call_sites: Vec<CallSite>,
    /// `Some` when the references could not be listed: the call sites are unknown, not absent.
    pub gap: Option<String>,
}

pub async fn removal_impact(
    p: &impl Primitives,
    changed: &[ChangedSymbol],
    files: &[FileDiff],
) -> Derived<Vec<RemovalImpact>> {
    let mut inputs = Inputs::new("removal impact");
    let mut out = Vec::new();
    for c in changed.iter().filter(|c| c.change == Change::Removed) {
        let at = c.symbol.location();
        inputs.query_at("references", Which::Base, &at);
        let refs = match p.references(Which::Base, &at).await {
            Ok(refs) => refs,
            Err(e) => {
                out.push(RemovalImpact {
                    symbol: c.symbol.clone(),
                    call_sites: Vec::new(),
                    gap: Some(e.to_string()),
                });
                continue;
            }
        };
        let mut call_sites = Vec::new();
        for base in refs {
            let fate = match locate(files, &base) {
                Located::Fate(fate) => fate,
                Located::Same(at) => {
                    inputs.query_at("definition", Which::Target, &at);
                    match p.definition(Which::Target, &at).await {
                        Ok(now) if now.is_empty() => Fate::LikelyBroken,
                        Ok(now) => Fate::BindingChanged { now },
                        Err(e) => Fate::Unknown(e.to_string()),
                    }
                }
            };
            call_sites.push(CallSite { base, fate });
        }
        out.push(RemovalImpact {
            symbol: c.symbol.clone(),
            call_sites,
            gap: None,
        });
    }
    for file in files {
        if let Some(old) = &file.old_path {
            inputs.hunks.push(format!(
                "{}: {}",
                path(old),
                file.hunks
                    .iter()
                    .map(|h| format!("old {}..{} new {}..{}", h.old.start, h.old.end, h.new.start, h.new.end))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Derived { value: out, inputs }
}

enum Located {
    Fate(Fate),
    /// The text is identical; this is where it is in the target.
    Same(At),
}

/// Maps a base location through the diff using hunks alone.
fn locate(files: &[FileDiff], base: &Location) -> Located {
    let Some(file) = files.iter().find(|f| f.old_path.as_ref() == Some(&base.file)) else {
        return Located::Same(At {
            file: base.file.clone(),
            pos: base.span.start,
        });
    };
    let Some(new_path) = &file.new_path else {
        return Located::Fate(Fate::Gone);
    };
    let lines = base.span.lines();
    let mut touched = 0;
    let mut deleted = 0;
    for line in lines.start..lines.end {
        let hit = file.hunks.iter().find(|h| !h.old.is_empty() && h.old.start <= line && line < h.old.end);
        if let Some(h) = hit {
            touched += 1;
            if h.new.is_empty() {
                deleted += 1;
            }
        }
    }
    if touched > 0 {
        return Located::Fate(if deleted == touched { Fate::Gone } else { Fate::Modified });
    }
    let shift: i64 = file
        .hunks
        .iter()
        .filter(|h| h.old.start <= lines.start)
        .map(|h| i64::from(h.new.len()) - i64::from(h.old.len()))
        .sum();
    let line = i64::from(base.span.start.line) + shift;
    Located::Same(At {
        file: new_path.clone(),
        pos: Pos {
            line: u32::try_from(line).unwrap_or(0),
            col: base.span.start.col,
        },
    })
}
