//! Text search over the files of a view, or over the whole target tree.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use futures::stream::FuturesOrdered;
use vet_core::completion::{Completion, HunkState};
use vet_core::content::{
    BlobId, EntryKind, FileDiff, FileEntry, Hunk, NonTextChange, RepoPath, SnapshotDiff, decode_text,
};
use vet_core::filter::FileMask;

use super::views::{FileStatus, FileView};
use super::{LiveSession, Result, SessionError, View};
use crate::ports::{BlobJob, BlobRead, BlobSession, GitError, GitRepo, LiveStore};
use crate::search::{FoundLine, LineMatcher, SearchFilters, Sides, search_text};

/// The most matches a search lists, whatever limit it asks for.
pub const MAX_HITS: usize = 1000;

/// How many files a worker reads and matches in one go.
const CHUNK: usize = 64;
/// How many chunks are worked on at once.
const WORKERS: usize = 4;

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
    pub matcher: Arc<dyn LineMatcher>,
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
                let (st, shown) = self.viewing(view).await?;
                if st.revision != revision {
                    return Err(SessionError::Stale { current: st.revision });
                }
                let changes = shown.diffed(&st);
                let jobs = view_jobs(&manifest.files, query.mask, &changes.diff)?;
                self.look(jobs, &changes.blobs, query, &manifest.files, &mut out)
                    .await?;
            }
            SearchScope::Repository => {
                let jobs = {
                    let st = self.published.read();
                    if st.revision != revision {
                        return Err(SessionError::Stale { current: st.revision });
                    }
                    repository_jobs(&st.target.entries, &manifest.files, query)
                };
                self.look(jobs, &BTreeMap::new(), query, &manifest.files, &mut out)
                    .await?;
            }
        }
        Ok(out.finish())
    }

    /// Looks in the files of `jobs` in chunks, a few at a time, and files their matches in `out`
    /// in the order of `jobs` until the limit is reached. `known` has the texts at hand. The
    /// chunks left are not worked on once this stops, or is given up on.
    async fn look(
        &self,
        jobs: Vec<Job>,
        known: &BTreeMap<BlobId, Vec<u8>>,
        query: &SearchQuery<'_>,
        files: &[FileView],
        out: &mut Collector,
    ) -> Result<()> {
        let stop = Stop::default();
        let mut pending = jobs.into_iter();
        let mut chunks = std::iter::from_fn(|| {
            let chunk: Vec<Job> = pending.by_ref().take(CHUNK).collect();
            (!chunk.is_empty()).then_some(chunk)
        });
        let mut worked = FuturesOrdered::new();
        loop {
            while worked.len() < WORKERS
                && let Some(chunk) = chunks.next()
            {
                worked.push_back(self.work(chunk, known, query, &stop));
            }
            let Some(chunk) = worked.next().await else { break };
            for looked in chunk? {
                let Some(found) = looked.found else { continue };
                out.file(looked.path, looked.changed.map(|i| &files[i]), found);
                if out.full() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Reads and matches one chunk of files on a blocking thread.
    async fn work(
        &self,
        mut jobs: Vec<Job>,
        known: &BTreeMap<BlobId, Vec<u8>>,
        query: &SearchQuery<'_>,
        stop: &Stop,
    ) -> Result<Vec<Looked>> {
        for source in jobs.iter_mut().flat_map(Job::sources) {
            source.bytes = known.get(&source.blob).cloned();
        }
        let wanted: Vec<BlobId> = (jobs.iter_mut().flat_map(Job::sources))
            .filter(|s| s.bytes.is_none())
            .map(|s| s.blob)
            .collect();
        if !wanted.is_empty() {
            let stored = self.acquisition.store.blobs(wanted).await?;
            for source in jobs.iter_mut().flat_map(Job::sources) {
                if source.bytes.is_none() {
                    source.bytes = stored.get(&source.blob).cloned();
                }
            }
        }
        let chunk = Chunk {
            jobs,
            matcher: query.matcher.clone(),
            filters: query.filters,
            stopped: stop.flag(),
        };
        let looked = self.acquisition.git.with_blobs(chunk).await?;
        Ok(looked.into_iter().collect::<std::result::Result<_, GitError>>()?)
    }
}

/// The files a worker reads and matches in one go.
struct Chunk {
    jobs: Vec<Job>,
    matcher: Arc<dyn LineMatcher>,
    filters: SearchFilters,
    stopped: Arc<AtomicBool>,
}

impl BlobJob for Chunk {
    type Output = Vec<std::result::Result<Looked, GitError>>;

    fn run(self, git: &mut dyn BlobSession) -> Self::Output {
        let matcher = Stoppable {
            matcher: &*self.matcher,
            stopped: &self.stopped,
        };
        let mut looked = Vec::with_capacity(self.jobs.len());
        for job in self.jobs {
            if self.stopped.load(Ordering::Relaxed) {
                break;
            }
            looked.push(job.look(git, &matcher, self.filters));
        }
        looked
    }
}

/// A matcher that finds nothing once the search is over, so that a worker in the middle of a file
/// stops at its next line.
struct Stoppable<'a> {
    matcher: &'a dyn LineMatcher,
    stopped: &'a AtomicBool,
}

impl LineMatcher for Stoppable<'_> {
    fn hits(&self, line: &str) -> Vec<Range<usize>> {
        if self.stopped.load(Ordering::Relaxed) {
            return Vec::new();
        }
        self.matcher.hits(line)
    }
}

/// Where a search is over: it ended, or nobody is waiting for it any more.
#[derive(Default)]
struct Stop(Arc<AtomicBool>);

impl Stop {
    fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A text to look in, with its bytes once they are at hand.
struct Source {
    path: RepoPath,
    blob: BlobId,
    bytes: Option<Vec<u8>>,
}

impl Source {
    fn new((path, blob): (RepoPath, BlobId)) -> Self {
        Self {
            path,
            blob,
            bytes: None,
        }
    }

    /// The text, or `None` if the file has none: binary, over the cap, or gone.
    fn text(self, git: &mut dyn BlobSession) -> std::result::Result<Option<String>, GitError> {
        let bytes = match self.bytes {
            Some(bytes) => bytes,
            None => match git.read_blob_capped(&self.path, self.blob)? {
                BlobRead::Found(bytes) => bytes,
                BlobRead::TooLarge | BlobRead::Missing => return Ok(None),
            },
        };
        Ok(decode_text(&bytes))
    }
}

/// A file to look in. A side the file lacks is an empty text.
struct Job {
    path: RepoPath,
    /// The index of the file in the manifest, for a changed file.
    changed: Option<usize>,
    base: Option<Source>,
    target: Option<Source>,
    hunks: Vec<Hunk>,
    states: Vec<HunkState>,
    sides: Sides,
}

/// What a job found; `None` if the file has no text to look in.
struct Looked {
    path: RepoPath,
    changed: Option<usize>,
    found: Option<Vec<FoundLine>>,
}

impl Job {
    fn sources(&mut self) -> impl Iterator<Item = &mut Source> {
        self.base.iter_mut().chain(self.target.iter_mut())
    }

    fn look(
        self,
        git: &mut dyn BlobSession,
        matcher: &dyn LineMatcher,
        filters: SearchFilters,
    ) -> std::result::Result<Looked, GitError> {
        let Self {
            path,
            changed,
            base,
            target,
            hunks,
            states,
            sides,
        } = self;
        let mut texts = [None, None];
        for (text, source) in texts.iter_mut().zip([base, target]) {
            *text = match source {
                Some(source) => source.text(git)?,
                None => Some(String::new()),
            };
        }
        let found = match texts {
            [Some(base), Some(target)] => Some(search_text(&base, &target, &hunks, &states, matcher, filters, sides)),
            _ => None,
        };
        Ok(Looked { path, changed, found })
    }
}

/// The changed files of a view that the mask lets through, with both sides of each.
fn view_jobs(files: &[FileView], mask: &FileMask, diff: &SnapshotDiff) -> Result<Vec<Job>> {
    let mut texts: HashMap<String, &FileDiff> = HashMap::new();
    for d in &diff.files {
        texts.entry(d.path().to_string_lossy()).or_insert(d);
    }
    let mut non_texts: HashMap<String, &NonTextChange> = HashMap::new();
    for n in &diff.non_text {
        non_texts.entry(n.path.to_string_lossy()).or_insert(n);
    }
    let mut jobs = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let name = file.path.to_string_lossy();
        if !mask.matches(&name) {
            continue;
        }
        let (base, target) = if let Some(d) = texts.get(&name) {
            let side = |p: &Option<RepoPath>, b: Option<BlobId>| p.clone().zip(b);
            (side(&d.old_path, d.old_blob), side(&d.new_path, d.new_blob))
        } else if let Some(n) = non_texts.get(&name) {
            let side = |p: RepoPath, e: &Option<FileEntry>| {
                e.as_ref().filter(|e| e.kind == EntryKind::File).map(|e| (p, e.blob))
            };
            (
                side(n.from.clone().unwrap_or_else(|| n.path.clone()), &n.before),
                side(n.path.clone(), &n.after),
            )
        } else {
            return Err(SessionError::NotFound(format!("file {name}")));
        };
        jobs.push(Job {
            path: file.path.clone(),
            changed: Some(i),
            base: base.map(Source::new),
            target: target.map(Source::new),
            hunks: file.hunks.clone(),
            states: file.review.hunks.clone(),
            sides: Sides::Both,
        });
    }
    Ok(jobs)
}

/// The files of the target tree that the mask lets through, the changed ones with their hunks.
fn repository_jobs(entries: &BTreeMap<RepoPath, FileEntry>, files: &[FileView], query: &SearchQuery<'_>) -> Vec<Job> {
    let changed: BTreeMap<&RepoPath, usize> = files.iter().enumerate().map(|(i, f)| (&f.path, i)).collect();
    let skip_unchanged = query.filters.only_changes || query.filters.not_seen;
    let mut jobs = Vec::new();
    for (path, entry) in entries {
        if entry.kind != EntryKind::File || !query.mask.matches(&path.to_string_lossy()) {
            continue;
        }
        let file = changed.get(path).copied();
        if file.is_none() && skip_unchanged {
            continue;
        }
        jobs.push(Job {
            path: path.clone(),
            changed: file,
            base: None,
            target: Some(Source::new((path.clone(), entry.blob))),
            hunks: file.map(|i| files[i].hunks.clone()).unwrap_or_default(),
            states: file.map(|i| files[i].review.hunks.clone()).unwrap_or_default(),
            sides: Sides::TargetOnly,
        });
    }
    jobs
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
            matcher: Arc::new(Substring("delay")),
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

    #[tokio::test]
    async fn the_first_hits_are_those_of_the_first_files_whichever_chunks_end_first() {
        let names: Vec<String> = (0..5 * CHUNK).map(|i| format!("f{i:03}.txt")).collect();
        let text = "delay\n".repeat(4);
        let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), text.as_str())).collect();
        let fx = Fixture::open(&[], &files).await;
        let in_order = &names[..MAX_HITS / 4];
        for scope in [SearchScope::Repository, SearchScope::View(View::Changeset)] {
            let inverted = std::time::Duration::from_millis(20);
            *fx.git.stagger.lock().unwrap() = Some(inverted);
            fx.git.jobs.store(0, Ordering::SeqCst);
            let result = run(&fx, scope, "", ALL, usize::MAX).await;
            *fx.git.stagger.lock().unwrap() = None;
            assert!(
                fx.git.jobs.load(Ordering::SeqCst) >= WORKERS,
                "the chunks were not worked on together"
            );
            assert_eq!(paths(&result), in_order);
            assert_eq!((result.hits as usize, result.truncated), (MAX_HITS, true));
            assert_eq!(result.searched as usize, in_order.len() + 1);
            assert_eq!(result, run(&fx, scope, "", ALL, usize::MAX).await);
        }
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
        let query = SearchQuery {
            scope: SearchScope::Repository,
            matcher: Arc::new(Slow(calls.clone())),
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
