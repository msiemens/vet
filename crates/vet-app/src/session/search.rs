//! Text search over the files of a view, or over the whole target tree.

use std::collections::BTreeMap;

use vet_core::completion::{Completion, HunkState};
use vet_core::content::{EntryKind, Hunk, RepoPath};
use vet_core::filter::FileMask;

use super::views::{FileStatus, FileView};
use super::{LiveSession, NoText, Result, SessionError, View};
use crate::ports::{GitRepo, LiveStore};
use crate::search::{FoundLine, LineMatcher, SearchFilters, Sides, search_text};

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
        let mut out = Collector::new(revision, query.limit);
        match query.scope {
            SearchScope::View(view) => {
                for file in &manifest.files {
                    let path = file.path.to_string_lossy();
                    if !query.mask.matches(&path) {
                        continue;
                    }
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

    /// Adds a searched file. The line that reaches the limit is the last one taken.
    fn file(&mut self, path: RepoPath, changed: Option<&FileView>, found: Vec<FoundLine>) {
        self.result.searched += 1;
        let mut lines = Vec::new();
        let mut hits = 0u32;
        for line in found {
            let n = line.hits.len() as u32;
            hits += n;
            self.result.hits += n;
            lines.push(line);
            if self.result.hits as usize >= self.limit {
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
    async fn the_limit_stops_the_search() {
        let fx = fixture().await;
        let result = run(&fx, SearchScope::Repository, "", ALL, 2).await;
        assert!(result.truncated);
        assert_eq!(paths(&result), ["a.txt"]);
        assert!(!run(&fx, SearchScope::Repository, "", ALL, 100).await.truncated);
    }
}
