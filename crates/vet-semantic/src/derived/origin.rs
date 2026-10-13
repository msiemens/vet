//! Which diagnostics a change introduced (§7.3): the target's diagnostics of a file against the
//! base's. A target diagnostic is pre-existing if the base has one with equal severity, source,
//! code and message (whitespace-collapsed) on a line that maps to it, and new otherwise. While the
//! base side cannot be had the origin is unknown, never new.

use vet_core::content::{Hunk, RepoPath};
use vet_core::review::Side;

use super::{Derived, Inputs};
use crate::join::join;
use crate::ports::Diagnostic;
use crate::query::{Asked, Primitives, SemanticError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    /// The change introduced it.
    New,
    /// The base has it too, where the change leaves that line.
    Existing,
    /// The base side's diagnostics are not available.
    Unknown,
}

/// A target-side diagnostic and where it comes from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Judged {
    pub diagnostic: Diagnostic,
    pub origin: Origin,
}

/// The target-side diagnostics of a file with their origins.
#[derive(Debug)]
pub struct Judgement {
    pub diagnostics: Vec<Judged>,
    /// Why the base side's diagnostics are missing, if they are: every origin is then unknown.
    pub base: Option<SemanticError>,
}

/// The diagnostics of the file `new` on the target side, each with its origin against the diagnostics
/// of `old` on the base side (`None`: the file is new, so the base has none). `hunks` are the file's
/// text hunks, none for a file the change leaves alone.
pub async fn judged(
    p: &impl Primitives,
    old: Option<&RepoPath>,
    new: &RepoPath,
    hunks: &[Hunk],
) -> Derived<Result<Judgement, SemanticError>> {
    let mut inputs = Inputs::new("diagnostic origin");
    inputs.record_hunks(&new.to_string_lossy(), hunks);
    let base_side = async {
        match old {
            Some(old) => Some(p.diagnostics(Side::Base, old).await),
            None => None,
        }
    };
    let (target, base) = join(p.diagnostics(Side::Target, new), base_side).await;
    inputs.query(
        "diagnostics",
        Side::Target,
        new,
        None,
        target.as_ref().ok().map(|a| &a.provenance),
    );
    if let (Some(old), Some(base)) = (old, &base) {
        inputs.query(
            "diagnostics",
            Side::Base,
            old,
            None,
            base.as_ref().ok().map(|a| &a.provenance),
        );
    }
    let value = target.map(|target| judge(target.value, base, hunks));
    Derived { value, inputs }
}

fn judge(target: Vec<Diagnostic>, base: Option<Asked<Vec<Diagnostic>>>, hunks: &[Hunk]) -> Judgement {
    match base.map_or(Ok(Vec::new()), |base| base.map(|answered| answered.value)) {
        Ok(base) => {
            let origins = origins(&target, &base, hunks);
            Judgement {
                diagnostics: (target.into_iter().zip(origins))
                    .map(|(diagnostic, origin)| Judged { diagnostic, origin })
                    .collect(),
                base: None,
            }
        }
        Err(e) => Judgement {
            diagnostics: (target.into_iter())
                .map(|diagnostic| Judged {
                    diagnostic,
                    origin: Origin::Unknown,
                })
                .collect(),
            base: Some(e),
        },
    }
}

/// The origin of each of the `target` diagnostics against the `base` ones. A base diagnostic
/// vouches for one target diagnostic only, so two equal ones on a line are not both explained by one.
fn origins(target: &[Diagnostic], base: &[Diagnostic], hunks: &[Hunk]) -> Vec<Origin> {
    let mut taken = vec![false; base.len()];
    target
        .iter()
        .map(|t| {
            let found = base_line(hunks, t.span.start.line).and_then(|line| {
                (base.iter().enumerate()).find(|(i, b)| !taken[*i] && b.span.start.line == line && same(t, b))
            });
            match found {
                Some((i, _)) => {
                    taken[i] = true;
                    Origin::Existing
                }
                None => Origin::New,
            }
        })
        .collect()
}

fn same(a: &Diagnostic, b: &Diagnostic) -> bool {
    a.severity == b.severity
        && a.source == b.source
        && a.code == b.code
        && collapsed(&a.message) == collapsed(&b.message)
}

fn collapsed(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The base line the target `line` stands for; none for a line a hunk changed. `hunks` are in order.
fn base_line(hunks: &[Hunk], line: u32) -> Option<u32> {
    let mut shift = i64::from(line);
    for hunk in hunks {
        if hunk.new.start > line {
            break;
        }
        if line < hunk.new.end {
            return None;
        }
        shift += i64::from(hunk.old.len()) - i64::from(hunk.new.len());
    }
    u32::try_from(shift).ok()
}
