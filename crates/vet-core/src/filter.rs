//! The review filter: a view over a changeset, never part of snapshot identity (SPEC §3).

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::content::{NonTextChange, NonTextKind, RepoPath};

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ReviewFilter {
    pub roots: Vec<RepoPath>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub hide_binary: bool,
}

impl ReviewFilter {
    /// The first invalid glob of `include` and `exclude`, as a message fit to show.
    pub fn validate(&self) -> Result<(), String> {
        for pattern in self.include.iter().chain(&self.exclude) {
            if let Err(e) = GlobBuilder::new(anchored(pattern)).literal_separator(true).build() {
                return Err(format!("invalid pattern \"{pattern}\": {}", e.kind()));
            }
        }
        Ok(())
    }

    /// Compiles the globs once. Invalid patterns match nothing.
    pub fn compile(&self) -> CompiledFilter<'_> {
        CompiledFilter {
            roots: &self.roots,
            include: (!self.include.is_empty()).then(|| GlobList::new(&self.include)),
            exclude: GlobList::new(&self.exclude),
            hide_binary: self.hide_binary,
        }
    }
}

/// A [`ReviewFilter`] with its globs compiled. `*` and `?` stop at `/`, `**`
/// does not, a pattern without `/` matches the file name at any depth, and a
/// leading `/` anchors a pattern at the root (as in `.gitignore`).
/// Paths are matched as UTF-8 with invalid bytes replaced by U+FFFD.
pub struct CompiledFilter<'a> {
    roots: &'a [RepoPath],
    /// `None` includes everything.
    include: Option<GlobList>,
    exclude: GlobList,
    hide_binary: bool,
}

impl CompiledFilter<'_> {
    /// Whether a non-text change is displayed and counted.
    pub fn shows(&self, change: &NonTextChange) -> bool {
        self.passes(&change.path) && !(self.hide_binary && change.kind == NonTextKind::Binary)
    }

    pub fn passes(&self, path: &RepoPath) -> bool {
        let p = path.0.as_slice();
        let under = |root: &RepoPath| p.starts_with(&root.0) && (p.len() == root.0.len() || p[root.0.len()] == b'/');
        let text = String::from_utf8_lossy(p);
        (self.roots.is_empty() || self.roots.iter().any(under))
            && self.include.as_ref().is_none_or(|g| g.is_match(&text))
            && !self.exclude.is_match(&text)
    }
}

/// The pattern to compile: paths are relative, so a root anchor is dropped.
fn anchored(pattern: &str) -> &str {
    pattern.strip_prefix('/').unwrap_or(pattern)
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
                if let Ok(g) = GlobBuilder::new(anchored(p)).literal_separator(true).build() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_globs_are_named() {
        let ok = ReviewFilter {
            include: vec!["*.rs".into()],
            exclude: vec!["**/gen/*".into()],
            ..Default::default()
        };
        assert_eq!(ok.validate(), Ok(()));
        let bad = ReviewFilter {
            exclude: vec!["src/[a".into()],
            ..Default::default()
        };
        assert!(bad.validate().unwrap_err().starts_with("invalid pattern \"src/[a\""));
    }

    #[test]
    fn a_leading_slash_anchors_at_the_root() {
        let path = |p: &str| RepoPath(p.as_bytes().to_vec());
        let filter = ReviewFilter {
            exclude: vec!["/top.txt".into(), "any.txt".into()],
            ..Default::default()
        };
        let compiled = filter.compile();
        assert!(!compiled.passes(&path("top.txt")));
        assert!(compiled.passes(&path("src/top.txt")));
        assert!(!compiled.passes(&path("src/any.txt")));
        assert_eq!(filter.validate(), Ok(()));
    }
}
