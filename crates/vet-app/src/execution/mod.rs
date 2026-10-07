//! Execution and trust (§7.5, §9, §10): run targets, the run check, launching
//! in place, and the analysis policy. Executing restores and debug or test runs
//! go through [`Execution::check`] and [`Execution::approve`] the same way
//! [`Execution::launch`] does, before a pool or a process is asked.

pub mod check;
pub mod command;
pub mod rows;
pub mod targets;
pub mod trust;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, PathBuf};

use vet_core::content::{
    BlobId, Hash, LineDelta, NonTextKind, RepoPath, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus,
};

use crate::acquisition::Acquisition;
use crate::ports::{BlobRead, Command, Dirty, GitError, GitRepo, ProcessRunner, RunStore, Store, StoreError};
use check::{Approval, Change, RunCheck, RunCheckConfig};
use rows::DiffRows;
use targets::RunTarget;

#[derive(Debug)]
pub enum RunError {
    Git(GitError),
    Store(StoreError),
    Spawn(String),
    /// The request does not apply; the message is fit to show.
    Invalid(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(e) => write!(f, "git: {}", e.0),
            Self::Store(e) => write!(f, "store: {}", e.0),
            Self::Spawn(message) | Self::Invalid(message) => f.write_str(message),
        }
    }
}

/// A run target and whether the user edited or added it (it is in the store).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListedTarget {
    pub target: RunTarget,
    pub stored: bool,
}

/// One executor-relevant file of an approval prompt, with what changed in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ApprovalFile {
    pub path: RepoPath,
    /// The old path, if git detected a rename; the removed file is not listed then.
    pub from: Option<RepoPath>,
    pub change: Change,
    /// The changed lines with their context; `None` if there is no text to compare (binary,
    /// over the capture cap).
    pub rows: Option<DiffRows>,
    pub delta: LineDelta,
}

impl From<GitError> for RunError {
    fn from(e: GitError) -> Self {
        Self::Git(e)
    }
}

impl From<StoreError> for RunError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

pub struct LaunchRequest<'a> {
    pub target: &'a RunTarget,
    /// The target snapshot the user last reviewed.
    pub reviewed: &'a Snapshot,
    /// The hash from the [`Approval`] the user just confirmed. Launching
    /// proceeds only if the files about to run still hash to it.
    pub approved: Option<Hash>,
    /// Unix seconds.
    pub now: i64,
}

pub enum LaunchOutcome<P> {
    /// Nothing was started; show the changes and ask "run anyway?".
    NeedsApproval(Approval),
    Started(Started<P>),
}

pub struct Started<P> {
    /// What is running: the reconciled dirty tree at launch.
    pub snapshot: SnapshotId,
    /// Files that differ between the reviewed snapshot and `snapshot`; the run
    /// panel says "running snapshot X — N files changed since review" if not zero.
    pub changed_since_review: usize,
    pub process: P,
}

pub struct Execution<'a, G, S, P> {
    pub acquisition: &'a Acquisition<G, S>,
    pub runner: &'a P,
    /// Identifies the repository in the store.
    pub repo: String,
    /// The worktree root; a dirty tree runs in place.
    pub root: PathBuf,
    pub config: RunCheckConfig,
}

impl<G: GitRepo + Sync, S: Store + RunStore + Sync, P: ProcessRunner> Execution<'_, G, S, P> {
    /// Discovered targets of `snapshot` with the user's edits applied.
    pub async fn targets(&self, snapshot: &Snapshot) -> Result<Vec<ListedTarget>, RunError> {
        let mut blobs = BTreeMap::new();
        for (path, entry) in &snapshot.entries {
            let name = path.0.rsplit(|b| *b == b'/').next().unwrap_or_default();
            if matches!(name, b"Cargo.toml" | b"package.json" | b"pyproject.toml")
                && let Some(bytes) = self.acquisition.git.read_blob(path, entry.blob).await?
            {
                blobs.insert(entry.blob, bytes);
            }
        }
        let stored = self.acquisition.store.run_targets(self.repo.clone()).await?;
        let stored_ids: Vec<String> = stored.iter().map(|t| t.id.clone()).collect();
        let merged = targets::merge(targets::discover(snapshot, &blobs), stored);
        Ok(merged
            .into_iter()
            .map(|target| ListedTarget {
                stored: stored_ids.contains(&target.id),
                target,
            })
            .collect())
    }

    /// Saves an edited or user-added target. Ids of added targets are the
    /// caller's to choose.
    pub async fn save_target(&self, target: RunTarget) -> Result<(), RunError> {
        let invalid = |message: &str| Err(RunError::Invalid(message.into()));
        if target.id.trim().is_empty() {
            return invalid("a run target needs an id");
        }
        if target.program.trim().is_empty() {
            return invalid(command::NO_COMMAND);
        }
        let relative = std::path::Path::new(&target.cwd)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        if !relative {
            return invalid("the working directory must be inside the repository");
        }
        Ok(self.acquisition.store.put_run_target(self.repo.clone(), target).await?)
    }

    /// Removes the stored target: an edited discovered target reverts, a
    /// user-added one disappears.
    pub async fn reset_target(&self, id: String) -> Result<(), RunError> {
        Ok(self.acquisition.store.delete_run_target(self.repo.clone(), id).await?)
    }

    /// The run check for `snapshot` (§7.5).
    pub async fn check(&self, snapshot: &Snapshot) -> Result<RunCheck, RunError> {
        let store = &self.acquisition.store;
        let current = self.config.relevant(snapshot);
        let last = store.last_run_approval(self.repo.clone()).await?;
        let baseline = match last {
            Some((hash, id)) if hash != current.id.0 => store.snapshot(id).await?.map(|(s, _)| s),
            _ => None,
        };
        Ok(check::decide(current, last.map(|(hash, _)| hash), baseline))
    }

    /// The text changes behind an approval prompt, from the existing diff path.
    pub async fn diff(&self, approval: &Approval) -> Result<SnapshotDiff, RunError> {
        let diff = self
            .acquisition
            .git
            .diff(&self.acquisition.store, &approval.baseline, &approval.current)
            .await?;
        Ok(diff)
    }

    /// The files behind an approval prompt with their text, for display.
    pub async fn approval_files(&self, approval: &Approval) -> Result<Vec<ApprovalFile>, RunError> {
        let diff = self.diff(approval).await?;
        let is = |path: &RepoPath, change| approval.changed.iter().any(|c| &c.path == path && c.change == change);
        // New path to old path of the renames whose two sides are both in the prompt.
        let renamed: BTreeMap<&RepoPath, &RepoPath> = diff
            .renames
            .0
            .iter()
            .filter(|(old, new)| is(old, Change::Removed) && is(new, Change::Added))
            .map(|(old, new)| (new, old))
            .collect();
        let mut files = Vec::new();
        for changed in &approval.changed {
            if renamed.values().any(|old| **old == changed.path) {
                continue;
            }
            let found = diff
                .files
                .iter()
                .find(|d| d.new_path.as_ref().or(d.old_path.as_ref()) == Some(&changed.path));
            let unchanged_rename = || {
                let item = diff
                    .non_text
                    .iter()
                    .find(|n| n.kind == NonTextKind::Rename && n.path == changed.path);
                item.map(|n| (n.before.as_ref().map(|e| e.blob), n.after.as_ref().map(|e| e.blob)))
            };
            let (old_blob, new_blob, hunks) = match found {
                Some(d) => (d.old_blob, d.new_blob, d.hunks.clone()),
                None => {
                    let (old, new) = unchanged_rename().unwrap_or_default();
                    (old, new, Vec::new())
                }
            };
            let from = renamed.get(&changed.path).map(|old| (*old).clone());
            let (old, new) = (
                self.text(from.as_ref().unwrap_or(&changed.path), old_blob).await?,
                self.text(&changed.path, new_blob).await?,
            );
            files.push(ApprovalFile {
                path: changed.path.clone(),
                from,
                change: changed.change,
                rows: rows::diff_rows(old.as_deref(), new.as_deref(), changed.change, &hunks),
                delta: LineDelta::of(&hunks),
            });
        }
        Ok(files)
    }

    /// UTF-8 content, from the store, else git or the worktree; nothing over the capture cap.
    async fn text(&self, path: &RepoPath, blob: Option<BlobId>) -> Result<Option<String>, RunError> {
        let Some(blob) = blob else { return Ok(None) };
        let bytes = match self.acquisition.store.blob(blob).await? {
            Some(bytes) => bytes,
            None => match self.acquisition.git.read_blob_capped(path, blob).await? {
                BlobRead::Found(bytes) => bytes,
                BlobRead::TooLarge | BlobRead::Missing => return Ok(None),
            },
        };
        Ok(String::from_utf8(bytes).ok())
    }

    /// Records the approval and keeps the approved files' content, so later
    /// prompts can diff against it.
    pub async fn approve(&self, approval: &Approval, now: i64) -> Result<(), RunError> {
        let store = &self.acquisition.store;
        for (path, entry) in &approval.current.entries {
            if let Some(bytes) = self.acquisition.git.read_blob(path, entry.blob).await? {
                store.put_blob(bytes).await?;
            }
        }
        store
            .put_snapshot(approval.current.clone(), SnapshotStatus::of(&approval.current), now)
            .await?;
        store
            .record_run_approval(self.repo.clone(), approval.hash, approval.current.id, now)
            .await?;
        Ok(())
    }

    /// [`Self::launch`] for the target with this id among those of `reviewed`.
    pub async fn run(
        &self,
        target_id: &str,
        reviewed: &Snapshot,
        approved: Option<Hash>,
        now: i64,
    ) -> Result<LaunchOutcome<P::Process>, RunError> {
        let target = (self.targets(reviewed).await?.into_iter())
            .find(|t| t.target.id == target_id)
            .ok_or_else(|| RunError::Invalid(format!("no run target {target_id}")))?
            .target;
        self.launch(LaunchRequest {
            target: &target,
            reviewed,
            approved,
            now,
        })
        .await
    }

    /// Reconcile, run check, start the process in place (§9).
    pub async fn launch(&self, request: LaunchRequest<'_>) -> Result<LaunchOutcome<P::Process>, RunError> {
        let snapshot = self.acquisition.reconcile(Dirty::All).await?.snapshot;
        if let RunCheck::NeedsApproval(approval) = self.check(&snapshot).await? {
            if request.approved != Some(approval.hash) {
                return Ok(LaunchOutcome::NeedsApproval(approval));
            }
            self.approve(&approval, request.now).await?;
        }
        let target = request.target;
        let command = Command {
            program: target.program.clone(),
            args: target.args.clone(),
            cwd: self.root.join(&target.cwd),
            env: target.env.clone(),
        };
        let process = self.runner.spawn(&command).map_err(RunError::Spawn)?;
        Ok(LaunchOutcome::Started(Started {
            snapshot: snapshot.id,
            changed_since_review: changed_files(request.reviewed, &snapshot),
            process,
        }))
    }
}

/// Number of paths whose entry differs between `a` and `b`.
pub fn changed_files(a: &Snapshot, b: &Snapshot) -> usize {
    if a.id == b.id {
        return 0;
    }
    let differing = a.entries.iter().filter(|(p, e)| b.entries.get(*p) != Some(e)).count();
    let added = b.entries.keys().filter(|p| !a.entries.contains_key(*p)).count();
    differing + added
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    use vet_core::content::{BlobId, EntryKind, FileDiff, FileEntry, LineRange, RepoPath};
    use vet_core::merkle::snapshot_id;

    use vet_core::content::Hunk;

    use super::rows::Row;
    use super::*;
    use crate::pin::CommitId;
    use crate::ports::{BlobRead, CheckoutError, GcReport, ProcessEvent, RunningProcess, Scan};
    use crate::review::SessionId;

    fn snapshot(files: &[(&str, &str)]) -> (Snapshot, HashMap<BlobId, Vec<u8>>) {
        let mut entries = BTreeMap::new();
        let mut blobs = HashMap::new();
        for (path, content) in files {
            let blob = BlobId::of(content.as_bytes());
            blobs.insert(blob, content.as_bytes().to_vec());
            let entry = FileEntry {
                blob,
                mode: 0o100644,
                kind: EntryKind::File,
                unfiltered: false,
            };
            entries.insert(RepoPath(path.as_bytes().to_vec()), entry);
        }
        let id = snapshot_id(&entries);
        (Snapshot { id, entries }, blobs)
    }

    #[derive(Default)]
    struct World {
        /// What the worktree currently holds.
        tree: Option<Snapshot>,
        blobs: HashMap<BlobId, Vec<u8>>,
        snapshots: HashMap<SnapshotId, Snapshot>,
        targets: Vec<RunTarget>,
        approval: Option<(Hash, SnapshotId)>,
        spawned: Vec<(String, Vec<String>, PathBuf)>,
    }

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<World>>);

    impl Fake {
        fn set_tree(&self, files: &[(&str, &str)]) {
            let (snapshot, blobs) = snapshot(files);
            let mut w = self.0.lock().unwrap();
            w.blobs.extend(blobs);
            w.tree = Some(snapshot);
        }
    }

    impl GitRepo for Fake {
        async fn head(&self) -> Result<CommitId, GitError> {
            unreachable!()
        }
        async fn user_name(&self) -> Option<String> {
            None
        }
        async fn scan(&self, _: Dirty) -> Result<Scan, GitError> {
            let snapshot = self.0.lock().unwrap().tree.clone().unwrap();
            Ok(Scan {
                snapshot,
                status: Default::default(),
            })
        }
        async fn checkout<S: Store + Sync>(
            &self,
            _: &S,
            _: &CommitId,
            _: &Snapshot,
            _: &Path,
        ) -> Result<(), CheckoutError> {
            unreachable!()
        }
        async fn commit_snapshot(&self, _: &CommitId) -> Result<Snapshot, GitError> {
            unreachable!()
        }
        async fn read_blob(&self, _: &RepoPath, blob: BlobId) -> Result<Option<Vec<u8>>, GitError> {
            Ok(self.0.lock().unwrap().blobs.get(&blob).cloned())
        }
        async fn read_blob_capped(&self, _: &RepoPath, blob: BlobId) -> Result<BlobRead, GitError> {
            Ok(match self.0.lock().unwrap().blobs.get(&blob) {
                Some(bytes) => BlobRead::Found(bytes.clone()),
                None => BlobRead::Missing,
            })
        }
        async fn submodule_commit(&self, _: BlobId) -> Result<Option<String>, GitError> {
            unreachable!()
        }
        async fn merge_base(&self, _: &str) -> Result<CommitId, GitError> {
            unreachable!()
        }
        async fn resolve_commit(&self, _: &str) -> Result<CommitId, GitError> {
            unreachable!()
        }
        async fn base_candidates(&self, _: usize) -> Result<crate::ports::BaseCandidates, GitError> {
            unreachable!()
        }
        async fn branches(&self) -> Result<Vec<crate::ports::BranchInfo>, GitError> {
            unreachable!()
        }
        async fn commit_info(&self, _: &CommitId) -> Result<crate::ports::CommitInfo, GitError> {
            unreachable!()
        }
        async fn first_parent_distance(&self, _: &CommitId, _: &CommitId, _: usize) -> Result<Option<usize>, GitError> {
            unreachable!()
        }
        async fn commits_behind(&self, _: &CommitId, _: &CommitId, _: usize) -> Result<usize, GitError> {
            unreachable!()
        }
        async fn set_session_ref(&self, _: SessionId, _: &str, _: &CommitId) -> Result<(), GitError> {
            unreachable!()
        }
        async fn delete_session_refs(&self, _: SessionId) -> Result<(), GitError> {
            unreachable!()
        }
        /// One hunk over the whole file for every path whose blob differs.
        async fn diff<S: Store + Sync>(&self, _: &S, old: &Snapshot, new: &Snapshot) -> Result<SnapshotDiff, GitError> {
            let w = self.0.lock().unwrap();
            let lines = |e: Option<&FileEntry>| {
                e.map_or(0, |e| w.blobs[&e.blob].split_inclusive(|&b| b == b'\n').count() as u32)
            };
            let mut out = SnapshotDiff::default();
            for path in old.entries.keys().chain(new.entries.keys()).collect::<BTreeSet<_>>() {
                let (o, n) = (old.entries.get(path), new.entries.get(path));
                if o == n {
                    continue;
                }
                out.files.push(FileDiff {
                    old_path: o.map(|_| path.clone()),
                    new_path: n.map(|_| path.clone()),
                    old_blob: o.map(|e| e.blob),
                    new_blob: n.map(|e| e.blob),
                    hunks: vec![Hunk {
                        old: LineRange {
                            start: 0,
                            end: lines(o),
                        },
                        new: LineRange {
                            start: 0,
                            end: lines(n),
                        },
                    }],
                });
            }
            Ok(out)
        }
    }

    impl Store for Fake {
        async fn put_blob(&self, bytes: Vec<u8>) -> Result<BlobId, StoreError> {
            let id = BlobId::of(&bytes);
            self.0.lock().unwrap().blobs.insert(id, bytes);
            Ok(id)
        }
        async fn blob(&self, id: BlobId) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self.0.lock().unwrap().blobs.get(&id).cloned())
        }
        async fn put_snapshot(&self, s: Snapshot, _: SnapshotStatus, _: i64) -> Result<(), StoreError> {
            self.0.lock().unwrap().snapshots.insert(s.id, s);
            Ok(())
        }
        async fn snapshot(&self, id: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
            let s = self.0.lock().unwrap().snapshots.get(&id).cloned();
            Ok(s.map(|s| (s, SnapshotStatus::default())))
        }
        async fn append_timeline(&self, _: SessionId, _: SnapshotId, _: i64) -> Result<(), StoreError> {
            unreachable!()
        }
        async fn timeline(&self, _: SessionId) -> Result<Vec<crate::review::Version>, StoreError> {
            unreachable!()
        }
        async fn gc(&self, _: i64) -> Result<GcReport, StoreError> {
            unreachable!()
        }
    }

    impl RunStore for Fake {
        async fn run_targets(&self, _: String) -> Result<Vec<RunTarget>, StoreError> {
            Ok(self.0.lock().unwrap().targets.clone())
        }
        async fn put_run_target(&self, _: String, target: RunTarget) -> Result<(), StoreError> {
            self.0.lock().unwrap().targets.push(target);
            Ok(())
        }
        async fn delete_run_target(&self, _: String, _: String) -> Result<(), StoreError> {
            unreachable!()
        }
        async fn record_run_approval(
            &self,
            _: String,
            hash: Hash,
            baseline: SnapshotId,
            _: i64,
        ) -> Result<(), StoreError> {
            self.0.lock().unwrap().approval = Some((hash, baseline));
            Ok(())
        }
        async fn last_run_approval(&self, _: String) -> Result<Option<(Hash, SnapshotId)>, StoreError> {
            Ok(self.0.lock().unwrap().approval)
        }
    }

    struct Exited;

    impl RunningProcess for Exited {
        async fn next(&mut self) -> ProcessEvent {
            ProcessEvent::Exit(Some(0))
        }
        fn kill(&self) {}
    }

    impl ProcessRunner for Fake {
        type Process = Exited;
        fn spawn(&self, c: &Command) -> Result<Exited, String> {
            self.0
                .lock()
                .unwrap()
                .spawned
                .push((c.program.clone(), c.args.clone(), c.cwd.clone()));
            Ok(Exited)
        }
    }

    fn run<T>(future: impl Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("fakes never block"),
        }
    }

    const PACKAGE: &str = r#"{"scripts":{"dev":"vite"}}"#;

    #[test]
    fn launch_asks_once_then_runs_until_a_build_file_changes() {
        let fake = Fake::default();
        let acquisition = Acquisition::new(fake.clone(), fake.clone());
        let exec = Execution {
            acquisition: &acquisition,
            runner: &fake,
            repo: "/repo".into(),
            root: PathBuf::from("/repo"),
            config: RunCheckConfig::default(),
        };
        fake.set_tree(&[("ui/package.json", PACKAGE), ("ui/src/a.ts", "1")]);
        let reviewed = fake.0.lock().unwrap().tree.clone().unwrap();
        let listed = run(exec.targets(&reviewed)).unwrap().remove(0);
        assert!(!listed.stored);
        let target = listed.target;
        assert_eq!(target.id, "script:ui:dev");
        let request = |approved| LaunchRequest {
            target: &target,
            reviewed: &reviewed,
            approved,
            now: 1,
        };

        // First run ever: prompt, nothing spawned.
        let LaunchOutcome::NeedsApproval(first) = run(exec.launch(request(None))).unwrap() else {
            panic!("expected a prompt");
        };
        assert!(first.first_run);
        assert!(fake.0.lock().unwrap().spawned.is_empty());

        // A stale approval hash does not count.
        assert!(matches!(
            run(exec.launch(request(Some(Hash([9; 32]))))).unwrap(),
            LaunchOutcome::NeedsApproval(_)
        ));

        let LaunchOutcome::Started(started) = run(exec.launch(request(Some(first.hash)))).unwrap() else {
            panic!("approved run must start");
        };
        assert_eq!(started.changed_since_review, 0);
        assert_eq!(
            fake.0.lock().unwrap().spawned,
            [(
                "npm".to_string(),
                vec!["run".to_string(), "dev".to_string()],
                PathBuf::from("/repo/ui")
            )]
        );

        // A source edit does not prompt and is reported as drift from the review.
        fake.set_tree(&[("ui/package.json", PACKAGE), ("ui/src/a.ts", "2"), ("ui/src/b.ts", "")]);
        let LaunchOutcome::Started(started) = run(exec.launch(request(None))).unwrap() else {
            panic!("no build file changed");
        };
        assert_eq!(started.changed_since_review, 2);

        // An edited script prompts, with the change against the approved version.
        fake.set_tree(&[("ui/package.json", r#"{"scripts":{"dev":"rm -rf ~"}}"#)]);
        let LaunchOutcome::NeedsApproval(second) = run(exec.launch(request(None))).unwrap() else {
            panic!("package.json changed");
        };
        assert!(!second.first_run);
        assert_eq!(second.changed.len(), 1);
        assert_eq!(second.baseline.entries.len(), 1);
    }

    fn execution<'a>(fake: &'a Fake, acquisition: &'a Acquisition<Fake, Fake>) -> Execution<'a, Fake, Fake, Fake> {
        Execution {
            acquisition,
            runner: fake,
            repo: "/repo".into(),
            root: PathBuf::from("/repo"),
            config: RunCheckConfig::default(),
        }
    }

    #[test]
    fn a_run_by_id_prompts_with_the_text_of_what_changed() {
        let fake = Fake::default();
        let acquisition = Acquisition::new(fake.clone(), fake.clone());
        let exec = execution(&fake, &acquisition);
        fake.set_tree(&[("package.json", PACKAGE), ("src/a.ts", "1")]);
        let reviewed = fake.0.lock().unwrap().tree.clone().unwrap();

        let Ok(LaunchOutcome::NeedsApproval(first)) = run(exec.run("script::dev", &reviewed, None, 1)) else {
            panic!("expected a prompt");
        };
        let files = run(exec.approval_files(&first)).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, RepoPath(b"package.json".to_vec()));
        assert_eq!(files[0].change, Change::Added);
        let added = Row::Added {
            number: 1,
            text: PACKAGE.into(),
        };
        assert_eq!(files[0].rows.as_ref().map(|r| r.rows.clone()), Some(vec![added]));

        assert!(matches!(
            run(exec.run("script::dev", &reviewed, Some(first.hash), 1)),
            Ok(LaunchOutcome::Started(_))
        ));
        let edited = r#"{"scripts":{"dev":"vite --open"}}"#;
        fake.set_tree(&[("package.json", edited), ("src/a.ts", "1")]);
        let Ok(LaunchOutcome::NeedsApproval(second)) = run(exec.run("script::dev", &reviewed, None, 2)) else {
            panic!("package.json changed");
        };
        let files = run(exec.approval_files(&second)).unwrap();
        let removed = Row::Removed {
            number: 1,
            text: PACKAGE.into(),
        };
        let added = Row::Added {
            number: 1,
            text: edited.into(),
        };
        assert_eq!(
            files[0].rows.as_ref().map(|r| r.rows.clone()),
            Some(vec![removed, added])
        );
        assert_eq!(files[0].delta, LineDelta { added: 1, removed: 1 });
        assert_eq!(files[0].change, Change::Modified);

        let missing = run(exec.run("nope", &reviewed, None, 1));
        assert_eq!(missing.err().unwrap().to_string(), "no run target nope");
    }

    #[test]
    fn saved_targets_are_listed_as_stored_and_must_make_sense() {
        let fake = Fake::default();
        let acquisition = Acquisition::new(fake.clone(), fake.clone());
        let exec = execution(&fake, &acquisition);
        fake.set_tree(&[("package.json", PACKAGE)]);
        let reviewed = fake.0.lock().unwrap().tree.clone().unwrap();
        let target = |id: &str, program: &str, cwd: &str| RunTarget {
            id: id.into(),
            source: None,
            label: "mine".into(),
            program: program.into(),
            args: vec![],
            cwd: cwd.into(),
            env: vec![],
        };

        for bad in [
            target("", "make", ""),
            target("t", " ", ""),
            target("t", "make", "../x"),
            target("t", "make", "/x"),
        ] {
            assert!(matches!(run(exec.save_target(bad)), Err(RunError::Invalid(_))));
        }
        run(exec.save_target(target("custom:1", "make", "sub"))).unwrap();

        let listed = run(exec.targets(&reviewed)).unwrap();
        let flags: Vec<_> = listed.iter().map(|t| (t.target.id.as_str(), t.stored)).collect();
        assert_eq!(flags, [("script::dev", false), ("custom:1", true)]);
    }

    #[test]
    fn counts_changed_paths() {
        let (a, _) = snapshot(&[("a", "1"), ("b", "1"), ("c", "1")]);
        let (b, _) = snapshot(&[("a", "1"), ("b", "2"), ("d", "1")]);
        assert_eq!(changed_files(&a, &a), 0);
        assert_eq!(changed_files(&a, &b), 3);
    }
}
