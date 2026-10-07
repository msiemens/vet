//! What a hunk seen against an old base looked like on the base side when it was marked, next to
//! what the base has there now (§5.2).

use vet_core::completion::HunkState;
use vet_core::content::{LineRange, SnapshotId};
use vet_core::hunks::changed_lines;
use vet_core::review::{Anchor, Side, SpanState};
use vet_core::text::Lines;

use super::interdiff::{View, update_in};
use super::state::{State, check_revision, find_diff};
use super::{LiveSession, Result, SessionError};
use crate::ports::{CommitInfo, GitRepo, LiveStore};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BaseDiffKind {
    Context,
    Removed,
    Added,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BaseDiffRow {
    pub kind: BaseDiffKind,
    /// 1-based, in the old base for a removed line and in the current base for the others.
    pub line: u32,
    /// The line without its terminator.
    pub text: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BaseDiffView {
    /// The base the hunk was marked against; `None` once git has dropped the commit.
    pub from: Option<CommitInfo>,
    /// The base now.
    pub to: CommitInfo,
    pub rows: Vec<BaseDiffRow>,
}

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// The base-side lines of the hunk `hunk` of `path` as they were when it was marked seen, as
    /// a diff to the lines the base has now. Fails if the hunk is not seen against an old base.
    pub async fn base_diff(&self, revision: u32, path: &str, hunk: u32) -> Result<BaseDiffView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let state = update_in(&st, View::Changeset, path)?
            .review
            .hunks
            .get(hunk as usize)
            .copied();
        match state {
            None => return Err(SessionError::NotFound(format!("hunk {hunk}"))),
            Some(HunkState::SeenAgainstOldBase) => {}
            Some(_) => {
                return Err(SessionError::Invalid(
                    "this hunk was not seen against an old base".into(),
                ));
            }
        }
        let (old, new, from_snapshot) = base_texts(&st, path, hunk as usize)?;
        let rows = rows(&old.text, &new.text, old.start, new.start);
        let from = self.commit_of_base(&st, from_snapshot).await?;
        let to = self.acquisition.git.commit_info(&st.base_commit).await?;
        Ok(BaseDiffView { from, to, rows })
    }

    /// The commit a version of the open review had as its base, if one had `snapshot`.
    async fn commit_of_base(&self, st: &State, snapshot: SnapshotId) -> Result<Option<CommitInfo>> {
        let Some(changeset) = st.changeset else {
            return Ok(None);
        };
        let versions = self.acquisition.store.versions(changeset).await?;
        let Some(version) = versions.iter().find(|v| v.base == snapshot) else {
            return Ok(None);
        };
        Ok(self.acquisition.git.commit_info(&version.base_commit).await.ok())
    }
}

/// Lines of a file's text and the 0-based line the first of them is.
struct Excerpt {
    text: String,
    start: u32,
}

/// The base lines recorded for hunk `hunk` of `path` and the base's lines there now, with the
/// snapshot the recorded ones are of.
fn base_texts(st: &State, path: &str, hunk: usize) -> Result<(Excerpt, Excerpt, SnapshotId)> {
    let changes = st.changes.as_ref().expect("computed");
    let diff = find_diff(&changes.diff, path).ok_or_else(|| SessionError::NotFound(format!("file {path}")))?;
    let h = diff
        .hunks
        .get(hunk)
        .ok_or_else(|| SessionError::NotFound(format!("hunk {hunk}")))?;
    let text_of = |blob| {
        changes
            .blobs
            .get(&blob)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    };
    let unavailable = || SessionError::Invalid("the base this was seen against is not available".into());

    let mut anchors: Vec<&Anchor> = Vec::new();
    for (rec, (_, pieces)) in st.recorded.iter().zip(&changes.remapped) {
        let covers = pieces.iter().any(|p| {
            p.state == SpanState::SeenAgainstOldBase
                && p.target.as_ref().is_some_and(|t| {
                    Some(&t.file) == diff.new_path.as_ref()
                        && t.side == Side::Target
                        && t.range.start < h.new.end.max(h.new.start + 1)
                        && h.new.start < t.range.end
                })
        });
        if let (true, Some(base)) = (covers, &rec.span.base)
            && !anchors.contains(&base)
        {
            anchors.push(base);
        }
    }
    anchors.sort_by(|a, b| (a.snapshot, &a.file, a.range.start).cmp(&(b.snapshot, &b.file, b.range.start)));
    let first = anchors.first().ok_or_else(unavailable)?;

    let mut old = String::new();
    for a in &anchors {
        let blob = st
            .snapshot(a.snapshot)
            .and_then(|s| s.entries.get(&a.file))
            .map(|e| e.blob)
            .ok_or_else(unavailable)?;
        let text = text_of(blob).ok_or_else(unavailable)?;
        old.push_str(Lines::new(&text).slice(a.range));
    }
    let new = diff
        .old_blob
        .and_then(text_of)
        .map(|t| Lines::new(&t).slice(h.old).to_owned())
        .unwrap_or_default();
    Ok((
        Excerpt {
            text: old,
            start: first.range.start,
        },
        Excerpt {
            text: new,
            start: h.old.start,
        },
        first.snapshot,
    ))
}

/// The lines of `old` and `new` as rows: what is the same, what only the old text has, and what
/// only the new one has. `old_start` and `new_start` are the 0-based lines the texts begin at.
fn rows(old: &str, new: &str, old_start: u32, new_start: u32) -> Vec<BaseDiffRow> {
    let (old_lines, new_lines) = (Lines::new(old), Lines::new(new));
    let line_of = |lines: &Lines, i: u32| {
        lines
            .slice(LineRange { start: i, end: i + 1 })
            .trim_end_matches(['\n', '\r'])
            .to_owned()
    };
    let row = |kind, line: u32, text: String| BaseDiffRow { kind, line, text };
    let mut rows = Vec::new();
    let mut at = 0;
    let context_to = |rows: &mut Vec<BaseDiffRow>, end: u32, at: &mut u32| {
        for i in *at..end {
            rows.push(row(BaseDiffKind::Context, new_start + i + 1, line_of(&new_lines, i)));
        }
    };
    for h in changed_lines(old.as_bytes(), new.as_bytes()) {
        context_to(&mut rows, h.new.start, &mut at);
        for i in h.old.start..h.old.end {
            rows.push(row(BaseDiffKind::Removed, old_start + i + 1, line_of(&old_lines, i)));
        }
        for i in h.new.start..h.new.end {
            rows.push(row(BaseDiffKind::Added, new_start + i + 1, line_of(&new_lines, i)));
        }
        at = h.new.end;
    }
    context_to(&mut rows, new_lines.starts().len().saturating_sub(1) as u32, &mut at);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(rows: &[BaseDiffRow]) -> Vec<String> {
        rows.iter()
            .map(|r| {
                let mark = match r.kind {
                    BaseDiffKind::Context => ' ',
                    BaseDiffKind::Removed => '-',
                    BaseDiffKind::Added => '+',
                };
                format!("{mark}{} {}", r.line, r.text)
            })
            .collect()
    }

    #[test]
    fn rows_number_the_old_lines_in_the_old_base_and_the_others_in_the_current_one() {
        let rows = rows("a\nb\nc\n", "a\nB\nc\nd\n", 10, 20);
        assert_eq!(shown(&rows), [" 21 a", "-12 b", "+22 B", " 23 c", "+24 d"]);
    }
}
