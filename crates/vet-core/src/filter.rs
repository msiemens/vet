//! The review filter: a view over a changeset, never part of snapshot identity (SPEC §3).

use std::cell::OnceCell;
use std::collections::BTreeSet;

use globset::{ErrorKind, Glob, GlobBuilder, GlobSet, GlobSetBuilder};

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
        self.include
            .iter()
            .chain(&self.exclude)
            .try_for_each(|pattern| glob(pattern).map(drop))
    }

    /// Compiles the globs once. Invalid patterns match nothing. `generated` holds the paths of the
    /// diff being filtered that the repository's attributes or the files' headers mark as generated.
    pub fn compile<'a>(&'a self, generated: &'a BTreeSet<RepoPath>) -> CompiledFilter<'a> {
        let built_in = built_in_excludes(self.review_generated);
        CompiledFilter {
            roots: &self.roots,
            include: (!self.include.is_empty()).then(|| GlobList::new(self.include.iter().map(String::as_str))),
            exclude: GlobList::new(self.exclude.iter().map(String::as_str).chain(built_in.iter().copied())),
            apart: OnceCell::new(),
            patterns: &self.exclude,
            review_generated: self.review_generated,
            hide_binary: self.hide_binary,
            generated: (!self.review_generated).then_some(generated),
        }
    }
}

/// Whether `path` is `root` or lies below it; `root` is a directory as [`ReviewFilter::normalized`] leaves it.
pub fn is_under(path: &RepoPath, root: &RepoPath) -> bool {
    let p = path.0.as_slice();
    p.starts_with(&root.0) && (p.len() == root.0.len() || p[root.0.len()] == b'/')
}

/// Which rule of the filter leaves a path out.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Exclusion {
    /// Not under any of the review roots.
    OutsideRoots,
    /// Matches none of the include globs.
    NotIncluded,
    /// Matches this exclude glob of the filter.
    Pattern(String),
    /// A lockfile or a generated file, which the filter leaves out unless they are reviewed.
    Generated,
    /// A binary change, hidden by the filter.
    Binary,
}

/// A [`ReviewFilter`] with its globs compiled. `*` and `?` stop at `/`, `**`
/// does not, a pattern without `/` matches the file name at any depth, and a
/// leading `/` anchors a pattern at the root (as in `.gitignore`).
/// Paths are matched as UTF-8 with invalid bytes replaced by U+FFFD.
pub struct CompiledFilter<'a> {
    roots: &'a [RepoPath],
    /// `None` includes everything.
    include: Option<GlobList>,
    /// The filter's exclude globs and the built-in ones, as one set.
    exclude: GlobList,
    /// The filter's own exclude globs and the built-in ones, each apart, to name the rules that leave
    /// a path out. Compiled when the rules of a path are first asked for, which `passes` never does.
    apart: OnceCell<Apart<'a>>,
    patterns: &'a [String],
    review_generated: bool,
    hide_binary: bool,
    /// The paths that are generated, if they are excluded.
    generated: Option<&'a BTreeSet<RepoPath>>,
}

struct Apart<'a> {
    patterns: Vec<(&'a str, GlobList)>,
    built_in: GlobList,
}

impl CompiledFilter<'_> {
    /// Whether a non-text change is displayed and counted.
    pub fn shows(&self, change: &NonTextChange) -> bool {
        self.passes(&change.path) && !(self.hide_binary && change.kind == NonTextKind::Binary)
    }

    /// Whether `path` is under one of the roots, or there are none.
    fn in_roots(&self, path: &RepoPath) -> bool {
        self.roots.is_empty() || self.roots.iter().any(|root| is_under(path, root))
    }

    pub fn passes(&self, path: &RepoPath) -> bool {
        let text = String::from_utf8_lossy(&path.0);
        self.in_roots(path)
            && self.include.as_ref().is_none_or(|g| g.is_match(&text))
            && !self.exclude.is_match(&text)
            && !self.generated.is_some_and(|g| g.contains(path))
    }

    /// Every rule that leaves a non-text change out; none if it is shown.
    pub fn exclusions_of(&self, change: &NonTextChange) -> Vec<Exclusion> {
        let mut rules = self.exclusions(&change.path);
        if self.hide_binary && change.kind == NonTextKind::Binary {
            rules.push(Exclusion::Binary);
        }
        rules
    }

    /// Every rule that leaves `path` out, in the order of [`Exclusion`]; none exactly when `passes`.
    pub fn exclusions(&self, path: &RepoPath) -> Vec<Exclusion> {
        let text = String::from_utf8_lossy(&path.0);
        let mut rules = Vec::new();
        if !self.in_roots(path) {
            rules.push(Exclusion::OutsideRoots);
        }
        if !self.include.as_ref().is_none_or(|g| g.is_match(&text)) {
            rules.push(Exclusion::NotIncluded);
        }
        if self.exclude.is_match(&text) {
            let apart = self.apart.get_or_init(|| Apart {
                patterns: self
                    .patterns
                    .iter()
                    .map(|p| (p.as_str(), GlobList::new(std::iter::once(p.as_str()))))
                    .collect(),
                built_in: GlobList::new(built_in_excludes(self.review_generated).iter().copied()),
            });
            for (pattern, list) in &apart.patterns {
                let named = Exclusion::Pattern((*pattern).to_owned());
                if list.is_match(&text) && !rules.contains(&named) {
                    rules.push(named);
                }
            }
            if apart.built_in.is_match(&text) {
                rules.push(Exclusion::Generated);
            }
        }
        if self.generated.is_some_and(|g| g.contains(path)) && !rules.contains(&Exclusion::Generated) {
            rules.push(Exclusion::Generated);
        }
        rules.sort();
        rules
    }
}

/// One glob of a filter on its own, to ask which paths it matches, read as the filter reads it.
pub struct PatternMatcher(GlobList);

impl PatternMatcher {
    /// Fails with why the pattern is invalid, as one short line fit to show beside the pattern.
    pub fn new(pattern: &str) -> Result<Self, String> {
        build(pattern).map_err(|e| short_reason(e.kind()))?;
        Ok(Self(GlobList::new(std::iter::once(pattern))))
    }

    pub fn matches(&self, path: &RepoPath) -> bool {
        self.0.is_match(&String::from_utf8_lossy(&path.0))
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
            glob(pattern)?;
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

/// The exclude globs every filter has, unless generated files are reviewed.
fn built_in_excludes(review_generated: bool) -> &'static [&'static str] {
    if review_generated { &[] } else { GENERATED_NAMES }
}

/// The pattern to compile: paths are relative, so a root anchor is dropped.
fn anchored(pattern: &str) -> &str {
    pattern.strip_prefix('/').unwrap_or(pattern)
}

/// The compiled glob, or why the pattern is invalid, as a message fit to show.
fn glob(pattern: &str) -> Result<Glob, String> {
    build(pattern).map_err(|e| format!("invalid pattern \"{pattern}\": {}", e.kind()))
}

fn build(pattern: &str) -> Result<Glob, globset::Error> {
    GlobBuilder::new(anchored(pattern)).literal_separator(true).build()
}

/// Why a glob is invalid, in one short line for the field it was typed into.
fn short_reason(kind: &ErrorKind) -> String {
    match kind {
        ErrorKind::UnclosedClass => "Unclosed [ in the pattern.".into(),
        ErrorKind::InvalidRange(from, to) => format!("Invalid range {from}-{to}."),
        ErrorKind::UnopenedAlternates => "} without { in the pattern.".into(),
        ErrorKind::UnclosedAlternates => "Unclosed { in the pattern.".into(),
        ErrorKind::NestedAlternates => "Nested { } are not supported.".into(),
        ErrorKind::DanglingEscape => "The pattern ends with a backslash.".into(),
        _ => "Not a valid pattern.".into(),
    }
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
                if let Ok(g) = glob(p) {
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
    fn a_pattern_matcher_says_in_one_line_why_a_glob_is_invalid() {
        let reason = |pattern: &str| PatternMatcher::new(pattern).err();
        assert_eq!(reason("src/[a").as_deref(), Some("Unclosed [ in the pattern."));
        assert_eq!(reason("{a,b").as_deref(), Some("Unclosed { in the pattern."));
        assert_eq!(reason("a}").as_deref(), Some("} without { in the pattern."));
        assert_eq!(reason("[z-a]").as_deref(), Some("Invalid range z-a."));
        assert_eq!(reason("a\\").as_deref(), Some("The pattern ends with a backslash."));
        assert_eq!(reason("**/*.snap"), None);
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
    fn every_rule_that_leaves_a_path_out_is_named_and_none_exactly_when_it_passes() {
        let path = |p: &str| RepoPath::from(p);
        let generated = BTreeSet::from([path("src/api.rs")]);
        let filter = ReviewFilter {
            roots: vec![path("src")],
            include: vec!["*.rs".into(), "*.lock".into()],
            exclude: vec!["**/*.snap.rs".into(), "t/**".into(), "**/x.*".into()],
            ..Default::default()
        };
        let compiled = filter.compile(&generated);
        let named = |p: &str| compiled.exclusions(&path(p));
        let pattern = |p: &str| Exclusion::Pattern(p.into());
        assert_eq!(named("docs/a.rs"), [Exclusion::OutsideRoots]);
        assert_eq!(named("src/a.md"), [Exclusion::NotIncluded]);
        assert_eq!(
            named("src/t/x.snap.rs"),
            [pattern("**/*.snap.rs"), pattern("**/x.*")],
            "two globs leave it out, and `t/**` is anchored at the root"
        );
        assert_eq!(
            named("docs/Cargo.lock"),
            [Exclusion::OutsideRoots, Exclusion::Generated]
        );
        assert_eq!(named("src/api.rs"), [Exclusion::Generated]);
        assert_eq!(named("src/lib.rs"), []);
        for p in [
            "docs/a.rs",
            "src/a.md",
            "src/t/x.snap.rs",
            "src/Cargo.lock",
            "src/api.rs",
            "src/lib.rs",
        ] {
            assert_eq!(compiled.passes(&path(p)), named(p).is_empty(), "{p}");
        }
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
