//! Execution and trust (§7.5, §9, §10): run targets, the run check, launching
//! in place, and the analysis policy. Executing restores and debug or test runs
//! go through [`Execution::check`] and [`Execution::approve`] the same way
//! [`Execution::launch`] does, before a pool or a process is asked.

pub mod check;
pub mod targets;
pub mod trust;

use std::collections::BTreeMap;
use std::path::PathBuf;

use vet_core::content::{Hash, Snapshot, SnapshotDiff, SnapshotId, SnapshotStatus};

use crate::acquisition::Acquisition;
use crate::ports::{Command, Dirty, GitError, GitRepo, ProcessRunner, RunStore, Store, StoreError};
use check::{Approval, RunCheck, RunCheckConfig};
use targets::RunTarget;

#[derive(Debug)]
pub enum RunError {
    Git(GitError),
    Store(StoreError),
    Spawn(String),
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
    pub async fn targets(&self, snapshot: &Snapshot) -> Result<Vec<RunTarget>, RunError> {
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
        Ok(targets::merge(targets::discover(snapshot, &blobs), stored))
    }

    /// Saves an edited or user-added target.
    pub async fn save_target(&self, target: RunTarget) -> Result<(), RunError> {
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
            .put_snapshot(approval.current.clone(), SnapshotStatus::default())
            .await?;
        store
            .record_run_approval(self.repo.clone(), approval.hash, approval.current.id, now)
            .await?;
        Ok(())
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
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath};
    use vet_core::merkle::snapshot_id;

    use super::*;
    use crate::pin::CommitId;
    use crate::ports::{CheckoutError, GcReport, ProcessEvent, RunningProcess, Scan};
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
        async fn set_session_ref(&self, _: SessionId, _: &str, _: &CommitId) -> Result<(), GitError> {
            unreachable!()
        }
        async fn delete_session_refs(&self, _: SessionId) -> Result<(), GitError> {
            unreachable!()
        }
        async fn diff<S: Store + Sync>(&self, _: &S, _: &Snapshot, _: &Snapshot) -> Result<SnapshotDiff, GitError> {
            unreachable!()
        }
    }

    impl Store for Fake {
        async fn put_blob(&self, bytes: Vec<u8>) -> Result<BlobId, StoreError> {
            let id = BlobId::of(&bytes);
            self.0.lock().unwrap().blobs.insert(id, bytes);
            Ok(id)
        }
        async fn blob(&self, _: BlobId) -> Result<Option<Vec<u8>>, StoreError> {
            unreachable!()
        }
        async fn put_snapshot(&self, s: Snapshot, _: SnapshotStatus) -> Result<(), StoreError> {
            self.0.lock().unwrap().snapshots.insert(s.id, s);
            Ok(())
        }
        async fn snapshot(&self, id: SnapshotId) -> Result<Option<(Snapshot, SnapshotStatus)>, StoreError> {
            let s = self.0.lock().unwrap().snapshots.get(&id).cloned();
            Ok(s.map(|s| (s, SnapshotStatus::default())))
        }
        async fn append_timeline(&self, _: SessionId, _: SnapshotId) -> Result<(), StoreError> {
            unreachable!()
        }
        async fn timeline(&self, _: SessionId) -> Result<Vec<SnapshotId>, StoreError> {
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
        let target = run(exec.targets(&reviewed)).unwrap().remove(0);
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

    #[test]
    fn counts_changed_paths() {
        let (a, _) = snapshot(&[("a", "1"), ("b", "1"), ("c", "1")]);
        let (b, _) = snapshot(&[("a", "1"), ("b", "2"), ("d", "1")]);
        assert_eq!(changed_files(&a, &a), 0);
        assert_eq!(changed_files(&a, &b), 3);
    }
}
