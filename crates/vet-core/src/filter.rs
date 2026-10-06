//! The review filter: a view over a changeset, never part of snapshot identity (SPEC §3).

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::content::RepoPath;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ReviewFilter {
    pub roots: Vec<RepoPath>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub hide_binary: bool,
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
        let under = |root: &RepoPath| p.starts_with(&root.0) && (p.len() == root.0.len() || p[root.0.len()] == b'/');
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
