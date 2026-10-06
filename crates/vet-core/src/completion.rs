//! Completion relative to a `ReviewFilter` (SPEC §5.2).

use std::collections::BTreeMap;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::content::{FileDiff, LineRange, NonTextChange, NonTextKind, RepoPath, ReviewFilter};
use crate::review::{ReviewedSpan, Side, SpanState};

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
    /// Compiles the globs once. Invalid patterns match nothing.
    pub fn compile(&self) -> CompiledFilter<'_> {
        CompiledFilter {
            roots: &self.roots,
            include: (!self.include.is_empty()).then(|| GlobList::new(&self.include)),
            exclude: GlobList::new(&self.exclude),
        }
    }
}

/// A [`ReviewFilter`] with its globs compiled. `*` and `?` stop at `/`, `**`
/// does not, and a pattern without `/` matches the file name at any depth.
/// Paths are matched as UTF-8 with invalid bytes replaced by U+FFFD.
pub struct CompiledFilter<'a> {
    roots: &'a [RepoPath],
    /// `None` includes everything.
    include: Option<GlobList>,
    exclude: GlobList,
}

impl CompiledFilter<'_> {
    pub fn passes(&self, path: &RepoPath) -> bool {
        let p = path.0.as_slice();
        let under = |root: &RepoPath| {
            p.starts_with(&root.0) && (p.len() == root.0.len() || p[root.0.len()] == b'/')
        };
        let text = String::from_utf8_lossy(p);
        (self.roots.is_empty() || self.roots.iter().any(under))
            && self.include.as_ref().is_none_or(|g| g.is_match(&text))
            && !self.exclude.is_match(&text)
    }
}

/// Patterns with a `/` match the whole path, the others the file name.
struct GlobList {
    path: GlobSet,
    name: GlobSet,
}

impl GlobList {
    fn new(patterns: &[String]) -> Self {
        let set = |with_slash: bool| {
            let mut b = GlobSetBuilder::new();
            for p in patterns.iter().filter(|p| p.contains('/') == with_slash) {
                if let Ok(g) = GlobBuilder::new(p).literal_separator(true).build() {
                    b.add(g);
                }
            }
            b.build().unwrap_or_else(|_| GlobSet::empty())
        };
        Self {
            path: set(true),
            name: set(false),
        }
    }

    fn is_match(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        self.path.is_match(path) || self.name.is_match(name)
    }
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
    let compiled = filter.compile();
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
        let passes = compiled.passes(path);
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
        if !compiled.passes(&n.path) || (filter.hide_binary && n.kind == NonTextKind::Binary) {
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
