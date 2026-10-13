//! Text search over the files of a view, or over the whole target tree.

use std::collections::BTreeMap;

use vet_core::completion::{Completion, HunkState};
use vet_core::content::{EntryKind, Hunk, RepoPath};
use vet_core::filter::FileMask;

use super::views::{FileStatus, FileView};
use super::{LiveSession, NoText, Result, SessionError, View};
use crate::ports::{GitRepo, LiveStore};
use crate::search::{FoundLine, LineMatcher, SearchFilters, Sides, search_text};

/// The most matches a search lists, whatever limit it asks for.
pub const MAX_HITS: usize = 1000;

/// Where to look.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchScope {
    /// The changed files of a view, in the review filter's sight; both sides of each.
    View(View),
    /// Every file of the target tree, the changed ones with their review state. Ignored files are
    /// not in the tree. The review filter does not apply; the files it hides count as unchanged.
    Repository,
}

pub struct SearchQuery<'a> {
    pub scope: SearchScope,
    pub matcher: &'a dyn LineMatcher,
    pub mask: &'a FileMask,
    pub filters: SearchFilters,
    /// Stop after this many matches.
    pub limit: usize,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SearchFile {
    pub path: RepoPath,
    /// `None` for a file the view does not change.
    pub status: Option<FileStatus>,
    pub completion: Option<Completion>,
    /// The file's hunks and their review states, for a changed file.
    pub hunks: Vec<Hunk>,
    pub states: Vec<HunkState>,
    pub lines: Vec<FoundLine>,
    /// The matches in `lines`.
    pub hits: u32,
}

#[derive(Clone, PartialEq, Debug)]
pub struct SearchResult {
    pub revision: u32,
    /// The files with matches, in path order.
    pub files: Vec<SearchFile>,
    pub hits: u32,
    /// The search stopped at the limit.
    pub truncated: bool,
    /// How many files had text to look in.
    pub searched: u32,
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    pub async fn search(&self, query: &SearchQuery<'_>) -> Result<SearchResult> {
        let view = match query.scope {
            SearchScope::View(view) => view,
            SearchScope::Repository => View::Changeset,
        };
        let manifest = self.manifest(view).await?;
        let revision = manifest.revision;
        let mut out = Collector::new(revision, query.limit.clamp(1, MAX_HITS));
        match query.scope {
            SearchScope::View(view) => {
                for file in &manifest.files {
                    let path = file.path.to_string_lossy();
                    if !query.mask.matches(&path) {
                        continue;
                    }
                    // A search that was given up on stops here, and the others get their turn.
                    tokio::task::yield_now().await;
                    let text = match self.file_text(view, &path).await {
                        Ok(text) => text,
                        Err(SessionError::NotText(_)) => continue,
                        Err(e) => return Err(e),
                    };
                    if text.revision != revision {
                        return Err(SessionError::Stale { current: text.revision });
                    }
                    let found = search_text(
                        text.base.as_deref().unwrap_or_default(),
                        text.target.as_deref().unwrap_or_default(),
                        &file.hunks,
                        &file.review.hunks,
                        query.matcher,
                        query.filters,
                        Sides::Both,
                    );
                    out.file(file.path.clone(), Some(file), found);
                    if out.full() {
                        break;
                    }
                }
            }
            SearchScope::Repository => {
                let changed: BTreeMap<&RepoPath, _> = manifest.files.iter().map(|f| (&f.path, f)).collect();
                let st = self.published.read();
                if st.revision != revision {
                    return Err(SessionError::Stale { current: st.revision });
                }
                let skip_unchanged = query.filters.only_changes || query.filters.not_seen;
                for (path, entry) in &st.target.entries {
                    let name = path.to_string_lossy();
                    if entry.kind != EntryKind::File || !query.mask.matches(&name) {
                        continue;
                    }
                    let file = changed.get(path).copied();
                    if file.is_none() && skip_unchanged {
                        continue;
                    }
                    tokio::task::yield_now().await;
                    let text = match self.target_text(View::Changeset, &name).await?.content {
                        Ok(text) => text,
                        Err(NoText::Binary | NoText::Unavailable | NoText::NotAFile | NoText::Absent) => continue,
                    };
                    let (hunks, states): (&[_], &[HunkState]) = match file {
                        Some(f) => (&f.hunks, &f.review.hunks),
                        None => (&[], &[]),
                    };
                    let found = search_text(
                        "",
                        &text,
                        hunks,
                        states,
                        query.matcher,
                        query.filters,
                        Sides::TargetOnly,
                    );
                    out.file(path.clone(), file, found);
                    if out.full() {
                        break;
                    }
                }
            }
        }
        Ok(out.finish())
    }
}

/// Gathers the matches of the files until the limit.
struct Collector {
    result: SearchResult,
    limit: usize,
}

impl Collector {
    fn new(revision: u32, limit: usize) -> Self {
        Self {
            result: SearchResult {
                revision,
                files: Vec::new(),
                hits: 0,
                truncated: false,
                searched: 0,
            },
            limit,
        }
    }

    fn full(&self) -> bool {
        self.result.truncated
    }

    /// Adds a searched file, as many of its matches as the limit leaves room for. The search is
    /// truncated once a match does not fit.
    fn file(&mut self, path: RepoPath, changed: Option<&FileView>, found: Vec<FoundLine>) {
        self.result.searched += 1;
        let mut lines = Vec::new();
        let mut hits = 0u32;
        for mut line in found {
            let room = self.limit.saturating_sub(self.result.hits as usize) as u32;
            if room == 0 {
                self.result.truncated = true;
                break;
            }
            let overflow = line.total > room;
            if overflow {
                line.keep(room);
            }
            hits += line.total;
            self.result.hits += line.total;
            lines.push(line);
            if overflow {
                self.result.truncated = true;
                break;
            }
        }
        if !lines.is_empty() {
            self.result.files.push(SearchFile {
                path,
                status: changed.map(|f| f.status),
                completion: changed.map(|f| f.completion),
                hunks: changed.map(|f| f.hunks.clone()).unwrap_or_default(),
                states: changed.map(|f| f.review.hunks.clone()).unwrap_or_default(),
                lines,
                hits,
            });
        }
    }

    fn finish(self) -> SearchResult {
        self.result
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use vet_core::filter::FileMask;

    use super::*;
    use crate::search::SearchSide;
    use crate::session::tests::Fixture;

    struct Substring(&'static str);

    impl LineMatcher for Substring {
        fn hits(&self, line: &str) -> Vec<Range<usize>> {
            line.match_indices(self.0).map(|(i, m)| i..i + m.len()).collect()
        }
    }

    const ALL: SearchFilters = SearchFilters {
        side: SearchSide::Both,
        only_changes: false,
        not_seen: false,
    };

    async fn run(fx: &Fixture, scope: SearchScope, mask: &str, filters: SearchFilters, limit: usize) -> SearchResult {
        let mask = FileMask::parse(mask).unwrap();
        let query = SearchQuery {
            scope,
            matcher: &Substring("delay"),
            mask: &mask,
            filters,
            limit,
        };
        fx.session.search(&query).await.unwrap()
    }

    fn paths(result: &SearchResult) -> Vec<String> {
        result.files.iter().map(|f| f.path.to_string_lossy()).collect()
    }

    async fn fixture() -> Fixture {
        Fixture::open(
            &[
                ("a.txt", "delay one\nold delay\n"),
                ("quiet.txt", "x\n"),
                ("same.txt", "unchanged delay\n"),
            ],
            &[
                ("a.txt", "delay one\nnew delay\n"),
                ("quiet.txt", "x\n"),
                ("same.txt", "unchanged delay\n"),
                ("bin", "delay\0"),
            ],
        )
        .await
    }

    #[tokio::test]
    async fn a_view_searches_the_changed_files_with_text_on_both_sides() {
        let fx = fixture().await;
        let result = run(&fx, SearchScope::View(View::Changeset), "", ALL, 100).await;
        assert_eq!(paths(&result), ["a.txt"]);
        let a = &result.files[0];
        assert_eq!(a.status, Some(FileStatus::Modified));
        assert_eq!(a.lines.len(), 3);
        assert_eq!(result.hits, 3);
    }

    #[tokio::test]
    async fn the_repository_adds_the_unchanged_files_and_leaves_the_removed_lines_out() {
        let fx = fixture().await;
        let result = run(&fx, SearchScope::Repository, "", ALL, 100).await;
        assert_eq!(paths(&result), ["a.txt", "same.txt"]);
        assert_eq!(result.files[0].lines.len(), 2);
        assert_eq!(result.files[1].status, None);
        let only = SearchFilters {
            only_changes: true,
            ..ALL
        };
        let result = run(&fx, SearchScope::Repository, "", only, 100).await;
        assert_eq!(paths(&result), ["a.txt"]);
    }

    #[tokio::test]
    async fn the_mask_narrows_the_files() {
        let fx = fixture().await;
        let result = run(&fx, SearchScope::Repository, "same.*", ALL, 100).await;
        assert_eq!(paths(&result), ["same.txt"]);
        assert_eq!(result.searched, 1);
    }

    #[tokio::test]
    async fn a_search_that_finds_exactly_the_limit_is_not_truncated_and_one_that_finds_more_is() {
        let fx = fixture().await;
        // a.txt has two matches in the target, same.txt one.
        let exact = run(&fx, SearchScope::Repository, "", ALL, 3).await;
        assert_eq!((exact.hits, exact.truncated), (3, false));
        assert_eq!(paths(&exact), ["a.txt", "same.txt"]);
        let short = run(&fx, SearchScope::Repository, "", ALL, 2).await;
        assert_eq!((short.hits, short.truncated), (2, true));
        assert_eq!(paths(&short), ["a.txt"]);
    }

    #[tokio::test]
    async fn the_limit_counts_matches_not_lines() {
        let fx = Fixture::open(&[], &[("a.txt", "delay delay delay\ndelay\n")]).await;
        let result = run(&fx, SearchScope::Repository, "", ALL, 2).await;
        assert_eq!((result.hits, result.truncated), (2, true));
        let line = &result.files[0].lines[0];
        assert_eq!((line.hits.len(), line.total), (2, 2));
        assert_eq!(result.files[0].hits, 2);
        // A line whose matches all fit is whole, and the next line does not fit.
        let whole = run(&fx, SearchScope::Repository, "", ALL, 3).await;
        assert_eq!((whole.hits, whole.truncated, whole.files[0].lines.len()), (3, true, 1));
    }

    #[tokio::test]
    async fn the_server_caps_the_limit_it_is_asked_for() {
        let many = "delay\n".repeat(MAX_HITS + 5);
        let fx = Fixture::open(&[], &[("a.txt", many.as_str())]).await;
        let result = run(&fx, SearchScope::Repository, "", ALL, usize::MAX).await;
        assert_eq!((result.hits as usize, result.truncated), (MAX_HITS, true));
        let none = run(&fx, SearchScope::Repository, "", ALL, 0).await;
        assert_eq!(none.hits, 1);
    }

    /// A matcher that takes its time, so a search is under way for a while.
    struct Slow(Arc<AtomicUsize>);

    impl LineMatcher for Slow {
        fn hits(&self, _: &str) -> Vec<Range<usize>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            Vec::new()
        }
    }

    #[tokio::test]
    async fn a_search_that_is_given_up_on_stops_reading_files() {
        let names: Vec<String> = (0..40).map(|i| format!("f{i:02}.txt")).collect();
        let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "text\n")).collect();
        let fx = Fixture::open(&[], &files).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let mask = FileMask::parse("").unwrap();
        let matcher = Slow(calls.clone());
        let query = SearchQuery {
            scope: SearchScope::Repository,
            matcher: &matcher,
            mask: &mask,
            filters: ALL,
            limit: 10,
        };
        let outcome = tokio::time::timeout(Duration::from_millis(60), fx.session.search(&query)).await;
        assert!(outcome.is_err(), "the search ended before it was given up on");
        let seen = calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), seen);
        assert!(seen < files.len());
    }
}
