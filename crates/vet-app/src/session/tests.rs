use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use vet_core::classify::{changes, classify, text_blobs};
use vet_core::content::{
    BlobId, EntryKind, FileDiff, FileEntry, Hunk, LineDelta, LineRange, NonTextKind, RepoPath, Snapshot, SnapshotDiff,
    SnapshotId, SnapshotStatus,
};
use vet_core::filter::{Exclusion, ReviewFilter};
use vet_core::merkle;
use vet_core::review::{
    AnchorState, CommentTarget, Confidence, Location, Message, MessageId, PendingVote, PlatformThread, ReviewedSpan,
    Side, StaleReason, Strategy, Thread, ThreadId, ThreadStatus,
};

use vet_core::completion::HunkState;

use super::blobs::image_media_type;
use super::manifest::{old_path_of, update_of};
use super::state::{check_revision, outcome};
use super::*;
use crate::changeset::{BaseChoice, ChangesetId, CommitId, Lens, RemoteKey, Source, Version, VersionOrigin};
use crate::platform::{Discussion, FakeHosting, PullRequestHead, StoredDiscussion};
use crate::ports::{
    Attributes, BlobJob, BlobRead, BlobReader, BlobSession, BranchInfo, ChangesetRefs, ChangesetStore, Checkout,
    CommitDetail, CommitInfo, Differ, Dirty, GitError, History, Remerged, ReviewRow, ReviewStore, Scan, Scanner,
    SourceReview, Store, StoreError, TextDiff, ThreadStore,
};
use crate::pull_request::PullRequestRefs;
use crate::review::{ChangesetThread, NonTextAck, RecordedSpan, Review, ReviewId, ReviewSummary, SpanId, SpanOrigin};
use crate::trust::PullRequestTrust;

const REPO: &str = "/repo";

fn path(s: &str) -> RepoPath {
    RepoPath::from(s)
}

fn entry(text: &str, mode: u32) -> FileEntry {
    FileEntry {
        blob: BlobId::of(text.as_bytes()),
        mode,
        kind: EntryKind::File,
        unfiltered: false,
    }
}

fn snapshot(files: &[(&str, &str)]) -> Snapshot {
    let entries = files.iter().map(|(p, t)| (path(p), entry(t, 0o100644))).collect();
    Snapshot {
        id: merkle::snapshot_id(&entries),
        entries,
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// With equal line counts, one hunk per run of differing lines; otherwise one
/// hunk from the first to the last differing line.
fn hunks(old: &str, new: &str) -> Vec<Hunk> {
    let (o, n) = (lines(old), lines(new));
    if o.len() != n.len() {
        return hunk(old, new).into_iter().collect();
    }
    let mut out: Vec<Hunk> = Vec::new();
    for (i, (a, b)) in o.iter().zip(&n).enumerate() {
        let i = i as u32;
        match out.last_mut() {
            _ if a == b => {}
            Some(h) if h.old.end == i => {
                h.old.end += 1;
                h.new.end += 1;
            }
            _ => {
                let r = LineRange { start: i, end: i + 1 };
                out.push(Hunk { old: r, new: r });
            }
        }
    }
    out
}

fn hunk(old: &str, new: &str) -> Option<Hunk> {
    let (o, n) = (lines(old), lines(new));
    let prefix = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
    let suffix = o[prefix..]
        .iter()
        .rev()
        .zip(n[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let range = |len: usize| LineRange {
        start: prefix as u32,
        end: (len - suffix) as u32,
    };
    (old != new).then(|| Hunk {
        old: range(o.len()),
        new: range(n.len()),
    })
}

/// Git and worktree in one: every blob any snapshot refers to.
#[derive(Clone, Default)]
pub(crate) struct FakeGit {
    blobs: Arc<Mutex<HashMap<BlobId, Vec<u8>>>>,
    base: Arc<Mutex<Option<Snapshot>>>,
    scans: Arc<Mutex<Vec<Snapshot>>>,
    /// `<version>/<name>` of each held `refs/vet/changesets/<id>/<version>/<name>` and the commit.
    refs: Arc<Mutex<Vec<(String, CommitId)>>>,
    /// When set, deleting the changeset refs fails.
    refs_stuck: Arc<AtomicBool>,
    /// When set, `read_blob` fails; the capped read, which looks at the cap first, does not.
    reads_fail: Arc<AtomicBool>,
    /// When set, looking up a branch's tip fails as git itself failing would, not as a missing branch.
    tips_fail: Arc<AtomicBool>,
    /// When set, the tree of a commit the test did not add is not the base's but an error, as a
    /// commit the repository does not have is.
    unknown_commits_fail: Arc<AtomicBool>,
    /// What revisions (`merge-base <ref>` for merge bases) resolve to, with the commit's tree.
    commits: Arc<Mutex<HashMap<String, (CommitId, Snapshot)>>>,
    /// Largest blob `read_blob_capped` returns.
    cap: Arc<Mutex<Option<usize>>>,
    /// Renames `diff` reports, `old -> new`.
    renames: Arc<Mutex<Vec<(RepoPath, RepoPath)>>>,
    user: Arc<Mutex<Option<String>>>,
    branches: Arc<Mutex<Vec<BranchInfo>>>,
    /// Where a test moved `HEAD` to; it is the commit `head` until then.
    moved_head: Arc<Mutex<Option<CommitId>>>,
    /// First-parent history of a tip, the tip first.
    history: Arc<Mutex<HashMap<CommitId, Vec<CommitId>>>>,
    /// The revision branches are compared against by default.
    default_reference: Arc<Mutex<Option<String>>>,
    /// Unix seconds of commits; 100 for the others.
    times: Arc<Mutex<HashMap<CommitId, i64>>>,
    /// Parents of commits whose first-parent history is not the whole story (merges).
    parents: Arc<Mutex<HashMap<CommitId, Vec<CommitId>>>>,
    /// The automatic merge of the merges among the commits.
    remerges: Arc<Mutex<HashMap<CommitId, Remerged>>>,
    /// How many diffs were asked for.
    diffs: Arc<AtomicUsize>,
    /// The blobs `read_blob` was asked for.
    reads: Arc<Mutex<Vec<BlobId>>>,
    /// Held by a test, diffs and blob reads wait until it lets go.
    pub(crate) paused: Arc<tokio::sync::Mutex<()>>,
    /// How many `with_blobs` jobs were started.
    pub(crate) jobs: Arc<AtomicUsize>,
    /// When set, the first jobs of `with_blobs` take this much longer the earlier they started, so
    /// that they end in the opposite order.
    pub(crate) stagger: Arc<Mutex<Option<std::time::Duration>>>,
}

/// Reads from the blobs of a [`FakeGit`], as a repository would on a blocking thread.
struct FakeBlobs {
    blobs: Arc<Mutex<HashMap<BlobId, Vec<u8>>>>,
    cap: Option<usize>,
}

impl BlobSession for FakeBlobs {
    fn read_blob_capped(&mut self, _: &RepoPath, blob: BlobId) -> std::result::Result<BlobRead, GitError> {
        Ok(match self.blobs.lock().unwrap().get(&blob) {
            Some(bytes) if self.cap.is_some_and(|cap| bytes.len() > cap) => BlobRead::TooLarge,
            Some(bytes) => BlobRead::Found(bytes.clone()),
            None => BlobRead::Missing,
        })
    }
}

fn clock() -> i64 {
    NOW.load(Ordering::Relaxed)
}

static NOW: AtomicI64 = AtomicI64::new(0);

impl FakeGit {
    /// A local branch at `tip` whose first-parent history is `history` after it.
    fn add_branch(&self, name: &str, current: bool, tip: &str, history: &[&str]) {
        let chain: Vec<CommitId> = [tip].iter().chain(history).map(|c| CommitId((*c).into())).collect();
        self.history.lock().unwrap().insert(chain[0].clone(), chain.clone());
        let mut branches = self.branches.lock().unwrap();
        branches.retain(|b| b.name != name);
        branches.push(BranchInfo {
            name: name.into(),
            commit: chain[0].clone(),
            current,
            default: false,
            remote: false,
        });
        branches.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// The local branch `name`, not checked out, at commit `tip` with the tree `files`.
    fn branch(&self, name: &str, tip: &str, files: &[(&str, &str)], history: &[&str]) {
        self.add_commit(&format!("refs/heads/{name}"), tip, files);
        self.add_commit(tip, tip, files);
        self.add_branch(name, false, tip, history);
    }

    /// The branch `feature` at commit `tip1` (tree `files`), which left the default branch
    /// `main` at commit `m1` (tree `base`).
    pub(crate) fn feature(&self, base: &[(&str, &str)], files: &[(&str, &str)]) {
        *self.default_reference.lock().unwrap() = Some("main".into());
        self.add_commit("merge-base main", "m1", base);
        self.add_commit("main", "m2", base);
        self.add_branch("main", true, "m2", &["m1"]);
        self.branch("feature", "tip1", files, &["m1"]);
    }

    /// The branch `feature` has the commits `commits` (id and tree), oldest first, on top of
    /// `main`'s commit `m1` (tree `base`).
    pub(crate) fn feature_commits(&self, base: &[(&str, &str)], commits: &[(&str, &[(&str, &str)])]) {
        *self.default_reference.lock().unwrap() = Some("main".into());
        self.add_commit("merge-base main", "m1", base);
        self.add_commit("main", "m2", base);
        self.add_branch("main", true, "m2", &["m1"]);
        for (id, files) in commits {
            self.add_commit(id, id, files);
        }
        let (tip, files) = commits.last().expect("a commit");
        self.add_commit("refs/heads/feature", tip, files);
        let history: Vec<&str> = (commits.iter().rev().skip(1).map(|(id, _)| *id))
            .chain(["m1"])
            .collect();
        self.add_branch("feature", false, tip, &history);
    }

    /// The next scan of the working tree finds `files`.
    pub(crate) fn queue_scan(&self, files: &[(&str, &str)]) {
        let snapshot = self.snapshot(files);
        self.scans.lock().unwrap().push(snapshot);
    }

    /// `commit` is a merge of `parents`, the first being the one the branch's history follows.
    pub(crate) fn set_parents(&self, commit: &str, parents: &[&str]) {
        let parents = parents.iter().map(|p| CommitId((*p).into())).collect();
        self.parents.lock().unwrap().insert(CommitId(commit.into()), parents);
    }

    /// What the parents of the merge `commit` merge to by themselves.
    pub(crate) fn set_remerge(&self, commit: &str, files: &[(&str, &str)]) {
        let snapshot = self.snapshot(files);
        let contents = files.iter().map(|(_, text)| text.as_bytes().to_vec()).collect();
        self.remerges
            .lock()
            .unwrap()
            .insert(CommitId(commit.into()), Remerged { snapshot, contents });
    }

    pub(crate) fn add_commit(&self, spec: &str, id: &str, files: &[(&str, &str)]) {
        let snapshot = self.snapshot(files);
        self.commits
            .lock()
            .unwrap()
            .insert(spec.into(), (CommitId(id.into()), snapshot));
    }

    /// Git drops the object, as it may once no ref reaches its commit.
    pub(crate) fn forget(&self, text: &str) {
        self.blobs.lock().unwrap().remove(&BlobId::of(text.as_bytes()));
    }

    fn snapshot(&self, files: &[(&str, &str)]) -> Snapshot {
        let mut blobs = self.blobs.lock().unwrap();
        for (_, text) in files {
            blobs.insert(BlobId::of(text.as_bytes()), text.as_bytes().to_vec());
        }
        snapshot(files)
    }
}

impl Scanner for FakeGit {
    async fn scan(&self, _: Dirty) -> std::result::Result<Scan, GitError> {
        let snapshot = self.scans.lock().unwrap().remove(0);
        Ok(Scan {
            snapshot,
            status: SnapshotStatus::default(),
        })
    }
}

impl Checkout for FakeGit {
    async fn checkout<St: Store + Sync>(
        &self,
        _: &St,
        _: &CommitId,
        _: &Snapshot,
        _: &Path,
    ) -> std::result::Result<(), crate::ports::CheckoutError> {
        Ok(())
    }
}

impl BlobReader for FakeGit {
    async fn read_blob(&self, _: &RepoPath, blob: BlobId) -> std::result::Result<Option<Vec<u8>>, GitError> {
        if self.reads_fail.load(Ordering::Relaxed) {
            return Err(GitError::failed("the object cannot be read"));
        }
        self.reads.lock().unwrap().push(blob);
        drop(self.paused.lock().await);
        Ok(self.blobs.lock().unwrap().get(&blob).cloned())
    }
    async fn read_blob_capped(&self, _: &RepoPath, blob: BlobId) -> std::result::Result<BlobRead, GitError> {
        let cap = *self.cap.lock().unwrap();
        Ok(match self.blobs.lock().unwrap().get(&blob) {
            Some(bytes) if cap.is_some_and(|cap| bytes.len() > cap) => BlobRead::TooLarge,
            Some(bytes) => BlobRead::Found(bytes.clone()),
            None => BlobRead::Missing,
        })
    }
    /// The first byte of the blob id, in hex.
    async fn submodule_commit(&self, blob: BlobId) -> std::result::Result<Option<String>, GitError> {
        Ok(Some(format!("{:02x}", blob.0.0[0])))
    }
    async fn with_blobs<J: BlobJob>(&self, job: J) -> std::result::Result<J::Output, GitError> {
        let started = self.jobs.fetch_add(1, Ordering::SeqCst);
        let stagger = *self.stagger.lock().unwrap();
        if let Some(step) = stagger {
            tokio::time::sleep(step * 4u32.saturating_sub(started as u32)).await;
        }
        let mut blobs = FakeBlobs {
            blobs: self.blobs.clone(),
            cap: *self.cap.lock().unwrap(),
        };
        let (done, result) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .spawn(move || {
                let _ = done.send(job.run(&mut blobs));
            })
            .map_err(|e| GitError::failed(e.to_string()))?;
        result.await.map_err(|e| GitError::failed(e.to_string()))
    }
}

impl History for FakeGit {
    async fn head(&self) -> std::result::Result<CommitId, GitError> {
        let moved = self.moved_head.lock().unwrap().clone();
        Ok(moved.unwrap_or_else(|| CommitId("head".into())))
    }
    async fn user_name(&self) -> Option<String> {
        self.user.lock().unwrap().clone()
    }
    async fn commit_snapshot(&self, commit: &CommitId) -> std::result::Result<Snapshot, GitError> {
        let known = self.commits.lock().unwrap();
        match known.values().find(|(c, _)| c == commit) {
            Some((_, snapshot)) => Ok(snapshot.clone()),
            None if self.unknown_commits_fail.load(Ordering::Relaxed) => {
                Err(GitError::UnknownRevision(format!("no commit {}", commit.0)))
            }
            None => Ok(self.base.lock().unwrap().clone().unwrap()),
        }
    }
    async fn merge_base(&self, reference: &str, tip: &CommitId) -> std::result::Result<CommitId, GitError> {
        let commits = self.commits.lock().unwrap();
        // A merge base set for one tip wins over one for any.
        [
            format!("merge-base {reference} {}", tip.0),
            format!("merge-base {reference}"),
        ]
        .iter()
        .find_map(|key| commits.get(key))
        .map(|(c, _)| c.clone())
        .ok_or_else(|| GitError::Unrelated(format!("no common ancestor with {reference}")))
    }
    async fn default_reference(&self) -> std::result::Result<Option<String>, GitError> {
        Ok(self.default_reference.lock().unwrap().clone())
    }
    async fn default_branch(&self) -> std::result::Result<Option<String>, GitError> {
        Ok(self.default_reference.lock().unwrap().clone())
    }
    async fn first_parent_range(
        &self,
        base: &CommitId,
        tip: &CommitId,
    ) -> std::result::Result<Vec<CommitId>, GitError> {
        let history = self.history.lock().unwrap();
        let chain = history.get(tip).map(Vec::as_slice).unwrap_or_default();
        let mut range: Vec<CommitId> = chain.iter().take_while(|c| *c != base).cloned().collect();
        range.reverse();
        Ok(range)
    }
    async fn resolve_commit(&self, spec: &str) -> std::result::Result<Option<CommitId>, GitError> {
        Ok(self.commits.lock().unwrap().get(spec).map(|(c, _)| c.clone()))
    }
    async fn branch_tip(&self, name: &str) -> std::result::Result<Option<CommitId>, GitError> {
        if self.tips_fail.load(Ordering::Relaxed) {
            return Err(GitError::failed("the repository cannot be opened"));
        }
        self.resolve_commit(&format!("refs/heads/{name}")).await
    }
    async fn branches(&self) -> std::result::Result<Vec<BranchInfo>, GitError> {
        Ok(self.branches.lock().unwrap().clone())
    }
    async fn commit_info(&self, commit: &CommitId) -> std::result::Result<CommitInfo, GitError> {
        Ok(CommitInfo {
            id: commit.clone(),
            subject: format!("subject of {}", commit.0),
            author: "Ann".into(),
            time: self.times.lock().unwrap().get(commit).copied().unwrap_or(100),
        })
    }
    async fn commit_detail(&self, commit: &CommitId) -> std::result::Result<CommitDetail, GitError> {
        let parents = match self.parents.lock().unwrap().get(commit) {
            Some(parents) => parents.clone(),
            None => {
                let history = self.history.lock().unwrap();
                let next = history.values().find_map(|chain| {
                    let at = chain.iter().position(|c| c == commit)?;
                    chain.get(at + 1).cloned()
                });
                next.into_iter().collect()
            }
        };
        Ok(CommitDetail {
            info: self.commit_info(commit).await?,
            body: format!("body of {}", commit.0),
            parents,
        })
    }
    async fn remerge(&self, merge: &CommitId) -> std::result::Result<Option<Remerged>, GitError> {
        Ok(self.remerges.lock().unwrap().get(merge).cloned())
    }
    async fn commits_behind(&self, tip: &CommitId, base: &CommitId, _: usize) -> std::result::Result<usize, GitError> {
        let history = self.history.lock().unwrap();
        let chain = |c: &CommitId| history.get(c).cloned().unwrap_or_else(|| vec![c.clone()]);
        let tip_chain = chain(tip);
        Ok(match tip_chain.iter().position(|c| c == base) {
            Some(steps) => steps,
            None => {
                let base_chain = chain(base);
                tip_chain.iter().filter(|c| !base_chain.contains(c)).count()
            }
        })
    }
}

impl ChangesetRefs for FakeGit {
    async fn set_changeset_ref(
        &self,
        _: ChangesetId,
        version: u32,
        name: &str,
        commit: &CommitId,
    ) -> std::result::Result<(), GitError> {
        let name = format!("{version}/{name}");
        let mut refs = self.refs.lock().unwrap();
        refs.retain(|(n, _)| *n != name);
        refs.push((name, commit.clone()));
        Ok(())
    }
    async fn delete_changeset_refs(&self, _: ChangesetId) -> std::result::Result<(), GitError> {
        if self.refs_stuck.load(Ordering::Relaxed) {
            return Err(GitError::failed("the refs cannot be deleted"));
        }
        self.refs.lock().unwrap().clear();
        Ok(())
    }
}

impl Attributes for FakeGit {
    async fn linguist_generated<St: Store + Sync>(
        &self,
        _: &St,
        _: &Snapshot,
        _: Vec<RepoPath>,
    ) -> std::result::Result<BTreeMap<RepoPath, bool>, GitError> {
        Ok(BTreeMap::new())
    }
}

impl Differ for FakeGit {
    async fn diff<St: Store + Sync>(
        &self,
        store: &St,
        old: &Snapshot,
        new: &Snapshot,
    ) -> std::result::Result<TextDiff, GitError> {
        self.diffs.fetch_add(1, Ordering::Relaxed);
        drop(self.paused.lock().await);
        let mut blobs: BTreeMap<_, _> = self.blobs.lock().unwrap().clone().into_iter().collect();
        let wanted = text_blobs(&changes(old, new))
            .into_iter()
            .map(|(_, blob)| blob)
            .collect();
        blobs.extend(store.blobs(wanted).await.map_err(|e| GitError::failed(e.0))?);
        let mut out = SnapshotDiff::default();
        out.renames.0.extend(self.renames.lock().unwrap().iter().cloned());
        for change in changes(old, new) {
            let classified = classify(&change, &blobs);
            out.non_text.extend(classified.non_text);
            if let Some(text) = classified.text {
                let utf8 = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).unwrap();
                out.files.push(FileDiff {
                    old_path: text.old.map(|(p, _)| p.clone()),
                    new_path: text.new.map(|(p, _)| p.clone()),
                    old_blob: text.old.map(|(_, b)| b),
                    new_blob: text.new.map(|(_, b)| b),
                    hunks: hunks(&utf8(text.before), &utf8(text.after)),
                });
            }
        }
        let texts = (out.files.iter())
            .flat_map(|f| [f.old_blob, f.new_blob])
            .flatten()
            .filter_map(|b| Some((b, blobs.get(&b)?.clone())))
            .collect();
        Ok(TextDiff { diff: out, texts })
    }
}

/// The pull request rows the store keeps; the copies read longest ago go, as `SqliteStore`
/// reads its rows. `load_discussions` writes within it.
const KEPT_PULL_REQUESTS: usize = 256;

/// One repository's changesets and their reviews, each review of the changesets it was started
/// for; starting a review starts a new one and leaves the rest of the store as it was, like
/// `SqliteStore` reads its rows back. `ChangesetId(n)` is the nth changeset stored, `ReviewId(n)`
/// the nth review started, both per store.
struct MemReview {
    id: ReviewId,
    changeset: ChangesetId,
    base: BaseChoice,
    spans: Vec<(SpanId, ReviewedSpan)>,
    origins: HashMap<SpanId, SpanOrigin>,
    acks: Vec<NonTextAck>,
    last_reviewed: Option<u32>,
    summary: Option<ReviewSummary>,
    opened_at: i64,
    closed_at: Option<i64>,
    complete: Option<bool>,
}

impl MemReview {
    /// A review that has marked nothing yet.
    fn fresh(id: ReviewId, changeset: ChangesetId, base: BaseChoice, opened_at: i64) -> Self {
        Self {
            id,
            changeset,
            base,
            spans: Vec::new(),
            origins: HashMap::new(),
            acks: Vec::new(),
            last_reviewed: None,
            summary: None,
            opened_at,
            closed_at: None,
            complete: None,
        }
    }
}

#[derive(Default)]
struct Mem {
    blobs: HashMap<BlobId, Vec<u8>>,
    /// Every blob given to `put_blobs`, in order.
    put: Vec<BlobId>,
    snapshots: HashMap<SnapshotId, (Snapshot, SnapshotStatus)>,
    /// The stored changesets, in the order they were first stored.
    changesets: Vec<(String, Source)>,
    /// The changeset's versions, numbered as they were appended and read by their numbers.
    versions: HashMap<u64, Vec<Version>>,
    /// Every review ever started of any changeset, newest last.
    reviews: Vec<MemReview>,
    /// The threads of each changeset, which outlive any review of it.
    threads: HashMap<u64, Vec<ChangesetThread>>,
    lens: HashMap<u64, Lens>,
    heads: HashMap<(String, Source), PullRequestHead>,
    pins: HashMap<(String, Source), CommitId>,
    votes: HashMap<(String, Source), PendingVote>,
    trusted_heads: HashMap<(String, Source), CommitId>,
    /// What a platform last said of a pull request, by remote and number, numbered by the
    /// order it was written in, and how many rows are kept (the copies read longest ago go).
    discussions: HashMap<(RemoteKey, u64), (StoredDiscussion, u64)>,
    next_row: u64,
    filters: HashMap<String, ReviewFilter>,
    trusted: HashSet<String>,
    /// How many reviews were started; the ids run on.
    started: u64,
    next_message: u64,
    next_span: u64,
}

impl Mem {
    /// The changeset of a repository and source, stored if it was not.
    fn changeset_id(&mut self, repo: &str, source: &Source) -> u64 {
        match self
            .changesets
            .iter()
            .position(|(r, s)| r.as_str() == repo && s == source)
        {
            Some(index) => index as u64 + 1,
            None => {
                self.changesets.push((repo.to_owned(), source.clone()));
                self.changesets.len() as u64
            }
        }
    }

    fn review_of(&self, id: ReviewId) -> Option<&MemReview> {
        self.reviews.iter().find(|r| r.id == id)
    }

    fn review_of_mut(&mut self, id: ReviewId) -> Option<&mut MemReview> {
        self.reviews.iter_mut().find(|r| r.id == id)
    }

    /// The threads of `id`-th changeset; nothing for a changeset that was never stored.
    fn thread_of(&mut self, id: ThreadId) -> Option<&mut Thread> {
        let found = self.threads.values_mut().flatten().find(|t| t.thread.id == id);
        found.map(|t| &mut t.thread)
    }

    /// The versions of `id`, by their numbers.
    fn versions_of(&self, id: u64) -> Vec<Version> {
        let mut versions = self.versions.get(&id).cloned().unwrap_or_default();
        versions.sort_by_key(|v| v.n);
        versions
    }

    fn threads_of(&self, id: u64) -> Vec<ChangesetThread> {
        let mut threads = self.threads.get(&id).cloned().unwrap_or_default();
        // As the store reads them: what the platform has first, then the drafts.
        for t in threads.iter_mut() {
            if t.thread.platform.is_some() {
                t.thread.messages.sort_by_key(|m| (m.remote.is_none(), m.at, m.id));
            }
        }
        threads
    }
}

#[derive(Clone, Default)]
pub(crate) struct MemStore(Arc<Mutex<Mem>>);

type StoreResult<T> = std::result::Result<T, StoreError>;

impl Store for MemStore {
    async fn put_blobs(&self, blobs: Vec<Vec<u8>>) -> StoreResult<Vec<BlobId>> {
        let mut mem = self.0.lock().unwrap();
        Ok(blobs
            .into_iter()
            .map(|bytes| {
                let id = BlobId::of(&bytes);
                mem.blobs.insert(id, bytes);
                mem.put.push(id);
                id
            })
            .collect())
    }
    async fn missing_blobs(&self, ids: Vec<BlobId>) -> StoreResult<Vec<BlobId>> {
        let mem = self.0.lock().unwrap();
        Ok(ids.into_iter().filter(|id| !mem.blobs.contains_key(id)).collect())
    }
    async fn blob(&self, id: BlobId) -> StoreResult<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().blobs.get(&id).cloned())
    }
    async fn blobs(&self, ids: Vec<BlobId>) -> StoreResult<BTreeMap<BlobId, Vec<u8>>> {
        let mem = self.0.lock().unwrap();
        Ok(ids
            .into_iter()
            .filter_map(|id| Some((id, mem.blobs.get(&id)?.clone())))
            .collect())
    }
    async fn snapshot(&self, id: SnapshotId) -> StoreResult<Option<(Snapshot, SnapshotStatus)>> {
        Ok(self.0.lock().unwrap().snapshots.get(&id).cloned())
    }
}

impl ChangesetStore for MemStore {
    /// The nth stored changeset, `None` when this repository and source were never stored.
    async fn changeset(&self, repo: String, source: Source) -> StoreResult<Option<ChangesetId>> {
        let mem = self.0.lock().unwrap();
        Ok(Self::index(&mem, &repo, &source).map(|index| ChangesetId(index as u64 + 1)))
    }
    async fn source_reviews(&self, repo: String, sources: &[Source]) -> StoreResult<Vec<SourceReview>> {
        let mem = self.0.lock().unwrap();
        Ok(sources
            .iter()
            .map(|source| {
                let changeset = Self::index(&mem, &repo, source).map(|index| ChangesetId(index as u64 + 1));
                let open_threads = changeset.map_or(0, |id| {
                    mem.threads.get(&id.0).map_or(0, |threads| {
                        threads.iter().filter(|t| t.thread.status == ThreadStatus::Open).count()
                    }) as u32
                });
                let latest =
                    changeset.and_then(|id| mem.reviews.iter().rfind(|r| r.changeset == id).map(review_row_of));
                let latest_version =
                    changeset.map_or(0, |id| mem.versions_of(id.0).iter().map(|v| v.n).max().unwrap_or(0));
                SourceReview {
                    source: source.clone(),
                    changeset,
                    open_threads,
                    latest,
                    latest_version,
                }
            })
            .collect())
    }
    async fn lens(&self, repo: String, source: Source) -> StoreResult<Option<Lens>> {
        let mem = self.0.lock().unwrap();
        Ok(Self::index(&mem, &repo, &source).and_then(|index| mem.lens.get(&(index as u64 + 1)).copied()))
    }
    async fn pull_request_head(&self, repo: String, source: Source) -> StoreResult<Option<PullRequestHead>> {
        Ok(self.0.lock().unwrap().heads.get(&(repo, source)).cloned())
    }
    async fn pull_request_trust(&self, repo: String, source: Source) -> StoreResult<Option<PullRequestTrust>> {
        let mem = self.0.lock().unwrap();
        let head = mem.heads.get(&(repo.clone(), source.clone())).cloned();
        Ok(head.map(|head| PullRequestTrust {
            fork: head.fork,
            trusted_head: mem.trusted_heads.get(&(repo, source)).cloned(),
        }))
    }
    async fn pinned_base(&self, repo: String, source: Source) -> StoreResult<Option<CommitId>> {
        Ok(self.0.lock().unwrap().pins.get(&(repo, source)).cloned())
    }
    async fn pending_vote(&self, repo: String, source: Source) -> StoreResult<Option<PendingVote>> {
        Ok(self.0.lock().unwrap().votes.get(&(repo, source)).cloned())
    }
    async fn open_changesets(&self) -> StoreResult<Vec<ChangesetId>> {
        let mem = self.0.lock().unwrap();
        let mut open: Vec<ChangesetId> = mem
            .reviews
            .iter()
            .filter(|r| r.closed_at.is_none())
            .map(|r| r.changeset)
            .collect();
        open.sort_by_key(|id| id.0);
        open.dedup();
        Ok(open)
    }
    async fn stored_discussion(&self, remote: RemoteKey, number: u64) -> StoreResult<Option<StoredDiscussion>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .discussions
            .get(&(remote, number))
            .map(|(kept, _)| kept.clone()))
    }
    async fn versions(&self, changeset: ChangesetId) -> StoreResult<Vec<Version>> {
        Ok(self.0.lock().unwrap().versions_of(changeset.0))
    }
}

impl MemStore {
    /// The index of the changeset's row among the stored ones.
    fn index(mem: &Mem, repo: &str, source: &Source) -> Option<usize> {
        mem.changesets.iter().position(|(r, s)| r == repo && s == source)
    }
}

/// The listing row of a review, of the marks it does not need.
fn review_row_of(review: &MemReview) -> ReviewRow {
    ReviewRow {
        opened_at: review.opened_at,
        closed_at: review.closed_at,
        complete: review.complete.unwrap_or(false),
        last_reviewed: review.last_reviewed,
        summary: review.summary,
    }
}

/// The fake of the session tests. Its changesets and reviews are real-keyed (by repository and
/// source, by id), so tests that read what their session stored name the changeset their
/// review is on, not the first row the store got.
impl MemStore {
    /// The changeset of the newest review, the one every session test's session is on; none
    /// while no review has started, so what is read of it is empty.
    fn reviewed_changeset_of(mem: &Mem) -> Option<u64> {
        mem.reviews.last().map(|review| review.changeset.0)
    }

    /// The versions of the changeset of the newest review.
    pub(crate) fn reviewed_versions(&self) -> Vec<Version> {
        let mem = self.0.lock().unwrap();
        Self::reviewed_changeset_of(&mem).map_or(Vec::new(), |id| mem.versions_of(id))
    }

    /// The threads of the changeset of the newest review.
    pub(crate) fn reviewed_threads(&self) -> Vec<ChangesetThread> {
        let mem = self.0.lock().unwrap();
        Self::reviewed_changeset_of(&mem).map_or(Vec::new(), |id| mem.threads_of(id))
    }
}

/// The review of `id`, as a reader has it.
fn review_of(review: &MemReview) -> Review {
    Review {
        id: review.id,
        changeset: review.changeset,
        base: review.base.clone(),
        spans: review
            .spans
            .iter()
            .map(|(id, span)| RecordedSpan {
                id: *id,
                span: span.clone(),
                origin: review.origins.get(id).cloned().unwrap_or(SpanOrigin::Squash),
            })
            .collect(),
        acks: review.acks.clone(),
        last_reviewed: review.last_reviewed,
        opened_at: review.opened_at,
        closed_at: review.closed_at,
        complete: review.closed_at.map(|_| review.complete.unwrap_or(false)),
        summary: review.summary,
    }
}

impl ReviewStore for MemStore {
    async fn open_review(&self, changeset: ChangesetId) -> StoreResult<Option<ReviewId>> {
        let mem = self.0.lock().unwrap();
        Ok(mem
            .reviews
            .iter()
            .rfind(|r| r.changeset == changeset && r.closed_at.is_none())
            .map(|r| r.id))
    }
    async fn latest_review(&self, changeset: ChangesetId) -> StoreResult<Option<ReviewId>> {
        let mem = self.0.lock().unwrap();
        Ok(mem.reviews.iter().rfind(|r| r.changeset == changeset).map(|r| r.id))
    }
    async fn review(&self, id: ReviewId) -> StoreResult<Option<Review>> {
        let mem = self.0.lock().unwrap();
        Ok(mem.review_of(id).map(review_of))
    }
    async fn filter(&self, repo: String) -> StoreResult<ReviewFilter> {
        Ok(self.0.lock().unwrap().filters.get(&repo).cloned().unwrap_or_default())
    }
    async fn project_trusted(&self, repo: String) -> StoreResult<bool> {
        Ok(self.0.lock().unwrap().trusted.contains(&repo))
    }
}

impl ThreadStore for MemStore {
    async fn threads(&self, changeset: ChangesetId) -> StoreResult<Vec<ChangesetThread>> {
        Ok(self.0.lock().unwrap().threads_of(changeset.0))
    }
}

impl LiveStore for MemStore {
    type Writes = MemStore;

    fn writes(&self) -> MemStore {
        self.clone()
    }
}

impl ReviewWrites for MemStore {
    /// Stored once under its id; a snapshot whose id is not what its entries build is refused,
    /// as `SqliteStore` refuses it.
    async fn put_snapshot(&self, snapshot: Snapshot, status: SnapshotStatus, _: i64) -> StoreResult<()> {
        if merkle::build(&snapshot.entries).0 != snapshot.id {
            return Err(StoreError("snapshot id does not match its entries".into()));
        }
        self.0.lock().unwrap().snapshots.insert(snapshot.id, (snapshot, status));
        Ok(())
    }
    /// The lens of the changeset, stored first if it was not.
    async fn set_lens(&self, repo: String, source: Source, lens: Lens) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let id = mem.changeset_id(&repo, &source);
        mem.lens.insert(id, lens);
        Ok(())
    }
    async fn set_pull_request_head(&self, repo: String, source: Source, head: PullRequestHead) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        mem.changeset_id(&repo, &source);
        mem.heads.insert((repo, source), head);
        Ok(())
    }
    /// A copy older than what is kept replaces nothing newer, and over the kept bound the
    /// copies read longest ago go, like `SqliteStore` reads its rows (the reads are the times
    /// it was synced).
    async fn set_discussion(
        &self,
        remote: RemoteKey,
        number: u64,
        _: String,
        updated_at: i64,
        synced_at: i64,
        discussion: &Discussion,
    ) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        if let Some((kept, _)) = mem.discussions.get(&(remote.clone(), number))
            && kept.updated_at > updated_at
        {
            // What is kept is newer: what the platform says replaces nothing newer.
            return Ok(());
        }
        mem.next_row += 1;
        let row = mem.next_row;
        mem.discussions.insert(
            (remote, number),
            (
                StoredDiscussion {
                    updated_at,
                    synced_at,
                    discussion: discussion.clone(),
                },
                row,
            ),
        );
        if mem.discussions.len() > KEPT_PULL_REQUESTS {
            // The copies with the greatest (read time, row) stay; the rest go.
            let read_last: Vec<((RemoteKey, u64), (StoredDiscussion, u64))> =
                mem.discussions.clone().into_iter().collect();
            let mut read_last = read_last;
            read_last.sort_by_key(|(_, (kept, row))| (kept.synced_at, *row));
            let doomed: Vec<(RemoteKey, u64)> = read_last
                .iter()
                .take(read_last.len() - KEPT_PULL_REQUESTS)
                .map(|(key, _)| key.clone())
                .collect();
            for key in doomed {
                mem.discussions.remove(&key);
            }
        }
        Ok(())
    }
    async fn set_pinned_base(&self, repo: String, source: Source, base: Option<CommitId>) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        mem.changeset_id(&repo, &source);
        match base {
            Some(base) => mem.pins.insert((repo, source), base),
            None => mem.pins.remove(&(repo, source)),
        };
        Ok(())
    }
    async fn set_pending_vote(&self, repo: String, source: Source, vote: Option<PendingVote>) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        mem.changeset_id(&repo, &source);
        match vote {
            Some(vote) => mem.votes.insert((repo, source), vote),
            None => mem.votes.remove(&(repo, source)),
        };
        Ok(())
    }
    async fn ensure_changeset(&self, repo: String, source: Source) -> StoreResult<ChangesetId> {
        let mut mem = self.0.lock().unwrap();
        Ok(ChangesetId(mem.changeset_id(&repo, &source)))
    }
    async fn append_version(&self, changeset: ChangesetId, version: Version) -> StoreResult<()> {
        self.0
            .lock()
            .unwrap()
            .versions
            .entry(changeset.0)
            .or_default()
            .push(version);
        Ok(())
    }
    async fn start_review(
        &self,
        repo: String,
        source: Source,
        choice: BaseChoice,
        versions: Vec<Version>,
        now: i64,
    ) -> StoreResult<(ChangesetId, ReviewId)> {
        let mut mem = self.0.lock().unwrap();
        let changeset = mem.changeset_id(&repo, &source);
        // A new review starts over instead of keeping the last versions.
        mem.versions.insert(changeset, versions);
        mem.started += 1;
        let id = ReviewId(mem.started);
        mem.reviews
            .push(MemReview::fresh(id, ChangesetId(changeset), choice, now));
        Ok((ChangesetId(changeset), id))
    }
    async fn close_review(&self, id: ReviewId, now: i64, complete: bool) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let Some(review) = mem.review_of_mut(id) else {
            return Err(StoreError(format!("review {} is not stored", id.0)));
        };
        review.closed_at = Some(now);
        review.complete = Some(complete);
        Ok(())
    }
    async fn set_filter(&self, repo: String, filter_to_hold: ReviewFilter) -> StoreResult<()> {
        self.0.lock().unwrap().filters.insert(repo, filter_to_hold);
        Ok(())
    }
    async fn set_project_trusted(&self, repo: String, trusted: bool) -> StoreResult<()> {
        if trusted {
            self.0.lock().unwrap().trusted.insert(repo);
        } else {
            self.0.lock().unwrap().trusted.remove(&repo);
        }
        Ok(())
    }
    async fn set_pull_request_trusted_head(
        &self,
        repo: String,
        source: Source,
        head: Option<CommitId>,
    ) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        mem.changeset_id(&repo, &source);
        match head {
            Some(head) => mem.trusted_heads.insert((repo, source), head),
            None => mem.trusted_heads.remove(&(repo, source)),
        };
        Ok(())
    }
    async fn set_last_reviewed(&self, id: ReviewId, version: u32) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let Some(review) = mem.review_of_mut(id) else {
            return Err(StoreError(format!("review {} is not stored", id.0)));
        };
        review.last_reviewed = Some(version);
        Ok(())
    }
    async fn set_review_summary(&self, id: ReviewId, summary: ReviewSummary) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let Some(review) = mem.review_of_mut(id) else {
            return Err(StoreError(format!("review {} is not stored", id.0)));
        };
        review.summary = Some(summary);
        Ok(())
    }
    async fn edit_spans(
        &self,
        id: ReviewId,
        remove: Vec<SpanId>,
        add: Vec<(ReviewedSpan, SpanOrigin)>,
    ) -> StoreResult<Vec<SpanId>> {
        let mut mem = self.0.lock().unwrap();
        let added = add
            .into_iter()
            .map(|(span, origin)| {
                mem.next_span += 1;
                (SpanId(mem.next_span), span, origin.clone())
            })
            .collect::<Vec<_>>();
        let Some(review) = mem.review_of_mut(id) else {
            return Err(StoreError(format!("review {} is not stored", id.0)));
        };
        let ids = added.iter().map(|(id, _, _)| *id).collect();
        review.spans.retain(|(held, _)| !remove.contains(held));
        for (id, span, origin) in added {
            review.spans.push((id, span));
            review.origins.insert(id, origin);
        }
        Ok(ids)
    }
    async fn acknowledge(&self, id: ReviewId, ack: NonTextAck) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let Some(review) = mem.review_of_mut(id) else {
            return Err(StoreError(format!("review {} is not stored", id.0)));
        };
        if !review.acks.contains(&ack) {
            review.acks.push(ack);
        }
        Ok(())
    }
    async fn unacknowledge(&self, id: ReviewId, ack: NonTextAck) -> StoreResult<()> {
        if let Some(review) = self.0.lock().unwrap().review_of_mut(id) {
            review.acks.retain(|held| *held != ack);
        }
        Ok(())
    }
    async fn add_thread(
        &self,
        changeset: ChangesetId,
        review: Option<ReviewId>,
        target: CommentTarget,
        opener: Message,
        platform: Option<PlatformThread>,
        status: ThreadStatus,
    ) -> StoreResult<(ThreadId, MessageId)> {
        let mut mem = self.0.lock().unwrap();
        if changeset.0 as usize > mem.changesets.len() || changeset.0 == 0 {
            return Err(StoreError(format!("changeset {} is not stored", changeset.0)));
        }
        if let Some(review) = review
            && mem.review_of(review).is_none()
        {
            return Err(StoreError(format!("review {} is not stored", review.0)));
        }
        let id = ThreadId(mem.threads.values().flatten().map(|t| t.thread.id.0).max().unwrap_or(0) + 1);
        mem.next_message += 1;
        let message = MessageId(mem.next_message);
        mem.threads.entry(changeset.0).or_default().push(ChangesetThread {
            thread: Thread {
                id,
                target,
                messages: vec![Message { id: message, ..opener }],
                status,
                platform,
            },
            review,
        });
        Ok((id, message))
    }
    async fn add_message(&self, thread: ThreadId, message: Message) -> StoreResult<MessageId> {
        let mut mem = self.0.lock().unwrap();
        mem.next_message += 1;
        let id = MessageId(mem.next_message);
        let Some(held) = mem.thread_of(thread) else {
            return Err(StoreError(format!("thread {} is not stored", thread.0)));
        };
        held.messages.push(Message { id, ..message });
        Ok(id)
    }
    async fn replace_message(&self, message: Message) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let held = mem
            .threads
            .values_mut()
            .flatten()
            .flat_map(|t| t.thread.messages.iter_mut())
            .find(|m| m.id == message.id);
        let Some(held) = held else {
            return Err(StoreError(format!("message {} is not stored", message.id.0)));
        };
        *held = message;
        Ok(())
    }
    async fn delete_message(&self, message: MessageId) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        for t in mem.threads.values_mut().flatten() {
            t.thread.messages.retain(|m| m.id != message);
        }
        Ok(())
    }
    async fn set_thread_status(&self, thread: ThreadId, status: ThreadStatus) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let Some(held) = mem.thread_of(thread) else {
            return Err(StoreError(format!("thread {} is not stored", thread.0)));
        };
        held.status = status;
        Ok(())
    }
    async fn set_thread_platform(&self, thread: ThreadId, platform: PlatformThread) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let Some(held) = mem.thread_of(thread) else {
            return Err(StoreError(format!("thread {} is not stored", thread.0)));
        };
        held.platform = Some(platform);
        Ok(())
    }
    async fn set_thread_target(&self, thread: ThreadId, target: CommentTarget) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let Some(held) = mem.thread_of(thread) else {
            return Err(StoreError(format!("thread {} is not stored", thread.0)));
        };
        held.target = target;
        Ok(())
    }
    async fn delete_thread(&self, thread: ThreadId) -> StoreResult<()> {
        let mut mem = self.0.lock().unwrap();
        let before: usize = mem.threads.values().map(Vec::len).sum();
        for threads in mem.threads.values_mut() {
            threads.retain(|t| t.thread.id != thread);
        }
        if mem.threads.values().map(Vec::len).sum::<usize>() == before {
            return Err(StoreError(format!("thread {} is not stored", thread.0)));
        }
        Ok(())
    }
}

/// Marks that state what they set rather than flip it, which tests need to set up several hunks
/// at once or to un-mark one that was never marked.
impl LiveSession<FakeGit, MemStore> {
    pub(crate) async fn set_seen(
        &self,
        revision: u32,
        path: &str,
        hunks: &[u32],
        seen: bool,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let (mut st, _) = self.beginning(View::Changeset).await?;
        check_revision(&st, revision)?;
        let started = seen && self.begin(&mut st).await?;
        let writes = st.writes();
        self.mark_hunks(
            &writes,
            &mut st,
            View::Changeset,
            path,
            hunks,
            super::marks::Mark::Seen(seen),
        )
        .await?;
        Ok(outcome(&st, update_of(&st, path)?, started))
    }

    pub(crate) async fn acknowledge(
        &self,
        revision: u32,
        path: &str,
        kind: NonTextKind,
        acknowledged: bool,
    ) -> Result<Outcome<ReviewUpdateView>> {
        let (mut st, _) = self.beginning(View::Changeset).await?;
        check_revision(&st, revision)?;
        let started = acknowledged && self.begin(&mut st).await?;
        let writes = st.writes();
        self.mark_item(&writes, &mut st, View::Changeset, path, kind, acknowledged)
            .await?;
        Ok(outcome(&st, update_of(&st, path)?, started))
    }
}

fn numbered(n: usize, edited: &[usize]) -> String {
    (0..n)
        .map(|i| {
            if edited.contains(&i) {
                format!("edited {i}\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect()
}

pub(crate) struct Fixture {
    pub(crate) git: FakeGit,
    pub(crate) store: MemStore,
    /// The session, shared: the retainer of its content is wired into the `Semantics`
    /// of the tests that ask it.
    pub(crate) session: Arc<LiveSession<FakeGit, MemStore>>,
    /// What the semantic layer reads of `session`.
    pub(crate) view: RevisionView<FakeGit, MemStore>,
}

impl Fixture {
    pub(crate) async fn open(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Self {
        let git = FakeGit::default();
        *git.base.lock().unwrap() = Some(git.snapshot(base));
        let first = git.snapshot(dirty);
        git.scans.lock().unwrap().push(first);
        let store = MemStore::default();
        let session = Arc::new(
            LiveSession::open(git.clone(), store.clone(), REPO.into(), clock)
                .await
                .unwrap(),
        );
        let view = session.view();
        Self {
            git,
            store,
            session,
            view,
        }
    }

    /// Like `open`, with the review started.
    pub(crate) async fn reviewing(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Self {
        let fx = Self::open(base, dirty).await;
        fx.begin().await;
        fx
    }

    pub(crate) async fn begin(&self) {
        let (mut st, _) = self.session.beginning(View::Changeset).await.unwrap();
        self.session.begin(&mut st).await.unwrap();
    }

    /// The tree settles at `files`.
    pub(crate) async fn settle(&self, files: &[(&str, &str)]) -> u32 {
        let snapshot = self.git.snapshot(files);
        let scan = Scan {
            snapshot,
            status: SnapshotStatus::default(),
        };
        self.session.settled(scan).await.unwrap().0
    }

    async fn hunks(&self, file: &str) -> Vec<HunkState> {
        let manifest = self.session.manifest(View::Changeset).await.unwrap();
        let f = manifest.files.iter().find(|f| f.path == path(file)).unwrap();
        f.review.hunks.clone()
    }
}

#[tokio::test]
async fn browsing_keeps_nothing_and_settling_does_not_start_a_review() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);

    fx.settle(&[("a.txt", "three\n")]).await;
    let m = fx.session.manifest(View::Changeset).await.unwrap();

    assert_eq!((m.revision, m.lifecycle), (1, Lifecycle::Browsing));
    assert_eq!(m.files.len(), 1);
    let mem = fx.store.0.lock().unwrap();
    assert_eq!(
        mem.changesets,
        [(REPO.to_owned(), Source::WorkingTree)],
        "browsing stores the working tree's changeset, for the base it was pinned at"
    );
    assert!(mem.snapshots.is_empty() && mem.versions.is_empty() && mem.reviews.is_empty());
    assert!(mem.blobs.is_empty());
    assert!(fx.git.refs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_first_mark_starts_the_review_at_version_1_and_retains_dirty_content() {
    let a = numbered(10, &[]);
    let b = numbered(10, &[3]);
    let fx = Fixture::open(&[("a.txt", &a)], &[("a.txt", &b), ("new.txt", "n\n")]).await;

    // Un-marking is not a review action.
    let unmarked = fx.session.set_seen(0, "a.txt", &[0], false).await.unwrap();
    assert_eq!(
        (fx.session.lifecycle().await, unmarked.started),
        (Lifecycle::Browsing, None)
    );

    let marked = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert!(matches!(marked.started, Some(Lifecycle::Open { version: 1, .. })));
    let again = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(
        again.started, None,
        "only the request that starts the review reports it"
    );

    assert!(matches!(
        fx.session.manifest(View::Changeset).await.unwrap().lifecycle,
        Lifecycle::Open { version: 1, .. }
    ));
    let started = fx.session.target_snapshot().await.id;
    let mem = fx.store.0.lock().unwrap();
    assert!(mem.blobs.contains_key(&BlobId::of(b.as_bytes())));
    assert!(mem.blobs.contains_key(&BlobId::of(b"n\n")));
    assert!(
        !mem.blobs.contains_key(&BlobId::of(a.as_bytes())),
        "HEAD content stays in git"
    );
    assert_eq!(mem.snapshots.len(), 2);
    assert_eq!(mem.reviews.last().expect("the started review").spans.len(), 1);
    assert_eq!(fx.git.refs.lock().unwrap().len(), 2);
    let stored_versions = mem.versions_of(1);
    let [first] = stored_versions.as_slice() else {
        panic!("{:?}", mem.versions)
    };
    assert_eq!(
        (
            first.n,
            &first.base_commit,
            &first.tip,
            first.snapshots.map(|s| s.target),
            first.origin
        ),
        (
            1,
            &CommitId("head".into()),
            &CommitId("head".into()),
            Some(started),
            VersionOrigin::Settled
        )
    );
    assert_eq!(mem.reviews.last().expect("the started review").base, detached_head());
}

#[tokio::test]
async fn acknowledging_and_a_new_thread_also_start_the_review() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", "echo\n")]);
    let mut dirty = base.clone();
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();
    session
        .acknowledge(0, "run.sh", NonTextKind::Mode, false)
        .await
        .unwrap();
    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);

    session.acknowledge(0, "run.sh", NonTextKind::Mode, true).await.unwrap();
    assert!(matches!(session.lifecycle().await, Lifecycle::Open { version: 1, .. }));

    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session
        .create_thread(0, View::Changeset, NewThread::Changeset, "why".into(), 1)
        .await
        .unwrap();
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 1, .. }
    ));
}

#[tokio::test]
async fn each_settled_snapshot_of_an_open_review_is_the_next_version() {
    // The only test that moves the clock.
    NOW.store(100, Ordering::Relaxed);
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert_eq!(
        fx.session.lifecycle().await,
        Lifecycle::Open {
            version: 1,
            updated_at: 100
        }
    );
    NOW.store(900, Ordering::Relaxed);

    assert_eq!(fx.settle(&[("a.txt", "three\n")]).await, 1);
    assert_eq!(fx.settle(&[("a.txt", "four\n")]).await, 2);

    assert_eq!(fx.session.revision().await, 2);
    assert!(fx.store.0.lock().unwrap().blobs.contains_key(&BlobId::of(b"three\n")));
    assert_eq!(
        fx.session.lifecycle().await,
        Lifecycle::Open {
            version: 3,
            updated_at: 900
        }
    );
    let target = fx.session.target_snapshot().await.id;
    let versions = fx.store.reviewed_versions();
    assert_eq!(
        versions.iter().map(|v| (v.n, v.origin, v.at)).collect::<Vec<_>>(),
        [
            (1, VersionOrigin::Settled, 100),
            (2, VersionOrigin::Settled, 900),
            (3, VersionOrigin::Settled, 900)
        ]
    );
    assert_eq!(
        versions[2].snapshots.map(|s| s.target),
        Some(target),
        "the version is the snapshot that settled"
    );
    let base_of = |v: &Version| (v.snapshots.map(|s| s.base), v.base_commit.clone(), v.tip.clone());
    assert!(
        versions.iter().all(|v| base_of(v) == base_of(&versions[0])),
        "versions of one review share its base and the commit it started at"
    );
    // A filter change is not a version.
    fx.session.set_filter(ReviewFilter::default()).await.unwrap();
    assert_eq!(fx.store.reviewed_versions().len(), 3);
}

/// The files `version` changed with their added and removed lines.
fn changed(version: &Version) -> Vec<(String, u32, u32)> {
    let files = version.change.files.iter();
    files
        .map(|f| (f.path.to_string_lossy(), f.delta.added, f.delta.removed))
        .collect()
}

#[tokio::test]
async fn a_version_records_what_it_changed_against_the_one_before_it() {
    let (a0, a1, a2) = (numbered(10, &[]), numbered(10, &[3]), numbered(10, &[3, 7]));
    let fx = Fixture::reviewing(
        &[("a.txt", &a0), ("keep.txt", "k\n")],
        &[("a.txt", &a1), ("new.txt", "n\n"), ("keep.txt", "k\n")],
    )
    .await;

    fx.settle(&[("a.txt", &a2), ("new.txt", "n\nm\n"), ("keep.txt", "k\n")])
        .await;
    fx.settle(&[("a.txt", &a2), ("keep.txt", "k\n")]).await;

    let versions = fx.store.reviewed_versions();
    assert_eq!(
        changed(&versions[0]),
        [("a.txt".to_string(), 1, 1), ("new.txt".to_string(), 1, 0)],
        "the first version changed what its changeset changes"
    );
    assert_eq!(
        changed(&versions[1]),
        [("a.txt".to_string(), 1, 1), ("new.txt".to_string(), 1, 0)],
        "a later one only what the burst before it wrote"
    );
    assert_eq!(
        changed(&versions[2]),
        [("new.txt".to_string(), 0, 2)],
        "untouched files are not listed, and a deleted one counts its lines"
    );
    assert_eq!(
        versions[2].change.delta(),
        vet_core::content::LineDelta { added: 0, removed: 2 }
    );
}

#[tokio::test]
async fn a_version_that_only_moves_the_base_changed_nothing_the_author_did() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    let pushed = [("a.txt", "two\n"), ("upstream.txt", "u\n")];
    fetch_pull_request_7(&fx.git, &[("a.txt", "one\n")], "pr1", &pushed);
    fx.session.open_changeset(pull_request_7()).await.unwrap();
    fx.begin().await;

    // The author rebased onto a target branch that has what the tree took over from upstream.
    fetch_pull_request_7(&fx.git, &[("a.txt", "one\n")], "pr2", &pushed);
    move_base_of_pull_request_7(&fx.git, "c1-full", &[("a.txt", "one\n"), ("upstream.txt", "u\n")]);
    fx.session.observe().await.unwrap().expect("head and base moved");

    let versions = fx.store.reviewed_versions();
    assert_eq!(versions[1].origin, VersionOrigin::Observed);
    assert_eq!(
        changed(&versions[0]),
        [("a.txt".to_string(), 1, 1), ("upstream.txt".to_string(), 1, 0)]
    );
    assert!(changed(&versions[1]).is_empty(), "{:?}", versions[1].change);
}

#[tokio::test]
async fn what_changed_since_the_reviewed_version_leaves_out_a_change_taken_back() {
    let fx = Fixture::open(
        &[
            ("a.txt", "a0\n"),
            ("b.txt", "b0\n"),
            ("c.txt", "c0\n"),
            ("d.txt", "d0\n"),
        ],
        &[
            ("a.txt", "a1\n"),
            ("b.txt", "b0\n"),
            ("c.txt", "c0\n"),
            ("d.txt", "d0\n"),
        ],
    )
    .await;
    fx.begin().await;
    let revision = fx.session.revision().await;
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();

    // Version 2 changes b.txt and d.txt; version 3 takes b.txt back and changes c.txt, so d.txt
    // and c.txt differ from version 1 while b.txt, changed and changed back, does not.
    fx.settle(&[
        ("a.txt", "a1\n"),
        ("b.txt", "b1\n"),
        ("c.txt", "c0\n"),
        ("d.txt", "d1\n"),
    ])
    .await;
    fx.settle(&[
        ("a.txt", "a1\n"),
        ("b.txt", "b0\n"),
        ("c.txt", "c1\n"),
        ("d.txt", "d1\n"),
    ])
    .await;

    let since = fx.session.changed_since(1).await.unwrap();
    let files: Vec<String> = since.files.iter().map(|p| p.to_string_lossy()).collect();
    assert_eq!(
        files,
        ["c.txt", "d.txt"],
        "a change taken back and an unchanged file count for nothing"
    );
    assert_eq!((since.from, since.to), (1, 3));
}

#[tokio::test]
async fn the_working_tree_keeps_the_head_it_was_opened_at_as_its_base() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.add_branch("feature", true, "head", &[]);

    // The agent commits: HEAD moves on, the base stays and nothing is amiss.
    fx.git.add_branch("feature", true, "f1", &["head"]);
    *fx.git.moved_head.lock().unwrap() = Some(CommitId("f1".into()));
    fx.settle(&[("a.txt", "two\n")]).await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.base.commit.id, CommitId("head".into()));
    assert_eq!(m.target.branch.as_deref(), Some("feature"));
    assert!(!m.base.head_off_base);
}

/// The working tree's `HEAD` moves to `commit`, which builds on the commit `ancestry` ends in.
fn move_head(git: &FakeGit, commit: &str, ancestry: &[&str]) {
    git.add_branch("feature", true, commit, ancestry);
    *git.moved_head.lock().unwrap() = Some(CommitId(commit.into()));
}

#[tokio::test]
async fn a_head_that_no_longer_builds_on_the_base_is_told_until_it_does_again() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let off_base = || async {
        fx.session
            .manifest(View::Changeset)
            .await
            .unwrap()
            .changeset_base
            .head_off_base
    };
    assert!(!off_base().await);

    // A rebase: HEAD is on other history, and the tree changed with it.
    move_head(&fx.git, "r2", &["r1"]);
    assert!(
        fx.session.observe().await.unwrap().is_some(),
        "a revision that tells it, with no snapshot settling"
    );
    assert!(off_base().await);
    assert_eq!(fx.session.observe().await.unwrap(), None, "told once");

    // Back on a commit that builds on the base.
    move_head(&fx.git, "h2", &["head"]);
    assert!(fx.session.observe().await.unwrap().is_some());
    assert!(!off_base().await);
}

#[tokio::test]
async fn a_head_that_left_the_base_is_told_with_the_snapshot_that_settles_after_it() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    move_head(&fx.git, "r2", &["r1"]);

    fx.settle(&[("a.txt", "three\n")]).await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert!(m.changeset_base.head_off_base);
}

#[tokio::test]
async fn a_head_that_left_the_base_while_vet_was_closed_is_told_on_resuming() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    move_head(&fx.git, "r2", &["r1"]);

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    let m = session.manifest(View::Changeset).await.unwrap();
    assert!(m.changeset_base.head_off_base);
    assert_eq!(m.base.commit.id, CommitId("head".into()), "the base stays");
}

#[tokio::test]
async fn a_version_made_after_the_agent_committed_is_at_the_new_head_and_keeps_it_reachable() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    move_head(&fx.git, "h2", &["head"]);

    let n = fx.settle(&[("a.txt", "three\n")]).await;

    let versions = fx.store.reviewed_versions();
    assert_eq!(versions.last().unwrap().tip, CommitId("h2".into()));
    let refs = fx.git.refs.lock().unwrap().clone();
    assert!(
        refs.contains(&(format!("{}/tip", versions.last().unwrap().n), CommitId("h2".into()))),
        "{refs:?} after revision {n}"
    );
}

#[tokio::test]
async fn a_workspace_reopened_before_its_first_mark_keeps_the_head_it_first_opened_at() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    // The agent commits while Vet is closed; nothing was marked yet.
    move_head(&fx.git, "h2", &["head"]);

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    let m = session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.base.commit.id, CommitId("head".into()));
}

#[tokio::test]
async fn a_workspace_opened_after_a_review_was_finished_starts_from_the_head_of_then() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session.finish_review().await.unwrap();
    move_head(&fx.git, "h2", &["head"]);
    let session = fx.reopen(&[("a.txt", "two\n")]).await;
    assert_eq!(
        session.manifest(View::Changeset).await.unwrap().base.commit.id,
        CommitId("h2".into())
    );
}

#[tokio::test]
async fn the_base_of_a_finished_review_stays_until_the_workspace_is_opened_again() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session.finish_review().await.unwrap();
    move_head(&fx.git, "h2", &["head"]);

    let m = fx.session.manifest(View::Changeset).await.unwrap();

    assert_eq!(m.base.commit.id, CommitId("head".into()));
}

#[tokio::test]
async fn only_the_working_tree_is_told_about_its_head() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[1]))]);
    *fx.git.moved_head.lock().unwrap() = Some(CommitId("r2".into()));
    view_feature(&fx).await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();

    assert!(!m.changeset_base.head_off_base);
}

#[tokio::test]
async fn finishing_returns_to_browsing_and_a_later_action_starts_a_new_review() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert!(matches!(
        fx.session.finish_review().await,
        Err(SessionError::Invalid(_))
    ));
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    fx.settle(&[("a.txt", "three\n")]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Unseen]);

    // Unseen changes may be left: the review is recorded as not complete.
    assert!(!fx.session.finish_review().await.unwrap());

    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
    assert!(fx.git.refs.lock().unwrap().is_empty());
    {
        let mem = fx.store.0.lock().unwrap();
        let review = mem.reviews.last().expect("the finished review");
        assert_eq!((review.closed_at.is_some(), review.complete), (true, Some(false)));
    }
    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!((m.revision, m.lifecycle), (2, Lifecycle::Browsing));
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    assert_eq!(m.completion.seen_lines, 0);

    // Editing is browsing again; the next review action starts review number two.
    fx.settle(&[("a.txt", "four\n")]).await;
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
    fx.session.set_seen(3, "a.txt", &[0], true).await.unwrap();
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 1, .. }
    ));
    let mem = fx.store.0.lock().unwrap();
    let review = mem.reviews.last().expect("the restarted review");
    assert_eq!(
        (
            review.closed_at.is_none(),
            mem.versions_of(MemStore::reviewed_changeset_of(&mem).expect("the restarted review"))
                .len(),
            review.spans.len()
        ),
        (true, 1, 1)
    );
}

#[tokio::test]
async fn a_review_closed_in_the_store_is_closed_in_the_session_even_if_its_refs_stay() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.refs_stuck.store(true, Ordering::Relaxed);

    fx.session.finish_review().await.unwrap();

    assert!(
        fx.store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the finished review")
            .closed_at
            .is_some()
    );
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
    assert_eq!(fx.session.revision().await, 1);
}

#[tokio::test]
async fn a_complete_review_is_recorded_as_complete_when_finished() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    assert!(fx.session.finish_review().await.unwrap());
    assert_eq!(
        fx.store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the finished review")
            .complete,
        Some(true)
    );
}

#[tokio::test]
async fn a_manifest_says_why_it_shows_nothing() {
    // The working tree, nothing changed and no review: watching.
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    assert_eq!(
        fx.session.manifest(View::Changeset).await.unwrap().empty,
        EmptyReason::Watching
    );
    // A review whose edits were taken back: undone.
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.settle(&[("a.txt", "one\n")]).await;
    assert_eq!(
        fx.session.manifest(View::Changeset).await.unwrap().empty,
        EmptyReason::Undone
    );
}

#[tokio::test]
async fn manifest_lists_changes_with_unseen_hunks() {
    let a = numbered(10, &[]);
    let fx = Fixture::open(
        &[("a.txt", &a), ("gone.txt", "x\ny\n")],
        &[("a.txt", &numbered(10, &[3])), ("new.txt", "n\n")],
    )
    .await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();

    assert_eq!(m.revision, 0);
    let paths: Vec<_> = m.files.iter().map(|f| f.path.to_string_lossy()).collect();
    assert_eq!(paths, ["a.txt", "gone.txt", "new.txt"]);
    let status: Vec<_> = m.files.iter().map(|f| f.status).collect();
    assert_eq!(status, [FileStatus::Modified, FileStatus::Deleted, FileStatus::Added]);
    // Ten lines and a final line break: the editor shows an eleventh, empty one.
    assert_eq!((m.files[0].base_lines, m.files[0].target_lines), (11, 11));
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    // a.txt: 1 + 1 lines, gone.txt: 2, new.txt: 1
    assert_eq!((m.completion.lines, m.completion.seen_lines), (5, 0));
    assert!(!m.completion.is_complete());
}

#[tokio::test]
async fn marking_survives_edits_elsewhere_and_a_revert() {
    let base = numbered(30, &[]);
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;

    let update = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    assert_eq!((update.completion.lines, update.completion.seen_lines), (2, 2));
    assert!(update.changeset.is_complete());

    // An edit far away adds an unseen hunk and leaves the first one seen.
    let more = numbered(30, &[3, 25]);
    assert_eq!(fx.settle(&[("a.txt", &more)]).await, 1);
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Unseen]);

    // Reverting it restores the settled state of the first mark.
    fx.settle(&[("a.txt", &dirty)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);

    // Editing the marked line itself uncovers it; reverting covers it again.
    let edited = numbered(30, &[3]).replace("edited 3", "edited three");
    fx.settle(&[("a.txt", &edited)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Unseen]);
    fx.settle(&[("a.txt", &dirty)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
}

#[tokio::test]
async fn a_seen_region_with_an_edit_in_the_middle_is_split() {
    let base = numbered(30, &[]);
    let region: Vec<usize> = (5..25).collect();
    let dirty = numbered(30, &region);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    let middle = dirty
        .replace("edited 14", "mid a")
        .replace("edited 15", "mid b")
        .replace("edited 16", "mid c");
    fx.settle(&[("a.txt", &middle)]).await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    // 20 base lines are still seen; 17 of 20 target lines are.
    assert_eq!((m.completion.lines, m.completion.seen_lines), (40, 37));
}

#[tokio::test]
async fn unmarking_removes_the_hunks_spans() {
    let base = numbered(30, &[]);
    let dirty = numbered(30, &[3, 20]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0, 1], true).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Seen]);

    let update = fx.session.set_seen(0, "a.txt", &[0], false).await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Unseen, HunkState::Seen]);
    assert_eq!(update.completion.seen_lines, 2);

    let update = fx.session.set_seen(0, "a.txt", &[1], false).await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Unseen, HunkState::Unseen]);
    assert!(
        fx.store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the open review")
            .spans
            .is_empty()
    );
}

#[tokio::test]
async fn completion_counts_the_seen_hunks_of_a_file_and_of_the_changeset() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(
        &[("a.txt", &base), ("b.txt", &base)],
        &[("a.txt", &numbered(30, &[3, 20])), ("b.txt", &numbered(30, &[5]))],
    )
    .await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!((m.files[0].completion.hunks, m.files[0].completion.seen_hunks), (2, 0));
    assert_eq!((m.completion.hunks, m.completion.seen_hunks), (3, 0));

    let update = fx.session.set_seen(0, "a.txt", &[1], true).await.unwrap();
    assert_eq!((update.completion.hunks, update.completion.seen_hunks), (2, 1));
    assert!(!update.completion.is_complete());
    assert_eq!((update.changeset.hunks, update.changeset.seen_hunks), (3, 1));

    let update = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!((update.completion.hunks, update.completion.seen_hunks), (2, 2));
    assert!(update.completion.is_complete() && !update.changeset.is_complete());
}

#[tokio::test]
async fn unmarking_cuts_a_hunk_out_of_a_larger_span() {
    let base = numbered(40, &[]);
    let region: Vec<usize> = (5..25).collect();
    let dirty = numbered(40, &region);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    // Reverting the lower part shrinks the hunk to 5..10; its base side lies
    // inside the old span's 5..25.
    let shorter = numbered(40, &(5..10).collect::<Vec<_>>());
    let rev = fx.settle(&[("a.txt", &shorter)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
    let update = fx.session.set_seen(rev, "a.txt", &[0], false).await.unwrap();

    assert_eq!(update.review.hunks, [HunkState::Unseen]);
    assert_eq!(update.completion.seen_lines, 0);
    let mem = fx.store.0.lock().unwrap();
    let left: Vec<_> = mem
        .reviews
        .last()
        .expect("the open review")
        .spans
        .iter()
        .flat_map(|(_, s)| s.base.iter().chain(&s.target))
        .map(|a| (a.side == Side::Base, a.range.start, a.range.end))
        .collect();
    assert_eq!(left, [(true, 10, 25), (false, 10, 25)], "the rest stays as recorded");
}

#[tokio::test]
async fn lines_lost_to_an_edit_come_back_after_unmarking_elsewhere_and_a_revert() {
    let region: Vec<usize> = (5..25).collect();
    let dirty = numbered(40, &region);
    let fx = Fixture::open(&[("a.txt", &numbered(40, &[]))], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();

    // Three lines in the middle go back to their base text: two hunks around them.
    let edited = dirty
        .replace("edited 14", "line 14")
        .replace("edited 15", "line 15")
        .replace("edited 16", "line 16");
    let rev = fx.settle(&[("a.txt", &edited)]).await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Seen]);
    fx.session.set_seen(rev, "a.txt", &[0], false).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Unseen, HunkState::Seen]);

    // Reverting the edit: the second part is seen again, including the three lines.
    fx.settle(&[("a.txt", &dirty)]).await;
    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.files[0].review.hunks, [HunkState::Unseen]);
    // Base and target lines 14..25 are seen.
    assert_eq!((m.completion.lines, m.completion.seen_lines), (40, 22));
}

#[tokio::test]
async fn requests_for_an_old_revision_are_rejected() {
    let fx = Fixture::open(&[("a.txt", &numbered(5, &[]))], &[("a.txt", &numbered(5, &[1]))]).await;
    fx.settle(&[("a.txt", &numbered(5, &[2]))]).await;

    let err = fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap_err();
    assert!(matches!(err, SessionError::Stale { current: 1 }));
    let err = fx.session.set_seen(1, "a.txt", &[5], true).await.unwrap_err();
    assert!(matches!(err, SessionError::NotFound(_)));
    assert!(
        fx.store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the open review")
            .spans
            .is_empty()
    );
}

#[tokio::test]
async fn items_of_one_file_are_acknowledged_separately() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("x.bin", "a\0\n")]);
    let mut dirty = git.snapshot(&[("x.bin", "b\0\n")]);
    dirty.entries.get_mut(&path("x.bin")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.completion.items, 2);
    let update = session
        .acknowledge(0, "x.bin", NonTextKind::Binary, true)
        .await
        .unwrap();

    assert_eq!(update.review.acknowledged, [NonTextKind::Binary]);
    assert_eq!((update.changeset.items, update.changeset.acknowledged_items), (2, 1));
}

/// `run.sh` is edited and made executable: a hunk and a mode change.
async fn edited_script() -> (LiveSession<FakeGit, MemStore>, MemStore) {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", &numbered(10, &[]))]);
    let mut dirty = git.snapshot(&[("run.sh", &numbered(10, &[2, 7]))]);
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let store = MemStore::default();
    let session = LiveSession::open(git, store.clone(), REPO.into(), clock).await.unwrap();
    (session, store)
}

#[tokio::test]
async fn toggling_a_file_marks_all_of_it_and_toggling_again_takes_it_back() {
    let (session, store) = edited_script().await;

    // Partly seen is not seen: everything gets marked.
    session.set_seen(0, "run.sh", &[0], true).await.unwrap();
    let update = session.toggle_file_seen(0, View::Changeset, "run.sh").await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Seen, HunkState::Seen]);
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);
    assert!(update.completion.is_complete() && update.changeset.is_complete());
    assert_eq!(
        store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the open review")
            .acks
            .len(),
        1
    );

    let update = session.toggle_file_seen(0, View::Changeset, "run.sh").await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Unseen, HunkState::Unseen]);
    assert!(update.review.acknowledged.is_empty());
    assert!(!update.completion.is_complete());
    let mem = store.0.lock().unwrap();
    let review = mem.reviews.last().expect("the open review");
    assert!(review.spans.is_empty() && review.acks.is_empty());
}

#[tokio::test]
async fn toggling_a_hunk_flips_what_is_stored() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3, 4]))]).await;

    let update = fx.session.toggle_hunk(0, View::Changeset, "a.txt", 0).await.unwrap();
    assert!(matches!(update.started, Some(Lifecycle::Open { .. })));
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    let update = fx.session.toggle_hunk(0, View::Changeset, "a.txt", 0).await.unwrap();
    assert_eq!(
        (update.review.hunks.clone(), update.started),
        (vec![HunkState::Unseen], None)
    );

    let missing = fx.session.toggle_hunk(0, View::Changeset, "a.txt", 3).await;
    assert!(matches!(missing, Err(SessionError::NotFound(_))));
}

#[tokio::test]
async fn toggling_a_hunk_seen_against_an_old_base_marks_it_seen_again() {
    let fx = seen_then_base_changed().await;
    let revision = fx.session.revision().await;
    assert_eq!(fx.hunks("a.txt").await, [HunkState::SeenAgainstOldBase]);

    let update = fx
        .session
        .toggle_hunk(revision, View::Changeset, "a.txt", 0)
        .await
        .unwrap();

    assert_eq!(update.review.hunks, [HunkState::Seen]);
}

#[tokio::test]
async fn toggling_an_item_flips_its_acknowledgement() {
    let (session, store) = edited_script().await;

    let update = session
        .toggle_item(0, View::Changeset, "run.sh", NonTextKind::Mode)
        .await
        .unwrap();
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);
    assert!(matches!(update.started, Some(Lifecycle::Open { .. })));
    assert_eq!(
        store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the open review")
            .acks
            .len(),
        1
    );

    let update = session
        .toggle_item(0, View::Changeset, "run.sh", NonTextKind::Mode)
        .await
        .unwrap();
    assert!(update.review.acknowledged.is_empty());
    assert!(
        store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the open review")
            .acks
            .is_empty()
    );
}

#[tokio::test]
async fn toggling_a_file_with_only_a_mode_change_acknowledges_it_and_starts_the_review() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", "echo\n")]);
    let mut dirty = base.clone();
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest(View::Changeset).await.unwrap();
    assert_eq!((m.completion.items, m.completion.acknowledged_items), (1, 0));
    assert!(!m.completion.is_complete());
    assert!(m.files[0].hunks.is_empty());

    let update = session.toggle_file_seen(0, View::Changeset, "run.sh").await.unwrap();

    assert!(matches!(update.started, Some(Lifecycle::Open { version: 1, .. })));
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);
    assert!(update.changeset.is_complete());
}

#[tokio::test]
async fn only_a_binary_image_has_a_preview_and_a_mode_item_says_how_the_mode_changed() {
    let git = FakeGit::default();
    let mut base = git.snapshot(&[
        ("pic.png", "a\0"),
        ("data.bin", "a\0"),
        ("gained.sh", "echo\n"),
        ("lost.sh", "echo\n"),
        ("other.txt", "echo\n"),
    ]);
    base.entries.get_mut(&path("lost.sh")).unwrap().mode = 0o100755;
    base.id = merkle::snapshot_id(&base.entries);
    let mut dirty = git.snapshot(&[
        ("pic.png", "b\0"),
        ("data.bin", "b\0"),
        ("gained.sh", "echo\n"),
        ("lost.sh", "echo\n"),
        ("other.txt", "echo\n"),
    ]);
    dirty.entries.get_mut(&path("gained.sh")).unwrap().mode = 0o100755;
    dirty.entries.get_mut(&path("other.txt")).unwrap().mode = 0o100664;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest(View::Changeset).await.unwrap();

    let item = |file: &str, kind| {
        let file = m.files.iter().find(|f| f.path == path(file)).unwrap();
        file.non_text.iter().find(|i| i.kind == kind).unwrap().clone()
    };
    assert!(item("pic.png", NonTextKind::Binary).has_preview);
    assert!(!item("data.bin", NonTextKind::Binary).has_preview);
    let change = |file| item(file, NonTextKind::Mode).mode_change;
    assert_eq!(change("gained.sh"), Some(ModeChange::GainedExecutable));
    assert_eq!(change("lost.sh"), Some(ModeChange::LostExecutable));
    assert_eq!(change("other.txt"), Some(ModeChange::Other));
    assert_eq!(item("pic.png", NonTextKind::Binary).mode_change, None);
}

#[tokio::test]
async fn submodule_changes_carry_both_commits() {
    let git = FakeGit::default();
    let sub = |commit: &[u8]| FileEntry {
        blob: BlobId::of(commit),
        mode: 0o160000,
        kind: EntryKind::Submodule,
        unfiltered: false,
    };
    let tree = |commit: &[u8]| {
        let entries = [(path("lib"), sub(commit))].into();
        Snapshot {
            id: merkle::snapshot_id(&entries),
            entries,
        }
    };
    *git.base.lock().unwrap() = Some(tree(b"one"));
    git.scans.lock().unwrap().push(tree(b"two"));
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest(View::Changeset).await.unwrap();

    let item = &m.files[0].non_text[0];
    assert_eq!(item.kind, NonTextKind::Submodule);
    let commit = |blob: &[u8]| Some(format!("{:02x}", BlobId::of(blob).0.0[0]));
    assert_eq!(item.before.as_ref().unwrap().target, commit(b"one"));
    assert_eq!(item.after.as_ref().unwrap().target, commit(b"two"));
}

#[tokio::test]
async fn the_commits_are_held_while_the_review_is_open() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let held = |fx: &Fixture| fx.git.refs.lock().unwrap().clone();
    assert!(held(&fx).is_empty());

    fx.begin().await;
    assert_eq!(
        held(&fx),
        [
            ("1/base".to_string(), CommitId("head".into())),
            ("1/tip".to_string(), CommitId("head".into()))
        ]
    );

    fx.session.finish_review().await.unwrap();

    assert!(held(&fx).is_empty());
    assert!(
        fx.store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the finished review")
            .closed_at
            .is_some()
    );
}

impl Fixture {
    /// Vet restarts: a new session on the same git and store, with the tree as
    /// `dirty` now.
    async fn reopen(&self, dirty: &[(&str, &str)]) -> LiveSession<FakeGit, MemStore> {
        let scan = self.git.snapshot(dirty);
        self.git.scans.lock().unwrap().push(scan);
        LiveSession::open(self.git.clone(), self.store.clone(), REPO.into(), clock)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn reopening_resumes_the_open_review_and_remaps_edits_made_while_closed() {
    let base = numbered(30, &[]);
    let dirty = numbered(30, &[3, 20]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    let range = LineRange { start: 20, end: 21 };
    let thread = NewThread::Range {
        side: Side::Target,
        path: path("a.txt"),
        range,
    };
    fx.session
        .create_thread(0, View::Changeset, thread, "why".into(), 1)
        .await
        .unwrap();
    let before = fx.session.manifest(View::Changeset).await.unwrap();

    assert_eq!(before.files[0].review.hunks, [HunkState::Seen, HunkState::Unseen]);

    // Another hunk appeared while Vet was closed.
    let edited = dirty.replace("line 25", "changed 25");
    let session = fx.reopen(&[("a.txt", &edited)]).await;

    let after = session.manifest(View::Changeset).await.unwrap();
    assert_eq!(after.revision, 1, "the tree on disk is a new settled snapshot");
    assert!(matches!(after.lifecycle, Lifecycle::Open { version: 2, .. }));
    assert_eq!(
        after.files[0].review.hunks,
        [HunkState::Seen, HunkState::Unseen, HunkState::Unseen]
    );
    let threads = session.threads(View::Changeset).await.unwrap().threads;
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].messages[0].body, "why");
    assert_eq!(threads[0].location.as_ref().map(|l| l.range), Some(range));
    assert_eq!(fx.store.reviewed_versions().len(), 2);
}

#[tokio::test]
async fn resuming_an_unchanged_tree_does_not_settle_again() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    assert_eq!(session.revision().await, 0);
    assert!(matches!(session.lifecycle().await, Lifecycle::Open { version: 1, .. }));
    assert_eq!(fx.store.reviewed_versions().len(), 1);
}

#[tokio::test]
async fn reopening_without_a_review_browses_again() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.settle(&[("a.txt", "three\n")]).await;

    let session = fx.reopen(&[("a.txt", "three\n")]).await;

    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);
    assert_eq!(session.revision().await, 0);
}

#[tokio::test]
async fn a_resumed_session_uses_the_repository_filter_and_keeps_its_base_and_refs() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let filter = ReviewFilter {
        hide_binary: true,
        ..Default::default()
    };
    fx.session.set_filter(filter.clone()).await.unwrap();

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    let m = session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.filter, filter);
    assert_eq!(m.base.commit.id, CommitId("head".into()));
    assert_eq!(fx.git.refs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_session_opens_with_the_repository_filter_and_keeps_it_across_reviews() {
    let git = FakeGit::default();
    *git.base.lock().unwrap() = Some(git.snapshot(&[("a.txt", "1\n"), ("docs/b.md", "1\n")]));
    let dirty = git.snapshot(&[("a.txt", "2\n"), ("docs/b.md", "2\n")]);
    git.scans.lock().unwrap().push(dirty);
    let filter = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };
    let store = MemStore::default();
    store.0.lock().unwrap().filters.insert(REPO.into(), filter.clone());

    let session = LiveSession::open(git, store.clone(), REPO.into(), clock).await.unwrap();

    let browsing = session.manifest(View::Changeset).await.unwrap();
    assert_eq!((&browsing.filter, browsing.files.len()), (&filter, 1));
    session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    session.finish_review().await.unwrap();
    assert_eq!(session.manifest(View::Changeset).await.unwrap().filter, filter);
    assert_eq!(store.0.lock().unwrap().filters.get(REPO).cloned(), Some(filter));
}

#[tokio::test]
async fn a_filter_set_while_browsing_is_stored_and_applies_to_the_review() {
    let fx = Fixture::open(&[("a.txt", "1\n")], &[("a.txt", "2\n")]).await;
    let filter = ReviewFilter {
        hide_binary: true,
        ..Default::default()
    };
    fx.session.set_filter(filter.clone()).await.unwrap();
    assert_eq!(
        fx.store.0.lock().unwrap().filters.get(REPO).cloned(),
        Some(filter.clone())
    );

    fx.session.set_seen(1, "a.txt", &[0], true).await.unwrap();

    assert_eq!(fx.session.manifest(View::Changeset).await.unwrap().filter, filter);
    assert_eq!(fx.store.0.lock().unwrap().filters.get(REPO).cloned(), Some(filter));
}

#[tokio::test]
async fn a_resumed_review_keeps_its_base_commit_however_head_moved() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    move_head(&fx.git, "h2", &["head"]);

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    let m = session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.base.commit.id, CommitId("head".into()));
}

#[tokio::test]
async fn a_review_whose_snapshots_are_gone_is_closed_and_browsing_starts() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.store.0.lock().unwrap().snapshots.clear();

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);
    assert!(
        fx.store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the finished review")
            .closed_at
            .is_some()
    );
}

#[tokio::test]
async fn a_finished_review_is_not_resumed() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    fx.session.finish_review().await.unwrap();

    let session = fx.reopen(&[("a.txt", "two\n")]).await;

    assert_eq!(session.lifecycle().await, Lifecycle::Browsing);
    assert_eq!(
        session.manifest(View::Changeset).await.unwrap().files[0].review.hunks,
        [HunkState::Unseen]
    );
}

/// With no branches, the default base is `HEAD` itself.
fn detached_head() -> BaseChoice {
    BaseChoice::Commit {
        id: CommitId("head".into()),
    }
}

#[tokio::test]
async fn the_working_tree_is_compared_against_the_head_not_the_checked_out_branchs_tip() {
    let git = FakeGit::default();
    *git.base.lock().unwrap() = Some(git.snapshot(&[("a.txt", "one\n")]));
    let scan = git.snapshot(&[("a.txt", "two\n")]);
    git.scans.lock().unwrap().push(scan);
    git.add_branch("feature", true, "f3", &["f2", "f1"]);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    let m = session.manifest(View::Changeset).await.unwrap();

    assert_eq!(m.base.commit.id, CommitId("head".into()));
    assert_eq!(m.target.name, "Working tree");
    assert_eq!(m.target.branch.as_deref(), Some("feature"));
    assert_eq!(m.base.commit.subject, "subject of head");
    assert_eq!(m.base.into, None);
}

#[tokio::test]
async fn a_pull_request_into_a_branch_other_than_the_default_says_so() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "one\n")]);
    fetch_pull_request_7(&fx.git, &[("a.txt", "one\n")], "pr1", &[("a.txt", "two\n")]);
    let into = |m: ManifestView| m.changeset_base.into;
    let pull_request = |target: &str| PullRequestHead {
        title: "Fix".into(),
        draft: false,
        fork: false,
        target_branch: target.into(),
    };

    fx.store
        .0
        .lock()
        .unwrap()
        .heads
        .insert((REPO.into(), pull_request_7()), pull_request("release/2.1"));
    fx.session.open_changeset(pull_request_7()).await.unwrap();
    assert_eq!(
        into(fx.session.manifest(View::Changeset).await.unwrap()).as_deref(),
        Some("release/2.1")
    );

    fx.store
        .0
        .lock()
        .unwrap()
        .heads
        .insert((REPO.into(), pull_request_7()), pull_request("main"));
    assert_eq!(into(fx.session.manifest(View::Changeset).await.unwrap()), None);
}

#[tokio::test]
async fn a_pull_request_is_headed_with_the_title_the_platform_last_named_it_and_a_branch_is_not() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "one\n")]);
    fetch_pull_request_7(&fx.git, &[("a.txt", "one\n")], "pr1", &[("a.txt", "two\n")]);
    fx.session.open_changeset(pull_request_7()).await.unwrap();

    // The platform has not said anything of the pull request yet, so nothing to be headed with.
    let unnamed = fx.session.manifest(View::Changeset).await.unwrap().target;
    assert_eq!(
        (unnamed.name.as_str(), unnamed.title.is_none(), unnamed.draft),
        ("#7", true, false)
    );

    // What the platform named it last is what it is headed with.
    fx.store.0.lock().unwrap().heads.insert(
        (REPO.into(), pull_request_7()),
        PullRequestHead {
            title: "Add idempotency keys".into(),
            draft: true,
            fork: false,
            target_branch: "main".into(),
        },
    );
    let named = fx.session.manifest(View::Changeset).await.unwrap().target;
    assert_eq!(
        (named.title.as_deref(), named.draft),
        (Some("Add idempotency keys"), true)
    );

    // A branch is not a pull request: its target says nothing of a platform.
    fx.session.open_changeset(feature()).await.unwrap().unwrap();
    let branch = fx.session.manifest(View::Changeset).await.unwrap().target;
    assert_eq!((branch.title.as_deref(), branch.draft), (None, false));
}

#[tokio::test]
async fn a_pull_request_its_target_branch_holds_is_empty_even_when_git_cannot_say_more() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    // The head is the commit it left the target branch at, and the fake cannot resolve the
    // target branch itself.
    fetch_pull_request_7(&fx.git, &[("a.txt", "one\n")], "m1", &[("a.txt", "one\n")]);
    fx.store.0.lock().unwrap().heads.insert(
        (REPO.into(), pull_request_7()),
        PullRequestHead {
            title: "Fix".into(),
            draft: false,
            fork: false,
            target_branch: "release/2.1".into(),
        },
    );
    fx.session.open_changeset(pull_request_7()).await.unwrap();

    let m = fx.session.manifest(View::Changeset).await.unwrap();

    assert!(m.files.is_empty());
    let contained = m.base.contained_in.expect("the target branch holds the head");
    assert_eq!(
        (contained.branch.as_str(), contained.same_tip, contained.merged_by),
        ("release/2.1", false, None)
    );
}

#[tokio::test]
async fn the_folders_of_what_is_left_out_count_every_file_not_only_the_listed_ones() {
    let names: Vec<String> = (0..600).map(|i| format!("big/dir{}/f{i}.txt", i % 3)).collect();
    let base: Vec<(&str, &str)> = [("small/x.txt", "1\n")]
        .into_iter()
        .chain(names.iter().map(|n| (n.as_str(), "1\n")))
        .collect();
    let dirty: Vec<(&str, &str)> = base.iter().map(|(n, _)| (*n, "2\n")).collect();
    let fx = Fixture::open(&base, &dirty).await;
    let filter = ReviewFilter {
        roots: vec!["small".into()],
        ..Default::default()
    };
    fx.session.set_filter(filter).await.unwrap();

    let m = fx.session.manifest(View::Changeset).await.unwrap();

    let group = &m.excluded[0];
    assert_eq!((group.count, group.files.len()), (600, 500));
    let folders: Vec<_> = group
        .folders
        .iter()
        .map(|f| (f.path.as_str(), f.dir, f.files))
        .collect();
    assert_eq!(folders, [("big", true, 600)]);
}

#[tokio::test]
async fn after_bringing_one_rule_back_a_file_is_left_out_by_the_rules_that_still_apply() {
    let files = [("src/a.rs", "1\n"), ("tests/x.snap", "1\n"), ("docs/d.md", "1\n")];
    let dirty: Vec<(&str, &str)> = files.iter().map(|(n, _)| (*n, "2\n")).collect();
    let fx = Fixture::open(&files, &dirty).await;
    let project = ReviewFilter {
        roots: vec!["src".into()],
        exclude: vec!["**/*.snap".into()],
        ..Default::default()
    };
    fx.session.set_filter(project.clone()).await.unwrap();
    let lifted = ReviewFilter {
        roots: vec![],
        ..project
    };

    fx.session.narrow(&Source::WorkingTree, Some(lifted)).await.unwrap();

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    let shown: Vec<_> = m.files.iter().map(|f| f.path.to_string_lossy()).collect();
    assert_eq!(shown, ["docs/d.md", "src/a.rs"]);
    let left_out: Vec<_> = m.excluded.iter().map(|g| (g.by.clone(), g.count)).collect();
    assert_eq!(
        left_out,
        [(vec![ExcludedBy::Rule(Exclusion::Pattern("**/*.snap".into()))], 1)],
        "the file is still outside the roots, but those are not in force"
    );
}

#[tokio::test]
async fn a_renamed_file_the_filter_leaves_out_can_still_be_read() {
    let text = "one\ntwo\nthree\n";
    let fx = Fixture::open(&[("old/a.txt", text)], &[("new/a.txt", text)]).await;
    let filter = ReviewFilter {
        roots: vec!["elsewhere".into()],
        ..Default::default()
    };
    fx.session.set_filter(filter).await.unwrap();
    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert!(m.files.is_empty());
    assert_eq!(m.excluded[0].files[0].path.to_string_lossy(), "new/a.txt");

    let read = fx.session.target_text(View::Changeset, "new/a.txt").await;

    assert_eq!(read.unwrap().content, Ok(text.to_owned()));
}

#[tokio::test]
async fn a_narrowing_lists_what_it_leaves_out_by_folders_as_deep_as_it_goes_and_root_files_alone() {
    let files = [
        ("lib/a/x.txt", "1\n"),
        ("lib/b/y.txt", "1\n"),
        ("lib/b/deep/z.txt", "1\n"),
        ("lib/c/w.txt", "1\n"),
        ("root.txt", "1\n"),
    ];
    let dirty: Vec<(&str, &str)> = files.iter().map(|(n, _)| (*n, "2\n")).collect();
    let fx = Fixture::open(&files, &dirty).await;
    let narrowing = ReviewFilter {
        roots: vec!["lib/a".into()],
        ..Default::default()
    };
    fx.session.narrow(&Source::WorkingTree, Some(narrowing)).await.unwrap();

    let m = fx.session.manifest(View::Changeset).await.unwrap();

    let folders: Vec<_> = m.excluded[0]
        .folders
        .iter()
        .map(|f| (f.path.as_str(), f.dir, f.files))
        .collect();
    assert_eq!(
        folders,
        [("lib/b", true, 2), ("lib/c", true, 1), ("root.txt", false, 1)]
    );
}

/// A hunk of `a.txt` marked seen in pull request 7, then the author rebased it onto a target
/// branch that lacks "line 4"; `b.txt` changes too and is not marked.
async fn seen_then_base_changed() -> Fixture {
    let base = numbered(10, &[]);
    let pushed = [("a.txt", numbered(10, &[3, 4])), ("b.txt", numbered(10, &[3, 4]))];
    let pushed: Vec<(&str, &str)> = pushed.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let fx = Fixture::open(
        &[("a.txt", &base), ("b.txt", &base)],
        &[("a.txt", &base), ("b.txt", &base)],
    )
    .await;
    fetch_pull_request_7(&fx.git, &[("a.txt", &base), ("b.txt", &base)], "pr1", &pushed);
    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();
    let other = base.replace("line 4\n", "");
    fetch_pull_request_7(&fx.git, &[("a.txt", &other), ("b.txt", &other)], "pr2", &pushed);
    move_base_of_pull_request_7(&fx.git, "mb", &[("a.txt", &other), ("b.txt", &other)]);
    fx.session.observe().await.unwrap().expect("head and base moved");
    fx
}

#[tokio::test]
async fn a_hunk_seen_against_an_old_base_counts_its_lines_and_shows_what_the_base_had() {
    let fx = seen_then_base_changed().await;
    let revision = fx.session.revision().await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    let (a, b) = (&m.files[0].review, &m.files[1].review);
    assert_eq!(
        (&a.hunks[..], &a.old_base_lines[..]),
        (&[HunkState::SeenAgainstOldBase][..], &[2][..])
    );
    assert_eq!(
        (&b.hunks[..], &b.old_base_lines[..]),
        (&[HunkState::Unseen][..], &[0][..])
    );

    let diff = fx.session.base_diff(revision, "a.txt", 0).await.unwrap();
    let rows: Vec<_> = diff.rows.iter().map(|r| (r.kind, r.line, r.text.as_str())).collect();
    // The base had "line 3" and "line 4" there; it has "line 3" now, at line 4 of its text.
    assert_eq!(
        rows,
        [
            (BaseDiffKind::Context, 4, "line 3"),
            (BaseDiffKind::Removed, 5, "line 4")
        ]
    );
    assert_eq!(diff.to.id, CommitId("mb".into()));
    let from = diff
        .from
        .expect("the base the hunk was marked against is a version's base");
    assert_ne!(from.id, diff.to.id);

    let unseen = fx.session.base_diff(revision, "b.txt", 0).await;
    assert!(matches!(unseen, Err(SessionError::Invalid(_))));
}

#[tokio::test]
async fn re_confirming_a_file_marks_its_hunks_seen_against_an_old_base_seen_and_nothing_else() {
    let fx = seen_then_base_changed().await;
    let revision = fx.session.revision().await;

    let nothing = fx.session.reconfirm_file(revision, "b.txt").await;
    assert!(matches!(nothing, Err(SessionError::Invalid(_))));
    assert_eq!(fx.hunks("b.txt").await, [HunkState::Unseen]);

    let update = fx.session.reconfirm_file(revision, "a.txt").await.unwrap();
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    assert_eq!(update.review.old_base_lines, [0]);
    assert_eq!(update.completion.old_base_lines, 0);
    assert_eq!(update.changeset.seen_lines, 3);
}

#[tokio::test]
async fn coverage_survives_git_dropping_the_old_base() {
    let base = numbered(10, &[]);
    let pushed = numbered(10, &[3, 4]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr1", &[("a.txt", &pushed)]);
    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();
    let held = |fx: &Fixture| {
        fx.store
            .0
            .lock()
            .unwrap()
            .blobs
            .contains_key(&BlobId::of(base.as_bytes()))
    };
    assert!(!held(&fx), "git holds the base");

    let other = base.replace("line 4\n", "");
    fetch_pull_request_7(&fx.git, &[("a.txt", &other)], "pr2", &[("a.txt", &pushed)]);
    move_base_of_pull_request_7(&fx.git, "c1-full", &[("a.txt", &other)]);
    fx.session.observe().await.unwrap().expect("head and base moved");
    assert!(held(&fx), "copied when the base changes");

    fx.git.forget(&base);
    assert_eq!(fx.hunks("a.txt").await, [HunkState::SeenAgainstOldBase]);
}

#[tokio::test]
async fn the_content_of_base_side_threads_is_copied_when_the_base_changes() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    let pushed = numbered(10, &[3]);
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr1", &[("a.txt", &pushed)]);
    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();
    let thread = NewThread::Range {
        side: Side::Base,
        path: path("a.txt"),
        range: LineRange { start: 5, end: 7 },
    };
    fx.session
        .create_thread(revision, View::Changeset, thread, "why".into(), 1)
        .await
        .unwrap();

    let other = base.replace("line 0\n", "");
    fetch_pull_request_7(&fx.git, &[("a.txt", &other)], "pr2", &[("a.txt", &pushed)]);
    move_base_of_pull_request_7(&fx.git, "c1-full", &[("a.txt", &other)]);
    fx.session.observe().await.unwrap().expect("head and base moved");

    let mem = fx.store.0.lock().unwrap();
    assert!(mem.blobs.contains_key(&BlobId::of(base.as_bytes())));
}

#[tokio::test]
async fn file_text_returns_both_sides() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n"), ("new.txt", "n\n")]).await;

    let text = fx.session.file_text(View::Changeset, "a.txt").await.unwrap();
    assert_eq!(
        (text.base.as_deref(), text.target.as_deref()),
        (Some("one\n"), Some("two\n"))
    );
    let added = fx.session.file_text(View::Changeset, "new.txt").await.unwrap();
    assert_eq!((added.base, added.target.as_deref()), (None, Some("n\n")));
    assert!(matches!(
        fx.session.file_text(View::Changeset, "nope").await,
        Err(SessionError::NotFound(_))
    ));
}

fn listing(l: &DirListing) -> Vec<(&str, DirEntryKind, bool, bool)> {
    l.entries
        .iter()
        .map(|e| (e.name.as_str(), e.kind, e.changed, e.excluded))
        .collect()
}

#[tokio::test]
async fn directories_list_first_in_byte_order_with_changed_below() {
    use DirEntryKind::{Dir, File};
    let base = [
        ("src/a/deep/x.rs", "x\n"),
        ("src/a-b/y.rs", "y\n"),
        ("src/b.rs", "b\n"),
        ("Z.md", "z\n"),
        ("a.md", "a\n"),
    ];
    let mut dirty = base;
    dirty[0].1 = "changed\n";
    let fx = Fixture::open(&base, &dirty).await;

    let root = fx.session.list_directory("").await.unwrap();
    assert_eq!(
        listing(&root),
        [
            ("src", Dir, true, false),
            ("Z.md", File, false, false),
            ("a.md", File, false, false)
        ]
    );
    let src = fx.session.list_directory("src").await.unwrap();
    assert_eq!(
        listing(&src),
        [
            ("a", Dir, true, false),
            ("a-b", Dir, false, false),
            ("b.rs", File, false, false)
        ]
    );
    let deep = fx.session.list_directory("src/a/deep").await.unwrap();
    assert_eq!(listing(&deep), [("x.rs", File, true, false)]);
    assert_eq!(deep.revision, 0);
    assert!(matches!(
        fx.session.list_directory("src/nope").await,
        Err(SessionError::NotFound(_))
    ));
    assert!(matches!(
        fx.session.list_directory("a.md").await,
        Err(SessionError::NotFound(_))
    ));
}

#[tokio::test]
async fn the_flat_listing_has_every_file_of_the_target_in_byte_order() {
    let base = [("src/a/x.rs", "x\n"), ("src/gone.rs", "g\n"), ("Z.md", "z\n")];
    let dirty = [
        ("src/a/x.rs", "changed\n"),
        ("src/new.rs", "n\n"),
        ("Z.md", "z\n"),
        ("a.md", "a\n"),
    ];
    let fx = Fixture::open(&base, &dirty).await;

    let files = fx.session.list_files().await.unwrap();
    assert_eq!(files.paths, ["Z.md", "a.md", "src/a/x.rs", "src/new.rs"]);
    assert_eq!(files.revision, 0);
}

#[tokio::test]
async fn deleted_and_added_files_flag_their_directories() {
    let fx = Fixture::open(
        &[("gone/f.txt", "f\n"), ("keep/k.txt", "k\n")],
        &[("keep/k.txt", "k\n"), ("new/n.txt", "n\n")],
    )
    .await;

    let root = fx.session.list_directory("").await.unwrap();
    // `gone` has no files left in the target, so it is not listed.
    assert_eq!(
        listing(&root),
        [
            ("keep", DirEntryKind::Dir, false, false),
            ("new", DirEntryKind::Dir, true, false)
        ]
    );
}

#[tokio::test]
async fn the_filter_excludes_files_and_directories_without_a_passing_file() {
    use DirEntryKind::{Dir, File};
    let files = [
        ("docs/a.md", "a\n"),
        ("mixed/b.md", "b\n"),
        ("mixed/c.rs", "c\n"),
        ("d.rs", "d\n"),
    ];
    let fx = Fixture::open(&files, &files).await;
    fx.session.hold().await.filter.include = vec!["*.rs".into()];

    let root = fx.session.list_directory("").await.unwrap();
    assert_eq!(
        listing(&root),
        [
            ("docs", Dir, false, true),
            ("mixed", Dir, false, false),
            ("d.rs", File, false, false)
        ]
    );
    let mixed = fx.session.list_directory("mixed").await.unwrap();
    assert_eq!(
        listing(&mixed),
        [("b.md", File, false, true), ("c.rs", File, false, false)]
    );
}

#[tokio::test]
async fn setting_the_filter_persists_it_and_moves_the_counts() {
    let base = [("a.txt", "1\n"), ("docs/b.md", "1\n")];
    let dirty = [("a.txt", "2\n"), ("docs/b.md", "2\n")];
    let fx = Fixture::reviewing(&base, &dirty).await;
    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(
        (m.files.len(), m.completion.lines, m.completion.excluded_lines),
        (2, 4, 0)
    );

    let filter = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };
    assert_eq!(fx.session.set_filter(filter.clone()).await.unwrap(), 1);
    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!((m.revision, m.filter.clone()), (1, filter.clone()));
    assert_eq!(
        (m.files.len(), m.completion.lines, m.completion.excluded_lines),
        (1, 2, 2)
    );
    assert_eq!(
        fx.store.0.lock().unwrap().filters.get(REPO).cloned(),
        Some(filter.clone())
    );

    // The same filter again changes nothing.
    assert_eq!(fx.session.set_filter(filter).await.unwrap(), 1);

    // An invalid glob is refused and changes nothing.
    let bad = ReviewFilter {
        include: vec!["[".into()],
        ..Default::default()
    };
    let err = fx.session.set_filter(bad).await.unwrap_err();
    assert!(err.to_string().starts_with("invalid pattern \"[\""), "{err}");
    assert_eq!(fx.session.revision().await, 1);
}

#[tokio::test]
async fn excluding_a_path_hides_exactly_that_file_or_everything_below_that_folder() {
    let names = [
        "src/a[1].rs",
        "src/a1.rs",
        "src/gen/x.rs",
        "src/gen/sub/y.rs",
        "src/genx.rs",
        "gen/z.rs",
    ];
    let base: Vec<_> = names.iter().map(|n| (*n, "1\n")).collect();
    let dirty: Vec<_> = names.iter().map(|n| (*n, "2\n")).collect();
    let fx = Fixture::open(&base, &dirty).await;
    let shown = || async {
        let m = fx.session.manifest(View::Changeset).await.unwrap();
        m.files.iter().map(|f| f.path.to_string_lossy()).collect::<Vec<_>>()
    };

    // A glob character in the name matches itself, not other files.
    assert_eq!(fx.session.exclude_path("src/a[1].rs", false).await.unwrap(), 1);
    assert_eq!(
        shown().await,
        [
            "gen/z.rs",
            "src/a1.rs",
            "src/gen/sub/y.rs",
            "src/gen/x.rs",
            "src/genx.rs"
        ]
    );
    // The folder at that path only, not a sibling that starts with its name, nor one deeper down.
    assert_eq!(fx.session.exclude_path("src/gen", true).await.unwrap(), 2);
    assert_eq!(shown().await, ["gen/z.rs", "src/a1.rs", "src/genx.rs"]);

    // Excluding it again changes nothing; the filter is stored.
    assert_eq!(fx.session.exclude_path("src/gen", true).await.unwrap(), 2);
    let stored = fx.store.0.lock().unwrap().filters.get(REPO).cloned().unwrap();
    assert_eq!(stored.exclude, ["/src/a[[]1[]].rs", "/src/gen/**"]);
}

#[tokio::test]
async fn filter_roots_are_taken_as_directories_without_slashes() {
    let files = [("a.txt", "1\n"), ("docs/b.md", "1\n")];
    let fx = Fixture::open(&files, &[("a.txt", "2\n"), ("docs/b.md", "2\n")]).await;
    let filter = ReviewFilter {
        roots: vec![path("/docs/"), path("/")],
        ..Default::default()
    };
    fx.session.set_filter(filter).await.unwrap();
    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.filter.roots, [path("docs")]);
    assert_eq!(m.files.len(), 1);
    assert_eq!(
        fx.store.0.lock().unwrap().filters.get(REPO).unwrap().roots,
        [path("docs")]
    );
}

#[test]
fn only_common_image_formats_have_a_media_type() {
    let media = |p: &str| image_media_type(&path(p));
    assert_eq!(media("a/b.PNG"), Some("image/png"));
    assert_eq!(media("x.jpeg"), Some("image/jpeg"));
    assert_eq!(media("x.svg"), Some("image/svg+xml"));
    assert_eq!(media("x.txt"), None);
    assert_eq!(media("png"), None);
    assert_eq!(media("dir.png/file"), None);
}

#[tokio::test]
async fn target_text_reads_unchanged_files_and_explains_missing_text() {
    let fx = Fixture::open(
        &[("a.txt", "one\n"), ("bin", "x\0y"), ("link", "a.txt")],
        &[("a.txt", "one\n"), ("bin", "x\0y"), ("link", "a.txt")],
    )
    .await;
    // The target is a plain file; make `link` a symlink entry.
    let mut st = fx.session.hold().await;
    Arc::make_mut(&mut st.target)
        .entries
        .get_mut(&path("link"))
        .unwrap()
        .kind = EntryKind::Symlink;
    drop(st);

    let text = fx.session.target_text(View::Changeset, "a.txt").await.unwrap();
    assert_eq!((text.revision, text.content), (0, Ok("one\n".into())));
    assert_eq!(
        fx.session.target_text(View::Changeset, "bin").await.unwrap().content,
        Err(NoText::Binary)
    );
    assert_eq!(
        fx.session.target_text(View::Changeset, "link").await.unwrap().content,
        Err(NoText::NotAFile)
    );
    // Over the cap, and not retained by the store (as the scan would do).
    let big = "x".repeat(100);
    fx.git.snapshot(&[("big.txt", &big)]);
    let mut st = fx.session.hold().await;
    Arc::make_mut(&mut st.target)
        .entries
        .insert(path("big.txt"), entry(&big, 0o100644));
    drop(st);
    *fx.git.cap.lock().unwrap() = Some(10);
    assert_eq!(
        fx.session
            .target_text(View::Changeset, "big.txt")
            .await
            .unwrap()
            .content,
        Err(NoText::Unavailable)
    );
    *fx.git.cap.lock().unwrap() = None;
    assert_eq!(
        fx.session
            .target_text(View::Changeset, "big.txt")
            .await
            .unwrap()
            .content,
        Ok(big)
    );
    // Content no source has any more.
    fx.git.blobs.lock().unwrap().clear();
    fx.store.0.lock().unwrap().blobs.clear();
    assert_eq!(
        fx.session.target_text(View::Changeset, "a.txt").await.unwrap().content,
        Err(NoText::Unavailable)
    );
    assert!(matches!(
        fx.session.target_text(View::Changeset, "nope").await,
        Err(SessionError::NotFound(_))
    ));
}

#[tokio::test]
async fn settling_moves_the_listing_to_the_new_revision() {
    let fx = Fixture::open(&[("a/x.txt", "x\n")], &[("a/x.txt", "x\n")]).await;
    assert!(!fx.session.list_directory("a").await.unwrap().entries[0].changed);

    fx.settle(&[("a/x.txt", "y\n")]).await;

    let after = fx.session.list_directory("").await.unwrap();
    assert_eq!(after.revision, 1);
    assert!(after.entries[0].changed);
}

fn on(file: &str, start: u32, end: u32) -> NewThread {
    NewThread::Range {
        side: Side::Target,
        path: path(file),
        range: LineRange { start, end },
    }
}

fn at(file: &str, start: u32, end: u32) -> Option<Location> {
    Some(Location {
        file: path(file),
        range: LineRange { start, end },
    })
}

impl Fixture {
    async fn thread(&self, target: NewThread) -> ThreadId {
        let revision = self.session.revision().await;
        let view = self
            .session
            .create_thread(revision, View::Changeset, target, "why?".into(), 100)
            .await
            .unwrap();
        view.id
    }

    async fn view(&self, id: ThreadId) -> ThreadView {
        let all = self.session.threads(View::Changeset).await.unwrap();
        all.threads.into_iter().find(|t| t.id == id).unwrap()
    }
}

#[tokio::test]
async fn a_thread_whose_anchor_cannot_be_placed_is_not_created() {
    let text = numbered(10, &[]);
    let fx = Fixture::open(
        &[("a.txt", "one\n"), ("b.txt", &text)],
        &[("a.txt", "two\n"), ("b.txt", &text)],
    )
    .await;

    // The unchanged file's text is only in git, which cannot be read.
    fx.git.reads_fail.store(true, Ordering::Relaxed);
    let created = fx
        .session
        .create_thread(0, View::Changeset, on("b.txt", 2, 4), "why?".into(), 100)
        .await;
    fx.git.reads_fail.store(false, Ordering::Relaxed);

    assert!(created.is_err());
    assert!(fx.store.0.lock().unwrap().threads_of(1).is_empty());
    assert!(fx.session.threads(View::Changeset).await.unwrap().threads.is_empty());
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
}

#[tokio::test]
async fn threads_read_back_from_the_store_that_cannot_be_placed_leave_the_threads_as_they_were() {
    let text = numbered(10, &[]);
    let fx = Fixture::open(
        &[("a.txt", "one\n"), ("b.txt", &text)],
        &[("a.txt", "two\n"), ("b.txt", &text)],
    )
    .await;
    let kept = fx.thread(on("a.txt", 0, 1)).await;
    // A thread arrives in the store, anchored in a file whose text is only in git.
    let target = fx.session.target_snapshot().await;
    let anchor = Anchor::from_text(
        Side::Target,
        target.id,
        path("b.txt"),
        text.as_bytes(),
        LineRange { start: 2, end: 4 },
    )
    .unwrap();
    fx.store
        .writes()
        .add_thread(
            ChangesetId(1),
            None,
            CommentTarget::Range(anchor),
            Message::written("lena".into(), "hi".into(), 1),
            None,
            ThreadStatus::Open,
        )
        .await
        .unwrap();

    fx.git.reads_fail.store(true, Ordering::Relaxed);
    let mut st = fx.session.hold().await;
    let reloaded = fx.session.reload_threads(&mut st, ChangesetId(1)).await;
    drop(st);
    fx.git.reads_fail.store(false, Ordering::Relaxed);

    assert!(reloaded.is_err());
    assert_eq!(thread_ids(&fx.session).await, [kept]);
}

#[tokio::test]
async fn a_range_thread_that_has_no_place_yet_is_an_error_not_a_panic() {
    let fx = Fixture::open(&[("a.txt", &numbered(10, &[]))], &[("a.txt", &numbered(10, &[3]))]).await;
    fx.thread(on("a.txt", 2, 4)).await;
    fx.session
        .hold()
        .await
        .changes
        .as_mut()
        .unwrap()
        .remapped
        .resolutions
        .clear();

    assert!(matches!(
        fx.session.threads(View::Changeset).await,
        Err(SessionError::Invalid(_))
    ));
}

const EXACT: AnchorState = AnchorState::Exact {
    whitespace_changed: false,
};

#[tokio::test]
async fn a_thread_follows_an_edit_above_it() {
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(30, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 10, 13)).await;
    let view = fx.view(id).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("a.txt", 10, 13)));

    fx.settle(&[("a.txt", &format!("new 1\nnew 2\n{dirty}"))]).await;

    let view = fx.view(id).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("a.txt", 12, 15)));
    assert_eq!(view.path, Some(path("a.txt")));
    let CommentTarget::Range(recorded) = view.target else {
        panic!()
    };
    assert_eq!(recorded.range, LineRange { start: 10, end: 13 }, "never rewritten");
}

fn on_base(file: &str, start: u32, end: u32) -> NewThread {
    NewThread::Range {
        side: Side::Base,
        path: path(file),
        range: LineRange { start, end },
    }
}

impl Fixture {
    /// The target line the card of a new thread is shown below.
    async fn card_line(&self, target: NewThread) -> Option<u32> {
        let id = self.thread(target).await;
        self.view(id).await.target_line
    }
}

#[tokio::test]
async fn a_cards_line_is_the_end_of_the_range_in_the_target_text() {
    // Base lines 3 and 4 became one line, at 3.
    let base = numbered(10, &[]);
    let dirty = base.replace("line 3\nline 4\n", "changed\n");
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;

    assert_eq!(fx.card_line(on("a.txt", 5, 8)).await, Some(7));
    assert_eq!(fx.card_line(on("a.txt", 3, 4)).await, Some(3));
    // Before the hunk, in it, and after it, from the base side.
    assert_eq!(fx.card_line(on_base("a.txt", 0, 2)).await, Some(1));
    assert_eq!(fx.card_line(on_base("a.txt", 4, 5)).await, Some(3));
    assert_eq!(fx.card_line(on_base("a.txt", 3, 5)).await, Some(3));
    assert_eq!(fx.card_line(on_base("a.txt", 7, 9)).await, Some(7));
}

#[tokio::test]
async fn a_line_without_a_thread_is_placed_in_the_target_like_a_cards() {
    let base = numbered(10, &[]);
    let dirty = base.replace("line 3\nline 4\n", "changed\n");
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    let revision = fx.session.revision().await;
    let at = |side, line| {
        fx.session
            .line_in_target(revision, View::Changeset, side, "a.txt", line)
    };

    assert_eq!(at(Side::Target, 7).await.unwrap(), 7);
    assert_eq!(at(Side::Base, 0).await.unwrap(), 0);
    assert_eq!(at(Side::Base, 4).await.unwrap(), 3);
    assert_eq!(at(Side::Base, 8).await.unwrap(), 7);
    // Not past the last line with text.
    assert_eq!(at(Side::Base, 9).await.unwrap(), 8);
    assert_eq!(at(Side::Target, 40).await.unwrap(), 8);

    let stale = fx
        .session
        .line_in_target(revision + 1, View::Changeset, Side::Base, "a.txt", 4)
        .await;
    assert!(matches!(stale, Err(SessionError::Stale { .. })));
}

#[tokio::test]
async fn a_card_on_deleted_lines_sits_where_they_were() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", "line 0\nline 1\nline 2\n")]).await;
    // The deleted lines face the last line that is left, not one past it.
    assert_eq!(fx.card_line(on_base("a.txt", 4, 6)).await, Some(2));
    assert_eq!(fx.card_line(on_base("a.txt", 9, 10)).await, Some(2));
}

#[tokio::test]
async fn a_card_in_a_deleted_file_sits_on_the_first_line() {
    let fx = Fixture::open(
        &[("a.txt", &numbered(10, &[])), ("keep.txt", "x\n")],
        &[("keep.txt", "x\n")],
    )
    .await;
    assert_eq!(fx.card_line(on_base("a.txt", 6, 8)).await, Some(0));
}

#[tokio::test]
async fn a_card_in_an_unchanged_file_sits_below_its_own_line() {
    let fx = Fixture::open(
        &[("a.txt", &numbered(10, &[])), ("same.txt", &numbered(5, &[]))],
        &[("a.txt", &numbered(10, &[2])), ("same.txt", &numbered(5, &[]))],
    )
    .await;
    assert_eq!(fx.card_line(on("same.txt", 1, 4)).await, Some(3));
    assert_eq!(fx.card_line(on_base("same.txt", 1, 4)).await, Some(3));
    // A replaced line is one line either way.
    assert_eq!(fx.card_line(on_base("a.txt", 2, 3)).await, Some(2));
}

#[tokio::test]
async fn file_and_changeset_threads_have_no_card_line() {
    let fx = Fixture::open(&[("a.txt", "x\n")], &[("a.txt", "y\n")]).await;
    for target in [
        NewThread::File {
            side: Side::Target,
            path: path("a.txt"),
        },
        NewThread::Changeset,
    ] {
        assert_eq!(fx.card_line(target).await, None);
    }
}

#[tokio::test]
async fn a_line_ending_change_keeps_the_anchor_exact() {
    let dirty = numbered(10, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(10, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 2, 5)).await;

    fx.settle(&[("a.txt", &dirty.replace('\n', "\r\n"))]).await;

    let view = fx.view(id).await;
    assert_eq!(
        view.state,
        Some(AnchorState::Exact {
            whitespace_changed: true
        })
    );
    assert_eq!(view.location, at("a.txt", 2, 5));
}

#[tokio::test]
async fn an_edit_inside_the_range_is_inferred_until_confirmed() {
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(30, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 10, 13)).await;

    let rev = fx.settle(&[("a.txt", &numbered(30, &[3, 11]))]).await;

    let view = fx.view(id).await;
    let Some(AnchorState::Inferred { strategy, .. }) = view.state else {
        panic!("{:?}", view.state)
    };
    assert_eq!(
        (strategy, view.location),
        (Strategy::ContextWindow, at("a.txt", 10, 13))
    );

    // Confirming from an older revision is refused.
    let err = fx.session.confirm_anchor(rev - 1, id).await.unwrap_err();
    assert!(matches!(err, SessionError::Stale { .. }));
    let confirmed = fx.session.confirm_anchor(rev, id).await.unwrap();
    assert_eq!(
        (confirmed.state, confirmed.location),
        (Some(EXACT), at("a.txt", 10, 13))
    );
    let stored = fx.store.0.lock().unwrap().threads_of(1)[0].thread.target.clone();
    let CommentTarget::Range(anchor) = stored else { panic!() };
    assert_eq!(anchor.snapshot, fx.session.hold().await.target.id);
    // An exact anchor cannot be confirmed again.
    let err = fx.session.confirm_anchor(rev, id).await.unwrap_err();
    assert!(matches!(err, SessionError::Invalid(_)));
}

#[tokio::test]
async fn resolution_starts_from_the_recorded_anchor() {
    let dirty = numbered(30, &[3]);
    let fx = Fixture::open(&[("a.txt", &numbered(30, &[]))], &[("a.txt", &dirty)]).await;
    let id = fx.thread(on("a.txt", 10, 13)).await;

    fx.settle(&[("a.txt", &numbered(30, &[3, 11]))]).await;
    assert!(matches!(fx.view(id).await.state, Some(AnchorState::Inferred { .. })));
    // The edit is reverted: the recorded anchor matches again, exactly.
    fx.settle(&[("a.txt", &dirty)]).await;

    assert_eq!(fx.view(id).await.state, Some(EXACT));
}

#[tokio::test]
async fn duplicated_content_is_ambiguous_until_one_is_attached() {
    let block = "dup A\ndup B\ndup C\n";
    let before = format!("p1\np2\np3\n{block}q1\nq2\nq3\n");
    let fx = Fixture::open(&[("a.txt", "x\n")], &[("a.txt", &before)]).await;
    let id = fx.thread(on("a.txt", 3, 6)).await;

    let after = format!("x1\nx2\nx3\n{block}x4\nx5\nx6\nx7\nx8\n{block}y1\ny2\ny3\n");
    let rev = fx.settle(&[("a.txt", &after)]).await;

    let view = fx.view(id).await;
    assert_eq!(view.location, None);
    assert_eq!(
        view.state,
        Some(AnchorState::Ambiguous {
            candidates: vec![at("a.txt", 3, 6).unwrap(), at("a.txt", 11, 14).unwrap()]
        })
    );
    let err = fx.session.attach_anchor(rev, id, 2).await.unwrap_err();
    assert!(matches!(err, SessionError::NotFound(_)));

    let attached = fx.session.attach_anchor(rev, id, 1).await.unwrap();
    assert_eq!((attached.state, attached.location), (Some(EXACT), at("a.txt", 11, 14)));
}

#[tokio::test]
async fn moved_content_in_another_changed_file_is_inferred() {
    let a = numbered(10, &[]);
    let b: String = (0..10).map(|i| format!("b {i}\n")).collect();
    let block = "m1\nm2\nm3\n";
    let with_block = format!(
        "{}{block}{}",
        &a[..a.find("line 5").unwrap()],
        &a[a.find("line 5").unwrap()..]
    );
    let fx = Fixture::open(
        &[("a.txt", &a), ("b.txt", &b)],
        &[("a.txt", &with_block), ("b.txt", &b)],
    )
    .await;
    let id = fx.thread(on("a.txt", 5, 8)).await;

    let moved = format!(
        "{}{block}{}",
        &b[..b.find("b 7").unwrap()],
        &b[b.find("b 7").unwrap()..]
    );
    fx.settle(&[("a.txt", &a), ("b.txt", &moved)]).await;

    let view = fx.view(id).await;
    assert!(
        matches!(
            view.state,
            Some(AnchorState::Inferred {
                strategy: Strategy::CrossFileMove,
                ..
            })
        ),
        "{:?}",
        view.state
    );
    assert_eq!((view.path, view.location), (Some(path("b.txt")), at("b.txt", 7, 10)));
}

#[tokio::test]
async fn deleting_the_file_detaches_range_and_file_threads_but_not_the_changeset_thread() {
    let fx = Fixture::open(&[("a.txt", &numbered(10, &[]))], &[("a.txt", &numbered(10, &[1]))]).await;
    let range = fx.thread(on("a.txt", 2, 5)).await;
    let file = fx
        .thread(NewThread::File {
            side: Side::Target,
            path: path("a.txt"),
        })
        .await;
    let whole = fx.thread(NewThread::Changeset).await;

    fx.settle(&[("other.txt", "o\n")]).await;

    let gone = Some(AnchorState::Stale {
        reason: StaleReason::FileDeleted,
    });
    let view = fx.view(range).await;
    assert_eq!((view.state, view.location), (gone.clone(), None));
    assert_eq!(fx.view(file).await.state, gone);
    let whole = fx.view(whole).await;
    assert_eq!((whole.state, whole.path, whole.location), (None, None, None));
}

#[tokio::test]
async fn range_and_file_threads_follow_a_rename() {
    let text = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &text)], &[("a.txt", &text)]).await;
    let range = fx.thread(on("a.txt", 2, 5)).await;
    let file = fx
        .thread(NewThread::File {
            side: Side::Target,
            path: path("a.txt"),
        })
        .await;
    fx.git.renames.lock().unwrap().push((path("a.txt"), path("b.txt")));

    fx.settle(&[("b.txt", &text)]).await;

    let view = fx.view(range).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("b.txt", 2, 5)));
    let view = fx.view(file).await;
    assert_eq!((view.state, view.path), (Some(EXACT), Some(path("b.txt"))));
}

impl Fixture {
    async fn comment_spot(&self, file: &str, at: CommentAt) -> CommentSpot {
        let revision = self.session.revision().await;
        self.session
            .comment_target(revision, View::Changeset, file, at)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_comment_on_a_hunk_targets_its_target_lines_or_for_a_pure_deletion_its_base_lines() {
    let base = numbered(12, &[]);
    // Line 3 is replaced by two lines.
    let replaced = base.replace("line 3\n", "new a\nnew b\n");
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &replaced)]).await;
    let spot = fx.comment_spot("a.txt", CommentAt::Hunk(0)).await;
    assert_eq!(spot.target, on("a.txt", 3, 5));
    // The form sits below the last line of the range.
    assert_eq!(spot.line, Some(4));
    let revision = fx.session.revision().await;
    let missing = fx
        .session
        .comment_target(revision, View::Changeset, "a.txt", CommentAt::Hunk(1))
        .await;
    assert!(matches!(missing, Err(SessionError::NotFound(_))));

    // Lines 7 and 8 are deleted: nothing is left to comment on in the target.
    let deleted = base.replace("line 7\nline 8\n", "");
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &deleted)]).await;
    let spot = fx.comment_spot("a.txt", CommentAt::Hunk(0)).await;
    assert_eq!(spot.target, on_base("a.txt", 7, 9));
    assert_eq!(spot.line, Some(7));
}

#[tokio::test]
async fn a_comment_on_a_file_targets_the_target_side_but_a_deleted_file_has_only_its_base() {
    let fx = Fixture::open(&[("a.txt", "one\n"), ("gone.txt", "x\ny\n")], &[("a.txt", "two\n")]).await;

    let spot = fx.comment_spot("a.txt", CommentAt::File).await;
    assert_eq!(
        (spot.target, spot.line),
        (
            NewThread::File {
                side: Side::Target,
                path: path("a.txt")
            },
            None
        )
    );
    let spot = fx.comment_spot("gone.txt", CommentAt::File).await;
    assert_eq!(
        spot.target,
        NewThread::File {
            side: Side::Base,
            path: path("gone.txt")
        }
    );
    // A deleted file's hunk is all deletion.
    let spot = fx.comment_spot("gone.txt", CommentAt::Hunk(0)).await;
    assert_eq!(spot.target, on_base("gone.txt", 0, 2));
}

#[tokio::test]
async fn a_comment_on_lines_targets_the_side_asked_for_and_is_checked() {
    let fx = Fixture::open(&[("a.txt", &numbered(10, &[]))], &[("a.txt", &numbered(10, &[4]))]).await;
    let lines = |side, start, len| CommentAt::Lines { side, start, len };

    let spot = fx.comment_spot("a.txt", lines(Side::Base, 2, 3)).await;
    assert_eq!(spot.target, on_base("a.txt", 2, 5));
    assert_eq!(
        fx.comment_spot("a.txt", lines(Side::Target, 2, 3)).await.target,
        on("a.txt", 2, 5)
    );

    let revision = fx.session.revision().await;
    let none = fx
        .session
        .comment_target(revision, View::Changeset, "a.txt", lines(Side::Target, 2, 0))
        .await;
    assert!(matches!(none, Err(SessionError::Invalid(_))));
    let stale = fx
        .session
        .comment_target(revision + 1, View::Changeset, "a.txt", CommentAt::File)
        .await;
    assert!(matches!(stale, Err(SessionError::Stale { .. })));
    let unknown = fx
        .session
        .comment_target(revision, View::Changeset, "nope.txt", CommentAt::File)
        .await;
    assert!(matches!(unknown, Err(SessionError::NotFound(_))));
}

#[test]
fn a_renamed_files_base_path_is_its_old_path() {
    let diff = |old: &str, new: &str| FileDiff {
        old_path: Some(path(old)),
        new_path: Some(path(new)),
        old_blob: None,
        new_blob: None,
        hunks: Vec::new(),
    };
    assert_eq!(
        old_path_of(&path("b.txt"), Some(&diff("a.txt", "b.txt")), &[]),
        Some(path("a.txt"))
    );
    assert_eq!(old_path_of(&path("a.txt"), Some(&diff("a.txt", "a.txt")), &[]), None);
}

#[tokio::test]
async fn replies_status_and_deletion_are_persisted() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    *fx.git.user.lock().unwrap() = Some("Ada".into());
    let id = fx.thread(on("a.txt", 0, 1)).await;
    let other = fx.thread(NewThread::Changeset).await;

    let view = fx.session.reply(id, "because".into(), 200).await.unwrap();
    assert_eq!(view.messages.len(), 2);
    assert_eq!(view.messages[1].at, 200);
    let view = fx.session.set_thread_status(id, ThreadStatus::Resolved).await.unwrap();
    assert_eq!(view.status, ThreadStatus::Resolved);
    fx.session.delete_thread(&FakeHosting::default(), other).await.unwrap();

    let stored = fx.store.0.lock().unwrap().threads_of(1);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].thread.status, ThreadStatus::Resolved);
    assert_eq!(
        stored[0]
            .thread
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        ["why?", "because"]
    );
    assert_eq!(fx.session.threads(View::Changeset).await.unwrap().threads.len(), 1);
    assert!(matches!(
        fx.session.reply(other, "x".into(), 1).await,
        Err(SessionError::NotFound(_))
    ));
    assert!(matches!(
        fx.session.reply(id, "  ".into(), 1).await,
        Err(SessionError::Invalid(_))
    ));
}

async fn thread_ids(session: &LiveSession<FakeGit, MemStore>) -> Vec<ThreadId> {
    session
        .threads(View::Changeset)
        .await
        .unwrap()
        .threads
        .iter()
        .map(|t| t.id)
        .collect()
}

#[tokio::test]
async fn finishing_a_review_keeps_its_open_threads_on_view_and_leaves_the_resolved_ones_in_it() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(30, &[3]))]).await;
    let anchored = fx.thread(on_base("a.txt", 10, 13)).await;
    let overall = fx.thread(NewThread::Changeset).await;
    let resolved = fx.thread(on("a.txt", 20, 22)).await;
    fx.session
        .set_thread_status(resolved, ThreadStatus::Resolved)
        .await
        .unwrap();
    assert_eq!(thread_ids(&fx.session).await, [anchored, overall, resolved]);

    fx.session.finish_review().await.unwrap();

    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
    // The base content is only in git, which may drop it now that the review holds no refs.
    fx.git.forget(&base);
    assert_eq!(thread_ids(&fx.session).await, [anchored, overall]);
    let view = fx.view(anchored).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("a.txt", 10, 13)));
    // Answering a thread is no review action.
    fx.session.reply(overall, "again".into(), 5).await.unwrap();
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
}

#[tokio::test]
async fn open_threads_of_a_finished_review_are_on_view_when_the_repository_is_opened_again_and_in_the_next_review() {
    let (base, dirty) = ("one\n", "two\n");
    let fx = Fixture::open(&[("a.txt", base)], &[("a.txt", dirty)]).await;
    let open = fx.thread(NewThread::Changeset).await;
    let resolved = fx.thread(NewThread::Changeset).await;
    fx.session
        .set_thread_status(resolved, ThreadStatus::Resolved)
        .await
        .unwrap();
    fx.session.finish_review().await.unwrap();

    let browsing = fx.reopen(&[("a.txt", dirty)]).await;

    assert_eq!(browsing.lifecycle().await, Lifecycle::Browsing);
    assert_eq!(thread_ids(&browsing).await, [open]);

    // The next review starts with it, and what is resolved in that one stays in it.
    browsing.set_seen(0, "a.txt", &[0], true).await.unwrap();
    let own = browsing
        .create_thread(0, View::Changeset, NewThread::Changeset, "again".into(), 1)
        .await
        .unwrap()
        .id;
    browsing.set_thread_status(own, ThreadStatus::Resolved).await.unwrap();
    assert_eq!(thread_ids(&browsing).await, [open, own]);
    let resumed = fx.reopen(&[("a.txt", dirty)]).await;
    assert_eq!(thread_ids(&resumed).await, [open, own]);

    resumed.finish_review().await.unwrap();
    assert_eq!(thread_ids(&resumed).await, [open]);
}

#[tokio::test]
async fn marking_records_the_version_last_reviewed_in() {
    let fx = Fixture::reviewing(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let last_reviewed = || {
        fx.store
            .0
            .lock()
            .unwrap()
            .reviews
            .last()
            .expect("the open review")
            .last_reviewed
    };
    assert_eq!(last_reviewed(), None);

    fx.settle(&[("a.txt", "three\n")]).await;
    fx.session.set_seen(1, "a.txt", &[0], true).await.unwrap();
    assert_eq!(last_reviewed(), Some(2));

    fx.settle(&[("a.txt", "four\n")]).await;
    assert_eq!(last_reviewed(), Some(2), "nothing was marked in version 3");
    fx.session.set_seen(2, "a.txt", &[0], false).await.unwrap();
    assert_eq!(last_reviewed(), Some(2), "taking a mark back marks nothing");
    fx.session.set_seen(2, "a.txt", &[0], true).await.unwrap();
    assert_eq!(last_reviewed(), Some(3));
}

#[tokio::test]
async fn a_session_is_named_after_its_repository_directory() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    assert_eq!(fx.session.manifest(View::Changeset).await.unwrap().name, "repo");
    assert_eq!(name_of_repo("/"), "/");
}

#[tokio::test]
async fn the_author_is_the_git_user_or_you() {
    let git = FakeGit::default();
    *git.base.lock().unwrap() = Some(git.snapshot(&[("a.txt", "one\n")]));
    let first = git.snapshot(&[("a.txt", "two\n")]);
    git.scans.lock().unwrap().push(first);
    *git.user.lock().unwrap() = Some("Ada".into());
    let named = LiveSession::open(git.clone(), MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();
    let anonymous = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;

    let view = named
        .create_thread(0, View::Changeset, NewThread::Changeset, "hi".into(), 1)
        .await
        .unwrap();
    assert_eq!(view.messages[0].author, "Ada");
    let view = anonymous
        .session
        .create_thread(0, View::Changeset, NewThread::Changeset, "hi".into(), 1)
        .await
        .unwrap();
    assert_eq!(view.messages[0].author, "You");
}

#[tokio::test]
async fn threads_need_text_inside_the_current_revision() {
    let fx = Fixture::open(&[("a.txt", "one\ntwo\n")], &[("a.txt", "one\ntwo\n"), ("bin", "x\0y")]).await;
    let new = |target| fx.session.create_thread(0, View::Changeset, target, "x".into(), 1);

    assert!(matches!(new(on("a.txt", 1, 3)).await, Err(SessionError::Invalid(_))));
    assert!(matches!(new(on("bin", 0, 1)).await, Err(SessionError::NotText(_))));
    assert!(matches!(new(on("nope", 0, 1)).await, Err(SessionError::NotFound(_))));
    fx.settle(&[("a.txt", "one\n")]).await;
    assert!(matches!(new(on("a.txt", 0, 1)).await, Err(SessionError::Stale { .. })));
    assert!(fx.store.0.lock().unwrap().threads_of(1).is_empty());
}

fn feature() -> Source {
    Source::Branch { name: "feature".into() }
}

/// Opens `feature` on `fx` and returns its revision.
async fn view_feature(fx: &Fixture) -> u32 {
    fx.session.open_changeset(feature()).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_branch_opened_as_a_changeset_shows_what_it_has_beyond_where_it_left_the_default_branch() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "dirty\n")]).await;
    fx.git
        .feature(&[("a.txt", "one\n")], &[("a.txt", "two\n"), ("new.txt", "n\n")]);

    let revision = view_feature(&fx).await;

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!((m.revision, m.lifecycle), (revision, Lifecycle::Browsing));
    assert_eq!(
        (
            m.target.source.clone(),
            m.target.name.as_str(),
            m.target.branch.as_deref()
        ),
        (feature(), "feature", Some("feature"))
    );
    assert_eq!(m.base.commit.id, CommitId("m1".into()));
    let files: Vec<_> = m.files.iter().map(|f| (f.path.to_string_lossy(), f.status)).collect();
    assert_eq!(
        files,
        [
            ("a.txt".to_string(), FileStatus::Modified),
            ("new.txt".to_string(), FileStatus::Added)
        ],
        "the branch's tip, not the dirty tree"
    );
    assert!(m.degraded.is_none());
}

#[tokio::test]
async fn a_new_commit_on_a_reviewed_branch_is_the_next_version_and_keeps_the_marks_that_still_hold() {
    let base = numbered(30, &[]);
    let (first, second) = (numbered(30, &[3]), numbered(30, &[3, 20]));
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &first)]);
    let revision = view_feature(&fx).await;
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 1, .. }
    ));
    assert_eq!(fx.session.observe().await.unwrap(), None, "nothing moved");

    fx.git.branch("feature", "tip2", &[("a.txt", &second)], &["tip1", "m1"]);
    let moved = fx.session.observe().await.unwrap().expect("the tip moved");

    assert!(moved > revision);
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 2, .. }
    ));
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Unseen]);
    let versions = fx.store.reviewed_versions();
    assert_eq!(
        versions
            .iter()
            .map(|v| (v.n, v.tip.0.as_str(), v.origin))
            .collect::<Vec<_>>(),
        [
            (1, "tip1", VersionOrigin::Settled),
            (2, "tip2", VersionOrigin::Observed)
        ]
    );
    assert_eq!(versions[0].base_commit, versions[1].base_commit);
    assert_eq!(fx.session.observe().await.unwrap(), None);
}

#[tokio::test]
async fn a_rebase_is_a_version_with_the_new_base() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(30, &[3]))]);
    let revision = view_feature(&fx).await;
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();

    // The default branch moved on, and the branch was rebased onto it: new base, new tip.
    let moved_base = numbered(30, &[25]);
    fx.git
        .add_commit("merge-base main tip2", "m3", &[("a.txt", &moved_base)]);
    fx.git
        .branch("feature", "tip2", &[("a.txt", &numbered(30, &[3, 25]))], &["m3"]);
    fx.session.observe().await.unwrap().expect("tip and base moved");

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert!(matches!(m.lifecycle, Lifecycle::Open { version: 2, .. }));
    assert_eq!(m.base.commit.id, CommitId("m3".into()));
    assert_eq!(
        m.files[0].review.hunks,
        [HunkState::Seen],
        "the mark is on the same lines"
    );
    let versions = fx.store.reviewed_versions();
    let (before, after) = (&versions[0], &versions[1]);
    assert_eq!(after.origin, VersionOrigin::Observed);
    assert_ne!(before.base_commit, after.base_commit);
    assert_ne!(before.snapshots.map(|s| s.base), after.snapshots.map(|s| s.base));
    assert_eq!(after.base_commit, CommitId("m3".into()));
}

/// The thread on lines 10..13 of `a.txt` of a branch with one edit at line 3, after the branch
/// moved to a commit with the tree `files`.
async fn thread_after_a_push(files: &[(&str, &str)]) -> (Fixture, ThreadId) {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(30, &[3]))]);
    view_feature(&fx).await;
    let id = fx.thread(on("a.txt", 10, 13)).await;
    fx.git.branch("feature", "tip2", files, &["tip1", "m1"]);
    fx.session.observe().await.unwrap().expect("the tip moved");
    (fx, id)
}

#[tokio::test]
async fn a_thread_on_a_branch_stays_put_when_a_push_edits_other_lines_or_rebases_the_branch() {
    let (fx, id) = thread_after_a_push(&[("a.txt", &numbered(30, &[3, 25]))]).await;
    let view = fx.view(id).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("a.txt", 10, 13)));

    // The default branch moved on, and the branch was rebased onto it.
    fx.git
        .add_commit("merge-base main tip3", "m3", &[("a.txt", &numbered(30, &[25]))]);
    fx.git
        .branch("feature", "tip3", &[("a.txt", &numbered(30, &[3, 25]))], &["m3"]);
    fx.session.observe().await.unwrap().expect("tip and base moved");

    let view = fx.view(id).await;
    assert_eq!((view.state, view.location), (Some(EXACT), at("a.txt", 10, 13)));
}

#[tokio::test]
async fn a_thread_on_a_branch_follows_its_hunk_when_a_push_rewrites_the_lines_around_it() {
    let (fx, id) = thread_after_a_push(&[("a.txt", &numbered(30, &[3, 8, 9, 10, 11, 12, 13, 14, 15]))]).await;

    let view = fx.view(id).await;

    assert!(
        matches!(view.state, Some(AnchorState::Inferred { .. })),
        "{:?}",
        view.state
    );
    assert_eq!(view.location, at("a.txt", 10, 13));
    assert!(view.target_line.is_some(), "it is shown, not only listed");
}

#[tokio::test]
async fn a_thread_on_a_branch_follows_its_hunk_however_large_the_push_rewrote_it() {
    let rewritten: String = ["line 0", "line 1", "line 2", "edited 3", "line 4"]
        .into_iter()
        .map(str::to_owned)
        .chain((0..40).map(|i| format!("rewritten {i}")))
        .chain((25..30).map(|i| format!("line {i}")))
        .map(|l| l + "\n")
        .collect();
    let (fx, id) = thread_after_a_push(&[("a.txt", &rewritten)]).await;

    let view = fx.view(id).await;

    assert_eq!(
        view.state,
        Some(AnchorState::Inferred {
            strategy: Strategy::ContextWindow,
            confidence: Confidence::Low
        })
    );
    assert_eq!(view.location, at("a.txt", 5, 45));
}

#[tokio::test]
async fn a_thread_on_lines_a_push_deleted_stays_where_they_were_marked_deleted() {
    let without: String = numbered(30, &[3])
        .lines()
        .enumerate()
        .filter(|(i, _)| !(10..13).contains(i))
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    let (fx, id) = thread_after_a_push(&[("a.txt", &without)]).await;

    let view = fx.view(id).await;

    assert_eq!(
        view.state,
        Some(AnchorState::Inferred {
            strategy: Strategy::Deleted,
            confidence: Confidence::Low
        })
    );
    assert_eq!(view.location, at("a.txt", 10, 11));
    assert!(view.target_line.is_some(), "it is shown, not only listed");
}

#[tokio::test]
async fn a_thread_on_a_file_the_branch_added_follows_a_later_commit_that_renamed_it() {
    let base = numbered(30, &[]);
    let added = numbered(30, &[]);
    let fx = Fixture::open(&[("keep.txt", &base)], &[("keep.txt", &base)]).await;
    fx.git
        .feature(&[("keep.txt", &base)], &[("keep.txt", &base), ("new.txt", &added)]);
    view_feature(&fx).await;
    let id = fx.thread(on("new.txt", 10, 13)).await;
    fx.git.branch(
        "feature",
        "tip2",
        &[("keep.txt", &base), ("renamed.txt", &added)],
        &["tip1", "m1"],
    );
    fx.session.observe().await.unwrap().expect("the tip moved");

    let view = fx.view(id).await;

    assert_eq!(
        (view.path, view.location),
        (Some(path("renamed.txt")), at("renamed.txt", 10, 13))
    );
}

#[tokio::test]
async fn a_moved_branch_that_is_only_browsed_follows_the_tip_and_stores_nothing() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let revision = view_feature(&fx).await;

    fx.git
        .branch("feature", "tip2", &[("a.txt", "three\n")], &["tip1", "m1"]);
    let moved = fx.session.observe().await.unwrap().expect("the tip moved");

    assert!(moved > revision);
    assert_eq!(fx.session.lifecycle().await, Lifecycle::Browsing);
    assert_eq!(
        fx.view.semantic_sides(moved).await.unwrap().head,
        CommitId("tip2".into())
    );
    assert!(fx.store.reviewed_versions().is_empty());
}

#[tokio::test]
async fn switching_away_from_a_changeset_and_back_keeps_its_review_and_revisions_only_increase() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "branch\n")]);
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);

    let on_branch = view_feature(&fx).await;
    assert_eq!(
        fx.session.lifecycle().await,
        Lifecycle::Browsing,
        "the branch has its own review"
    );
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Unseen]);
    // The tree settling while it is off view is not the revision on view.
    fx.settle(&[("a.txt", "two\n")]).await;
    assert_eq!(fx.session.revision().await, on_branch);

    let back = fx.session.open_changeset(Source::WorkingTree).await.unwrap().unwrap();

    assert!(back > on_branch && on_branch > 0);
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 2, .. }
    ));
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen]);
    assert_eq!(fx.session.source().await, Source::WorkingTree);
    assert_eq!(
        fx.session.open_changeset(Source::WorkingTree).await.unwrap(),
        None,
        "it is on view already"
    );
}

#[tokio::test]
async fn the_working_tree_is_found_when_it_comes_on_view_while_the_lookup_waits_for_the_token() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "branch\n")]);
    view_feature(&fx).await;
    let mut held = fx.session.hold().await;

    // The lookup starts first, sees the branch on view and waits; then the tree is switched back.
    let looking = fx.session.working_tree();
    let switching = async {
        let tree = held.parked().remove(&Source::WorkingTree).unwrap();
        let branch = held.replace(tree);
        held.parked().insert(branch.source.clone(), branch);
        drop(held);
    };
    let (tree, ()) = tokio::join!(looking, switching);

    assert_eq!(tree.source, Source::WorkingTree);
}

#[tokio::test]
async fn a_review_of_a_branch_is_resumed_when_the_branch_is_opened_in_a_new_session() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let revision = view_feature(&fx).await;
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();

    let tree = fx.git.snapshot(&[("a.txt", "one\n")]);
    fx.git.scans.lock().unwrap().push(tree);
    let again = LiveSession::open(fx.git.clone(), fx.store.clone(), REPO.into(), clock)
        .await
        .unwrap();
    again.open_changeset(feature()).await.unwrap();

    assert!(matches!(again.lifecycle().await, Lifecycle::Open { version: 1, .. }));
    let m = again.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.files[0].review.hunks, [HunkState::Seen]);
}

#[tokio::test]
async fn opening_a_branch_that_does_not_exist_changes_nothing() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let before = fx.session.revision().await;

    assert!(matches!(
        fx.session.open_changeset(feature()).await,
        Err(SessionError::NotFound(_))
    ));

    assert_eq!(
        (fx.session.source().await, fx.session.revision().await),
        (Source::WorkingTree, before)
    );
}

#[tokio::test]
async fn the_list_has_the_working_tree_first_then_every_branch_the_most_recent_first() {
    const DAY: i64 = 24 * 60 * 60;
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    fx.git.add_branch("merged", false, "m2", &["m1"]);
    fx.git.add_branch("stale", false, "s1", &["m1"]);
    fx.git.add_branch("older", false, "o1", &["m1"]);
    for (commit, age_days) in [("tip1", 1), ("s1", 40), ("o1", 10)] {
        fx.git
            .times
            .lock()
            .unwrap()
            .insert(CommitId(commit.into()), -age_days * DAY);
    }

    let list = fx.session.changesets().await.unwrap();

    let titles: Vec<_> = list.rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Working tree", "merged", "feature", "older", "stale"],
        "branches with nothing of their own or long untouched are listed too, the default branch is not; the most recent first"
    );
    assert_eq!(list.on_view, Source::WorkingTree);
    let feature = &list.rows[2];
    assert_eq!(feature.divergence, Some(Divergence { ahead: 1, behind: 1 }));
    assert_eq!(feature.last_commit, Some(-DAY));
    assert_eq!(
        (list.rows[0].author.as_deref(), feature.author.as_deref()),
        (None, Some("Ann")),
        "a branch has its tip's author, the working tree none"
    );
}

#[tokio::test]
async fn the_default_branch_is_listed_while_it_has_commits_its_upstream_lacks() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    let upstream_at = |tip: &str, history: &[&str]| {
        fx.git.add_branch("origin/main", false, tip, history);
        for b in fx
            .git
            .branches
            .lock()
            .unwrap()
            .iter_mut()
            .filter(|b| b.name == "origin/main")
        {
            b.remote = true;
        }
    };
    let main = |list: ChangesetList| list.rows.into_iter().find(|r| r.title == "main").map(|r| r.unpushed);

    assert_eq!(
        main(fx.session.changesets().await.unwrap()),
        None,
        "no upstream: nothing to push"
    );

    upstream_at("o1", &["m1"]);
    assert_eq!(main(fx.session.changesets().await.unwrap()), Some(Some(1)));

    upstream_at("m2", &["m1"]);
    assert_eq!(main(fx.session.changesets().await.unwrap()), None, "all pushed");

    fx.git.add_commit("refs/heads/main", "m2", &[("a.txt", "one\n")]);
    fx.session
        .open_changeset(Source::Branch { name: "main".into() })
        .await
        .unwrap();
    assert_eq!(
        main(fx.session.changesets().await.unwrap()),
        Some(Some(0)),
        "kept while on view"
    );
}

#[tokio::test]
async fn a_branch_that_does_not_exist_is_not_found_and_one_git_cannot_read_is_a_git_failure() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    let open = |name: &str| fx.session.open_changeset(Source::Branch { name: name.into() });

    assert!(matches!(open("nope").await, Err(SessionError::NotFound(_))));

    fx.git.tips_fail.store(true, Ordering::Relaxed);
    assert!(matches!(open("nope").await, Err(SessionError::Git(_))));
}

#[tokio::test]
async fn the_list_says_which_changesets_have_a_review_and_keeps_the_one_on_view() {
    const DAY: i64 = 24 * 60 * 60;
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    // Old, but on view.
    fx.git.times.lock().unwrap().insert(CommitId("tip1".into()), -90 * DAY);
    let revision = view_feature(&fx).await;
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();

    let open = fx.session.changesets().await.unwrap();
    assert_eq!(open.on_view, feature());
    let review_of = |list: &ChangesetList, title: &str| list.rows.iter().find(|r| r.title == title).unwrap().review;
    assert_eq!(review_of(&open, "Working tree"), RowReview::None);
    let RowReview::Open {
        version,
        started_at,
        last_reviewed,
        ..
    } = review_of(&open, "feature")
    else {
        panic!("the review is open")
    };
    assert_eq!((version, last_reviewed), (1, Some(1)));

    fx.session.finish_review().await.unwrap();
    let finished = fx.session.changesets().await.unwrap();
    let RowReview::Finished {
        complete,
        started_at: finished_started,
        finished_at,
        last_reviewed,
    } = review_of(&finished, "feature")
    else {
        panic!("the review is finished")
    };
    assert_eq!((complete, finished_started, last_reviewed), (true, started_at, Some(1)));
    assert!(finished_at >= started_at);
}

#[tokio::test]
async fn the_list_names_the_branch_of_the_working_tree_and_counts_the_open_threads_of_each_changeset() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);

    let before = fx.session.changesets().await.unwrap();
    assert_eq!(before.rows[0].branch.as_deref(), Some("main"));
    assert!(before.rows.iter().all(|r| r.open_threads == 0));

    fx.session
        .create_thread(0, View::Changeset, NewThread::Changeset, "why".into(), 1)
        .await
        .unwrap();
    let after = fx.session.changesets().await.unwrap();
    let count_of = |title: &str| after.rows.iter().find(|r| r.title == title).unwrap().open_threads;
    assert_eq!((count_of("Working tree"), count_of("feature")), (1, 0));
    assert!(after.rows.iter().skip(1).all(|r| r.branch.is_none()));
}

pub(crate) fn pull_request_7() -> Source {
    Source::PullRequest {
        remote: RemoteKey {
            host: "github.com".into(),
            path: "acme/app".into(),
        },
        number: 7,
    }
}

/// Pull request 7 as fetched: its head at commit `head` with the tree `files`, which left the
/// target branch at commit `m1` with the tree `base`.
pub(crate) fn fetch_pull_request_7(git: &FakeGit, base: &[(&str, &str)], head: &str, files: &[(&str, &str)]) {
    let Source::PullRequest { remote, number } = pull_request_7() else {
        unreachable!()
    };
    let refs = PullRequestRefs::of(&remote, number);
    git.add_commit(&refs.head, head, files);
    git.add_commit(&format!("merge-base {}", refs.target), "m1", base);
}

/// Pull request 7 was rebased onto the target branch as it is at commit `id` with the tree `files`.
pub(crate) fn move_base_of_pull_request_7(git: &FakeGit, id: &str, files: &[(&str, &str)]) {
    let Source::PullRequest { remote, number } = pull_request_7() else {
        unreachable!()
    };
    git.add_commit(
        &format!("merge-base {}", PullRequestRefs::of(&remote, number).target),
        id,
        files,
    );
}

#[tokio::test]
async fn a_pull_request_opened_as_a_changeset_shows_what_its_head_has_beyond_the_target_branch() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "dirty\n")]).await;
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", "one\n")],
        "pr1",
        &[("a.txt", "two\n"), ("new.txt", "n\n")],
    );

    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!((m.revision, m.lifecycle), (revision, Lifecycle::Browsing));
    assert_eq!(
        (
            m.target.source.clone(),
            m.target.name.as_str(),
            m.target.branch.as_deref()
        ),
        (pull_request_7(), "#7", None)
    );
    assert_eq!(m.base.commit.id, CommitId("m1".into()));
    let files: Vec<_> = m.files.iter().map(|f| (f.path.to_string_lossy(), f.status)).collect();
    assert_eq!(
        files,
        [
            ("a.txt".to_string(), FileStatus::Modified),
            ("new.txt".to_string(), FileStatus::Added)
        ],
        "the head, not the dirty tree"
    );
    assert_eq!(fx.session.source().await, pull_request_7());
}

#[tokio::test]
async fn a_new_push_to_a_pull_request_is_the_next_version_and_a_force_push_too() {
    let base = numbered(30, &[]);
    let (first, second, rewritten) = (numbered(30, &[3]), numbered(30, &[3, 20]), numbered(30, &[20]));
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr1", &[("a.txt", &first)]);
    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();
    assert_eq!(fx.session.observe().await.unwrap(), None, "nothing was pushed");

    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr2", &[("a.txt", &second)]);
    fx.session.observe().await.unwrap().expect("pushed to");
    assert_eq!(fx.hunks("a.txt").await, [HunkState::Seen, HunkState::Unseen]);

    // The old head is not an ancestor of the new one.
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr3", &[("a.txt", &rewritten)]);
    fx.session.observe().await.unwrap().expect("force-pushed");

    let versions = fx.store.reviewed_versions();
    assert_eq!(
        versions
            .iter()
            .map(|v| (v.n, v.tip.0.as_str(), v.base_commit.0.as_str(), v.origin))
            .collect::<Vec<_>>(),
        [
            (1, "pr1", "m1", VersionOrigin::Settled),
            (2, "pr2", "m1", VersionOrigin::Observed),
            (3, "pr3", "m1", VersionOrigin::Observed)
        ]
    );
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 3, .. }
    ));
}

#[tokio::test]
async fn a_target_branch_that_moved_under_a_pull_request_moves_its_base() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(&fx.git, &[("a.txt", &base)], "pr1", &[("a.txt", &numbered(30, &[3]))]);
    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();

    // The author rebased onto a target branch that moved on.
    fx.git.add_commit(
        "merge-base refs/vet/pr/github_com/acme/app/7/target pr2",
        "m3",
        &[("a.txt", &numbered(30, &[25]))],
    );
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", &base)],
        "pr2",
        &[("a.txt", &numbered(30, &[3, 25]))],
    );
    fx.session.observe().await.unwrap().expect("head and base moved");

    let m = fx.session.manifest(View::Changeset).await.unwrap();
    assert_eq!(m.base.commit.id, CommitId("m3".into()));
    assert_eq!(m.files[0].review.hunks, [HunkState::Seen]);
}

#[tokio::test]
async fn a_pull_request_that_was_not_fetched_cannot_be_opened_and_changes_nothing() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "two\n")]).await;
    let before = fx.session.revision().await;

    let error = fx.session.open_changeset(pull_request_7()).await.unwrap_err();

    assert!(matches!(error, SessionError::NotFound(_)), "{error}");
    assert_eq!(
        (fx.session.source().await, fx.session.revision().await),
        (Source::WorkingTree, before)
    );
}

#[tokio::test]
async fn the_review_state_of_pull_requests_is_listed_for_the_sources_asked() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fetch_pull_request_7(&fx.git, &[("a.txt", "one\n")], "pr1", &[("a.txt", "two\n")]);
    let other = Source::PullRequest {
        remote: RemoteKey {
            host: "github.com".into(),
            path: "acme/app".into(),
        },
        number: 8,
    };
    let revision = fx.session.open_changeset(pull_request_7()).await.unwrap().unwrap();
    fx.session.set_seen(revision, "a.txt", &[0], true).await.unwrap();

    let reviews = fx.session.reviews_of(&[other, pull_request_7()]).await.unwrap();

    assert_eq!(reviews[0], RowReview::None);
    assert!(matches!(reviews[1], RowReview::Open { version: 1, .. }), "{reviews:?}");
}

/// How far the working tree's review is, as the list says it.
async fn tree_progress(session: &LiveSession<FakeGit, MemStore>) -> Option<ReviewSummary> {
    match session.changesets().await.unwrap().rows[0].review {
        RowReview::Open { progress, .. } => progress,
        other => panic!("the review is open: {other:?}"),
    }
}

fn summary(total: u32, seen: u32, old_base: u32) -> ReviewSummary {
    ReviewSummary { total, seen, old_base }
}

#[tokio::test]
async fn the_list_says_how_far_an_open_review_is_and_follows_its_marks_and_its_filter() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3, 5]))]).await;
    assert_eq!(fx.session.changesets().await.unwrap().rows[0].review, RowReview::None);

    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(
        tree_progress(&fx.session).await,
        Some(summary(4, 2, 0)),
        "a hunk of two"
    );

    fx.session.set_seen(0, "a.txt", &[1], true).await.unwrap();
    assert_eq!(tree_progress(&fx.session).await, Some(summary(4, 4, 0)));

    fx.session.set_seen(0, "a.txt", &[0], false).await.unwrap();
    assert_eq!(
        tree_progress(&fx.session).await,
        Some(summary(4, 2, 0)),
        "a mark taken back"
    );

    fx.session.exclude_path("a.txt", false).await.unwrap();
    assert_eq!(
        tree_progress(&fx.session).await,
        Some(summary(0, 0, 0)),
        "progress is measured against the filter"
    );
}

#[tokio::test]
async fn the_progress_of_a_review_counts_the_non_text_items_it_acknowledged() {
    let git = FakeGit::default();
    let base = git.snapshot(&[("run.sh", "echo\n")]);
    let mut dirty = base.clone();
    dirty.entries.get_mut(&path("run.sh")).unwrap().mode = 0o100755;
    dirty.id = merkle::snapshot_id(&dirty.entries);
    *git.base.lock().unwrap() = Some(base);
    git.scans.lock().unwrap().push(dirty);
    let session = LiveSession::open(git, MemStore::default(), REPO.into(), clock)
        .await
        .unwrap();

    session.acknowledge(0, "run.sh", NonTextKind::Mode, true).await.unwrap();
    assert_eq!(tree_progress(&session).await, Some(summary(1, 1, 0)));

    session
        .acknowledge(0, "run.sh", NonTextKind::Mode, false)
        .await
        .unwrap();
    assert_eq!(tree_progress(&session).await, Some(summary(1, 0, 0)));
}

#[tokio::test]
async fn a_review_whose_base_moved_has_its_base_changed_lines_in_the_progress() {
    let fx = seen_then_base_changed().await;
    let m = fx.session.manifest(View::Changeset).await.unwrap();

    let RowReview::Open { progress, .. } = fx.session.reviews_of(&[pull_request_7()]).await.unwrap()[0] else {
        panic!("the review is open")
    };
    let progress = progress.expect("measured");

    assert_eq!(
        (progress.total, progress.seen, progress.old_base),
        (
            m.completion.lines + m.completion.items,
            m.completion.seen_lines + m.completion.acknowledged_items,
            m.completion.old_base_lines
        )
    );
    assert!(progress.old_base > 0, "the marked hunk was seen against the old base");
    assert!(progress.seen_percent() < 100.0 && progress.old_base_percent() > 0.0);
}

#[tokio::test]
async fn the_progress_of_a_working_tree_off_view_follows_the_tree_as_it_settles() {
    let base = numbered(10, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[3]))]).await;
    fx.git.feature(&[("a.txt", &base)], &[("a.txt", &numbered(10, &[1]))]);
    fx.session.set_seen(0, "a.txt", &[0], true).await.unwrap();
    assert_eq!(tree_progress(&fx.session).await, Some(summary(2, 2, 0)));
    view_feature(&fx).await;

    // Nobody looks at the tree while the branch is on view.
    fx.settle(&[("a.txt", &numbered(10, &[3, 7]))]).await;

    assert_eq!(
        tree_progress(&fx.session).await,
        Some(summary(4, 2, 0)),
        "the new hunk is not seen"
    );
}

#[tokio::test]
async fn the_lines_of_a_branch_are_counted_between_its_base_and_its_tip_once_per_pair_of_commits() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fx.git.feature(
        &[("a.txt", &base)],
        &[("a.txt", &numbered(30, &[3, 20])), ("new.txt", "n\n")],
    );

    let lines = fx.session.lines_of(&feature()).await.unwrap();

    assert_eq!(lines, Some(LineDelta { added: 3, removed: 2 }));
    assert_eq!(fx.session.lines_of(&feature()).await.unwrap(), lines);
    assert_eq!(fx.git.diffs.load(Ordering::Relaxed), 1, "the pair was diffed once");

    // The branch moved: another tip is another pair.
    fx.git
        .branch("feature", "tip2", &[("a.txt", &numbered(30, &[3]))], &["tip1", "m1"]);
    assert_eq!(
        fx.session.lines_of(&feature()).await.unwrap(),
        Some(LineDelta { added: 1, removed: 1 })
    );
    assert_eq!(fx.git.diffs.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn a_pull_request_has_lines_once_it_was_fetched_and_the_working_tree_has_none() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "dirty\n")]).await;
    assert_eq!(
        fx.session.lines_of(&pull_request_7()).await.unwrap(),
        None,
        "not fetched"
    );
    assert_eq!(fx.session.lines_of(&Source::WorkingTree).await.unwrap(), None);

    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", "one\n")],
        "pr1",
        &[("a.txt", "two\n"), ("new.txt", "n\n")],
    );

    assert_eq!(
        fx.session.lines_of(&pull_request_7()).await.unwrap(),
        Some(LineDelta { added: 2, removed: 1 })
    );
}

#[tokio::test]
async fn a_pull_request_pushed_to_and_fetched_again_is_counted_anew_and_not_from_the_old_head() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "dirty\n")]).await;
    fetch_pull_request_7(&fx.git, &[("a.txt", "one\n")], "pr1", &[("a.txt", "two\n")]);
    assert_eq!(
        fx.session.lines_of(&pull_request_7()).await.unwrap(),
        Some(LineDelta { added: 1, removed: 1 })
    );

    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", "one\n")],
        "pr2",
        &[("a.txt", "two\n"), ("new.txt", "n\nm\n")],
    );

    assert_eq!(
        fx.session.lines_of(&pull_request_7()).await.unwrap(),
        Some(LineDelta { added: 3, removed: 1 })
    );
}

mod ahead;
mod derived;
mod marks;
mod publish;
mod pushes;
mod sync;
