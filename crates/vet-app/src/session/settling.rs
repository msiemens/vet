//! Settling the working tree's snapshots into the session and following the tree as it changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use vet_core::content::{Snapshot, SnapshotId, SnapshotStatus};

use super::derived::Diffed;
use super::state::State;
use super::{LiveSession, Result};
use crate::acquisition::BlobError;
use crate::base;
use crate::changeset::{Snapshots, Source, Version, VersionChange, VersionOrigin};
use crate::mutex::lock;
use crate::ports::{Clock, Degraded, GitRepo, LiveStore, ReviewWrites, Scan, Watcher};
use crate::settle::{Settled, Settling};

/// What settling a snapshot into a tree needs that takes a while, worked out for the tree as it
/// was found, without the token.
struct Settlement {
    base: SnapshotId,
    target: SnapshotId,
    /// A review of the tree was open, so the settled snapshot is its next version.
    reviewing: bool,
    /// Why the content of the version could not be kept, if it could not.
    retained: Option<BlobError>,
    /// What the version changed against the one before it.
    change: Option<VersionChange>,
    /// The diff of the base against the settled snapshot, if it could be had.
    diffed: Option<Arc<Diffed>>,
}

impl Settlement {
    /// Whether `tree` is still the one this was worked out for.
    fn fits(&self, tree: &State) -> bool {
        let reviewing = tree.review.is_some() && tree.changeset.is_some();
        tree.base.id == self.base && tree.target.id == self.target && reviewing == self.reviewing
    }
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Looks at the working tree's `HEAD` again and keeps whether it still builds on the base.
    /// Answers whether that changed. Does nothing for another changeset.
    pub(super) async fn track_head(&self, st: &mut State) -> bool {
        if st.source != Source::WorkingTree {
            return false;
        }
        let off_base = base::head_left(&self.acquisition.git, &st.base_commit).await;
        std::mem::replace(&mut st.off_base, off_base) != off_base
    }

    fn set_degraded(&self, degraded: Option<Degraded>) {
        *lock(&self.degraded) = degraded;
    }

    /// Why the working tree is polled instead of watched, if it is; none for any other source.
    pub(super) fn degraded_for(&self, source: &Source) -> Option<Degraded> {
        let degraded = *lock(&self.degraded);
        degraded.filter(|_| *source == Source::WorkingTree)
    }

    /// Makes a settled snapshot the target and returns the new revision. While a
    /// review is open it is also the next version: its content is stored and
    /// the changeset gains a version. The revision comes with its diff and the review state
    /// remapped onto it, which are worked out before the token is taken. A failed copy of its
    /// content is not fatal: the diff then reads the worktree, or shows the file as unavailable.
    /// It is the working tree's, wherever that is: on view, or parked while a branch is.
    pub async fn settled(&self, scan: Scan) -> Result<(u32, Option<BlobError>)> {
        let Scan { snapshot, status } = scan;
        let snapshot = Arc::new(snapshot);
        loop {
            let tree = self.working_tree().await;
            let settlement = self.settlement(&tree, &snapshot, &status).await?;
            let mut held = self.hold().await;
            if held.source == Source::WorkingTree {
                if !settlement.fits(&held) {
                    continue;
                }
                let writes = held.writes();
                let settled = self
                    .settle_tree(&writes, &mut held, snapshot, status, settlement)
                    .await?;
                held.publish();
                self.revisions.advance(settled.0);
                return Ok(settled);
            }
            let writes = held.writes();
            let tree = held
                .parked()
                .tree_mut()
                .expect("the working tree is parked while another changeset is on view");
            if !settlement.fits(tree) {
                continue;
            }
            return self
                .settle_tree(&writes, Arc::make_mut(tree), snapshot, status, settlement)
                .await;
        }
    }

    /// The working tree's state: the one on view, or the one parked while another changeset is.
    pub(super) async fn working_tree(&self) -> Arc<State> {
        let st = self.published.read();
        if st.source == Source::WorkingTree {
            return st;
        }
        let mut held = self.hold().await;
        // It may have come on view while this waited for the token.
        if held.source == Source::WorkingTree {
            return held.shared();
        }
        let tree = held.parked().tree();
        Arc::clone(tree.expect("the working tree is parked while another changeset is on view"))
    }

    /// What settling `snapshot` into `tree` needs that takes a while: the content kept for its
    /// version and what it changed, while a review is open, and the diff the revision is shown
    /// with. A diff that cannot be had is left for the first request of the revision to try again.
    async fn settlement(&self, tree: &State, snapshot: &Arc<Snapshot>, status: &SnapshotStatus) -> Result<Settlement> {
        let mut settlement = Settlement {
            base: tree.base.id,
            target: tree.target.id,
            reviewing: tree.review.is_some() && tree.changeset.is_some(),
            retained: None,
            change: None,
            diffed: None,
        };
        if settlement.reviewing {
            let retained = self
                .acquisition
                .retain(
                    &[&*tree.base, &*tree.target],
                    snapshot,
                    status,
                    &BTreeMap::new(),
                    |_, _| {},
                )
                .await;
            settlement.retained = retained.err();
            let sides = (&tree.base, &tree.target);
            settlement.change = Some(self.change_of(sides, &tree.base, snapshot).await?);
        }
        settlement.diffed = self.facts.diff_of(&tree.base, snapshot).await.ok();
        Ok(settlement)
    }

    /// Settles `snapshot` into `st`, which need not be the one on view: only the revision of
    /// the one on view is announced, by the caller.
    async fn settle_tree(
        &self,
        store: &S::Writes,
        st: &mut State,
        snapshot: Arc<Snapshot>,
        status: SnapshotStatus,
        settlement: Settlement,
    ) -> Result<(u32, Option<BlobError>)> {
        // The agent commits on top of the base: the snapshot is built on the `HEAD` of now.
        let mut head = st.head.clone();
        if st.source == Source::WorkingTree
            && let Ok(now) = self.acquisition.git.head().await
        {
            head = now;
        }
        if let (Some(review), Some(changeset)) = (&st.review, st.changeset) {
            let version = Version {
                n: review.version + 1,
                snapshots: Some(Snapshots {
                    base: st.base.id,
                    target: snapshot.id,
                }),
                base_commit: st.base_commit.clone(),
                tip: head.clone(),
                at: (self.clock)(),
                origin: VersionOrigin::Settled,
                change: settlement.change.expect("worked out while the review was open"),
            };
            let previous = Some((&st.base_commit, &st.head));
            let write = store.append_version(changeset, version.clone());
            self.record_version(store, changeset, &version, previous, &[(&snapshot, &status)], write)
                .await?;
        }
        st.head = head;
        // Older versions stay materializable (§6.2).
        self.acquisition.record_commit(snapshot.id, st.head.clone());
        let old = std::mem::replace(&mut st.target, snapshot);
        st.target_status = status;
        if let Some(review) = &mut st.review {
            st.known.insert(old.id, old);
            review.version += 1;
            review.updated_at = (self.clock)();
        }
        st.revision += 1;
        st.snapshot += 1;
        st.changes = None;
        self.track_head(st).await;
        if let Some(diffed) = settlement.diffed
            && let Err(e) = self.install_diff(store, st, diffed).await
        {
            tracing::warn!(error = %e, "could not remap the review onto the settled tree");
            st.changes = None;
        }
        Ok((st.revision, settlement.retained))
    }

    /// Settles snapshots of the worktree as `settling` reports them, and looks at the branch
    /// on view when a ref was written, calling `on_settled` with each new revision of the
    /// changeset on view and the open review's latest version after it, until dropped. A settled
    /// tree that is not on view is not announced: it is not what is shown. `on_quiet` is called
    /// when `settling` finds the settled tree quiet, `on_edits` when edits start or stop waiting
    /// to settle.
    pub async fn follow<W: Watcher + Send, C: Clock + Send + Sync>(
        &self,
        mut settling: Settling<W, C>,
        mut on_settled: impl FnMut(u32, Option<u32>) + Send,
        mut on_quiet: impl FnMut() + Send,
        mut on_edits: impl FnMut(bool) + Send,
    ) {
        self.set_degraded(settling.degraded());
        loop {
            settling.set_busy(false);
            let settled = settling.next_settled(&self.acquisition).await;
            // Handling what settled is work too, even when it is not a scan: a client waiting for
            // quiet must not answer between the result and the event it announces.
            settling.set_busy(true);
            let scan = match settled {
                Ok(Settled::Tree(scan)) => scan,
                Ok(Settled::Quiet) => {
                    on_quiet();
                    continue;
                }
                Ok(Settled::Edits(waiting)) => {
                    on_edits(waiting);
                    continue;
                }
                Ok(Settled::RefsMoved) => {
                    match self.observe().await {
                        Ok(Some(revision)) => on_settled(revision, self.review_version()),
                        Ok(None) => {}
                        Err(e) => tracing::warn!(error = %e, "could not look at the branch on view"),
                    }
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "reconciliation failed");
                    continue;
                }
            };
            match self.settled(scan).await {
                Ok((revision, retained)) => {
                    if let Some(e) = retained {
                        tracing::warn!(revision, error = %e, "could not keep the content of the revision");
                    }
                    self.set_degraded(settling.degraded());
                    if self.published.read().source == Source::WorkingTree {
                        on_settled(revision, self.review_version());
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not record a settled snapshot"),
            }
        }
    }
}
