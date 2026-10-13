//! The review filter: a view over a changeset, never part of snapshot identity (SPEC §3).

use std::collections::BTreeSet;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::content::{NonTextChange, NonTextKind, RepoPath};
use crate::generated::GENERATED_NAMES;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ReviewFilter {
    pub roots: Vec<RepoPath>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub hide_binary: bool,
    /// Lockfiles and generated files are reviewed like any other file. Otherwise they are excluded:
    /// by name, by `linguist-generated` in `.gitattributes`, and by a header that says a tool wrote
    /// the file (see [`crate::generated`]).
    pub review_generated: bool,
}

impl ReviewFilter {
    /// The filter with its roots as `passes` expects them: directories without a leading or
    /// trailing `/`. A root that names no directory is dropped.
    pub fn normalized(mut self) -> Self {
        for root in &mut self.roots {
            let start = root.0.iter().take_while(|&&b| b == b'/').count();
            let end = root.0.len() - root.0[start..].iter().rev().take_while(|&&b| b == b'/').count();
            root.0 = root.0[start..end].to_vec();
        }
        self.roots.retain(|root| !root.0.is_empty());
        self
    }

    /// The first invalid glob of `include` and `exclude`, as a message fit to show.
    pub fn validate(&self) -> Result<(), String> {
        for pattern in self.include.iter().chain(&self.exclude) {
            if let Err(e) = GlobBuilder::new(anchored(pattern)).literal_separator(true).build() {
                return Err(format!("invalid pattern \"{pattern}\": {}", e.kind()));
            }
        }
        Ok(())
    }

    /// Compiles the globs once. Invalid patterns match nothing. `generated` holds the paths of the
    /// diff being filtered that the repository's attributes or the files' headers mark as generated.
    pub fn compile<'a>(&'a self, generated: &'a BTreeSet<RepoPath>) -> CompiledFilter<'a> {
        let built_in = if self.review_generated {
            &[][..]
        } else {
            GENERATED_NAMES
        };
        CompiledFilter {
            roots: &self.roots,
            include: (!self.include.is_empty()).then(|| GlobList::new(self.include.iter().map(String::as_str))),
            exclude: GlobList::new(self.exclude.iter().map(String::as_str).chain(built_in.iter().copied())),
            hide_binary: self.hide_binary,
            generated: (!self.review_generated).then_some(generated),
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
    /// The paths that are generated, if they are excluded.
    generated: Option<&'a BTreeSet<RepoPath>>,
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
            && !self.generated.is_some_and(|g| g.contains(path))
    }
}

/// The files a text search looks in, as the viewer writes them: globs separated by commas, a leading
/// `!` makes one exclude. Globs read as the review filter's do (a pattern without `/` matches the
/// file name at any depth). An empty mask takes every file.
pub struct FileMask {
    /// `None` takes every file.
    include: Option<GlobList>,
    exclude: GlobList,
}

impl FileMask {
    /// Fails with the glob that is invalid.
    pub fn parse(mask: &str) -> Result<Self, String> {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for token in mask.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let (list, pattern) = match token.strip_prefix('!') {
                Some(pattern) => (&mut exclude, pattern.trim()),
                None => (&mut include, token),
            };
            if let Err(e) = GlobBuilder::new(anchored(pattern)).literal_separator(true).build() {
                return Err(format!("invalid pattern \"{pattern}\": {}", e.kind()));
            }
            list.push(pattern);
        }
        Ok(Self {
            include: (!include.is_empty()).then(|| GlobList::new(include.iter().copied())),
            exclude: GlobList::new(exclude.iter().copied()),
        })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.include.as_ref().is_none_or(|i| i.is_match(path)) && !self.exclude.is_match(path)
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
    fn new<'p>(patterns: impl Iterator<Item = &'p str> + Clone) -> Self {
        let set = |with_slash: bool| {
            let mut b = GlobSetBuilder::new();
            for p in patterns.clone().filter(|p| p.contains('/') == with_slash) {
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
    fn a_file_mask_includes_by_name_or_path_and_excludes_with_a_bang() {
        let mask = FileMask::parse("*.rs, src/**/*.md, !tests/**").unwrap();
        assert!(mask.matches("src/sync/retry.rs"));
        assert!(mask.matches("retry.rs"));
        assert!(mask.matches("src/docs/guide.md"));
        assert!(!mask.matches("README.md"));
        assert!(!mask.matches("tests/retry.rs"));
        // Only exclusions: everything else passes.
        let only_out = FileMask::parse("!*.lock").unwrap();
        assert!(only_out.matches("src/a.rs"));
        assert!(!only_out.matches("Cargo.lock"));
        assert!(FileMask::parse("").unwrap().matches("anything/at/all"));
    }

    #[test]
    fn an_invalid_mask_names_its_glob() {
        let error = FileMask::parse("*.rs, src/[a").err().unwrap();
        assert!(error.starts_with("invalid pattern \"src/[a\""));
    }

    #[test]
    fn roots_are_directories_without_slashes_at_their_ends() {
        let filter = ReviewFilter {
            roots: ["/src/", "a/b/", "c", "/", "", "//d//e//"].map(RepoPath::from).into(),
            include: vec!["*.rs".into()],
            hide_binary: true,
            ..Default::default()
        };
        let normalized = filter.clone().normalized();
        assert_eq!(normalized.roots, ["src", "a/b", "c", "d//e"].map(RepoPath::from));
        assert_eq!((&normalized.include, normalized.hide_binary), (&filter.include, true));
        let passes = |filter: &ReviewFilter, p: &str| filter.compile(&BTreeSet::new()).passes(&RepoPath::from(p));
        assert!(passes(&normalized, "src/lib.rs"));
        assert!(!passes(&normalized, "srcs/lib.rs"));
        // No root left: everything is under one.
        let none = ReviewFilter {
            roots: vec![RepoPath::from("/")],
            ..Default::default()
        };
        assert!(passes(&none.normalized(), "any/file"));
    }

    #[test]
    fn a_leading_slash_anchors_at_the_root() {
        let path = |p: &str| RepoPath::from(p);
        let filter = ReviewFilter {
            exclude: vec!["/top.txt".into(), "any.txt".into()],
            ..Default::default()
        };
        let none = BTreeSet::new();
        let compiled = filter.compile(&none);
        assert!(!compiled.passes(&path("top.txt")));
        assert!(compiled.passes(&path("src/top.txt")));
        assert!(!compiled.passes(&path("src/any.txt")));
        assert_eq!(filter.validate(), Ok(()));
    }

    #[test]
    fn lockfiles_and_generated_paths_are_excluded_unless_they_are_reviewed() {
        let path = |p: &str| RepoPath::from(p);
        let generated = BTreeSet::from([path("src/api.rs")]);
        let lockfile = [
            "Cargo.lock",
            "web/app/yarn.lock",
            "src/Api.Designer.cs",
            "dist/app.min.js",
        ];
        let hand_written = ["src/lib.rs", "docs/lock.md", "src/cargo.lock.rs"];
        let filter = ReviewFilter::default();
        let compiled = filter.compile(&generated);
        assert!(lockfile.iter().all(|p| !compiled.passes(&path(p))));
        assert!(!compiled.passes(&path("src/api.rs")));
        assert!(hand_written.iter().all(|p| compiled.passes(&path(p))));

        let reviewing = ReviewFilter {
            review_generated: true,
            ..Default::default()
        };
        let compiled = reviewing.compile(&generated);
        assert!(lockfile.iter().all(|p| compiled.passes(&path(p))));
        assert!(compiled.passes(&path("src/api.rs")));
    }
}
