//! Completion relative to a `ReviewFilter` (SPEC §5.2).

use std::collections::BTreeMap;

use crate::content::{FileDiff, FileEntry, LineRange, RepoPath, ReviewFilter};
use crate::review::{ReviewedSpan, Side, SpanState};

/// A file-level change with no reviewable text (mode, symlink target, submodule
/// commit, binary or oversized content, pure rename). It is acknowledged as one
/// item, keyed by its before and after entries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NonTextChange {
    pub path: RepoPath,
    pub before: Option<FileEntry>,
    pub after: Option<FileEntry>,
    pub binary: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Completion {
    /// Changed lines passing the filter, counted on both sides.
    pub lines: u32,
    /// Of those, covered by a `Seen` span.
    pub seen_lines: u32,
    pub items: u32,
    pub acknowledged_items: u32,
    pub excluded_lines: u32,
    pub excluded_items: u32,
}

impl Completion {
    pub fn is_complete(&self) -> bool {
        self.seen_lines == self.lines && self.acknowledged_items == self.items
    }
}

impl ReviewFilter {
    pub fn passes(&self, path: &RepoPath) -> bool {
        let p = path.0.as_slice();
        let under = |root: &RepoPath| {
            p.starts_with(&root.0) && (p.len() == root.0.len() || p[root.0.len()] == b'/')
        };
        let matches = |pat: &String| glob_match(pat.as_bytes(), p, !pat.contains('/'));
        (self.roots.is_empty() || self.roots.iter().any(under))
            && (self.include.is_empty() || self.include.iter().any(matches))
            && !self.exclude.iter().any(matches)
    }
}

/// `*` and `?` stop at `/`, `**` does not. A pattern without `/` matches the
/// file name at any depth.
fn glob_match(pattern: &[u8], path: &[u8], basename: bool) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p {
            [] => t.is_empty(),
            [b'*', b'*', rest @ ..] => (0..=t.len()).any(|i| go(rest, &t[i..])),
            [b'*', rest @ ..] => (0..=t.iter().position(|&c| c == b'/').unwrap_or(t.len()))
                .any(|i| go(rest, &t[i..])),
            [b'?', rest @ ..] => t.first().is_some_and(|&c| c != b'/') && go(rest, &t[1..]),
            [c, rest @ ..] => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    let text = if basename {
        path.rsplit(|&c| c == b'/').next().unwrap_or(path)
    } else {
        path
    };
    go(pattern, text)
}

/// Spans must already be remapped into the current changeset. Only `Seen` spans
/// count; `acknowledged` holds the acknowledged non-text changes.
pub fn completion(
    filter: &ReviewFilter,
    diffs: &[FileDiff],
    non_text: &[NonTextChange],
    spans: &[ReviewedSpan],
    acknowledged: &[NonTextChange],
) -> Completion {
    let mut seen: BTreeMap<(&RepoPath, bool), Vec<LineRange>> = BTreeMap::new();
    for s in spans.iter().filter(|s| s.state == SpanState::Seen) {
        for a in s.base.iter().chain(&s.target) {
            seen.entry((&a.file, a.side == Side::Base))
                .or_default()
                .push(a.range);
        }
    }
    seen.values_mut().for_each(|v| v.sort_by_key(|r| r.start));

    let mut c = Completion {
        lines: 0,
        seen_lines: 0,
        items: 0,
        acknowledged_items: 0,
        excluded_lines: 0,
        excluded_items: 0,
    };
    for d in diffs {
        let Some(path) = d.new_path.as_ref().or(d.old_path.as_ref()) else {
            continue;
        };
        let passes = filter.passes(path);
        for h in &d.hunks {
            for (file, base, range) in [(&d.old_path, true, h.old), (&d.new_path, false, h.new)] {
                let Some(file) = file else { continue };
                if !passes {
                    c.excluded_lines += range.len();
                    continue;
                }
                c.lines += range.len();
                c.seen_lines += seen.get(&(file, base)).map_or(0, |v| covered(v, range));
            }
        }
    }
    for n in non_text {
        if !filter.passes(&n.path) || (filter.hide_binary && n.binary) {
            c.excluded_items += 1;
        } else {
            c.items += 1;
            c.acknowledged_items += acknowledged.contains(n) as u32;
        }
    }
    c
}

/// Lines of `r` inside `sorted` ranges, which may overlap.
fn covered(sorted: &[LineRange], r: LineRange) -> u32 {
    let mut total = 0;
    let mut from = r.start;
    for s in sorted {
        let start = s.start.max(from);
        let end = s.end.min(r.end);
        if start < end {
            total += end - start;
            from = end;
        }
    }
    total
}
