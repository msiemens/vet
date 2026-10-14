//! What the store keeps alive (§8). The store reads the facts and carries out
//! the plan; what is live is decided here.

use std::collections::HashSet;
use std::time::Duration;

use vet_core::content::SnapshotId;

use crate::ports::{ChangesetStore, GcFacts, GcPlan, GcReport, GcStore, StoreError};

#[derive(Clone, Copy, Debug)]
pub struct Retention {
    /// How long a changeset with no open review is kept after its last review
    /// closed (§8).
    pub closed_changesets: Duration,
    /// Content and snapshots younger than this are never collected, so a blob
    /// written just before the metadata that references it survives a GC.
    pub grace: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            closed_changesets: Duration::from_secs(30 * 24 * 3600),
            grace: Duration::from_secs(3600),
        }
    }
}

/// A changeset expires once its last review closed longer ago than the
/// retention, unless a review of it is open or a thread of it is; snapshots
/// live while a changeset that is kept refers to them, once they are past the
/// grace period.
pub fn plan(facts: &GcFacts, now: i64, retention: &Retention) -> GcPlan {
    let expired_before = now - retention.closed_changesets.as_secs() as i64;
    let (dead, kept): (Vec<_>, Vec<_>) = facts.changesets.iter().partition(|c| {
        !c.has_open_review
            && !c.has_open_thread
            && !c.has_pending_vote
            && c.last_closed_at.is_some_and(|at| at <= expired_before)
    });

    let live: HashSet<SnapshotId> = kept.iter().flat_map(|c| c.snapshots.iter().copied()).collect();
    let young_after = now - retention.grace.as_secs() as i64;
    GcPlan {
        changesets: dead.iter().map(|c| c.id).collect(),
        snapshots: facts
            .snapshots
            .iter()
            .filter(|(id, created)| *created <= young_after && !live.contains(id))
            .map(|&(id, _)| id)
            .collect(),
        content_grace: retention.grace,
    }
}

/// The bases materialized trees are kept for: those of the latest versions of the
/// changesets a review of which is open.
pub async fn open_bases<S: ChangesetStore>(store: &S) -> Result<HashSet<SnapshotId>, StoreError> {
    let mut bases = HashSet::new();
    for changeset in store.open_changesets().await? {
        if let Some(snapshots) = store.versions(changeset).await?.last().and_then(|v| v.snapshots) {
            bases.insert(snapshots.base);
        }
    }
    Ok(bases)
}

/// Deletes expired changesets, then the snapshots, tree nodes and content nothing
/// retained refers to. `now` is unix seconds.
pub async fn collect<S: GcStore>(store: &S, now: i64, retention: &Retention) -> Result<GcReport, StoreError> {
    store.gc(now, &|facts| plan(facts, now, retention)).await
}

#[cfg(test)]
mod tests {
    use vet_core::content::Hash;

    use super::*;
    use crate::changeset::ChangesetId;
    use crate::ports::ChangesetFacts;

    const DAY: i64 = 24 * 3600;
    const NOW: i64 = 100 * DAY;

    fn snap(n: u8) -> SnapshotId {
        SnapshotId(Hash([n; 32]))
    }

    fn changeset(id: u64, last_closed_at: Option<i64>, snapshots: &[u8]) -> ChangesetFacts {
        ChangesetFacts {
            id: ChangesetId(id),
            has_open_review: last_closed_at.is_none(),
            last_closed_at,
            has_open_thread: false,
            has_pending_vote: false,
            snapshots: snapshots.iter().map(|&n| snap(n)).collect(),
        }
    }

    fn facts(changesets: Vec<ChangesetFacts>) -> GcFacts {
        GcFacts {
            changesets,
            snapshots: (1..=11).map(|n| (snap(n), NOW - 2 * DAY)).collect(),
        }
    }

    #[test]
    fn a_changeset_expires_after_its_last_review_closed_and_an_open_review_or_thread_keeps_it() {
        let retention = Retention::default();
        let closed = |ago: i64| changeset(1, Some(NOW - ago), &[1]);
        let expired = plan(&facts(vec![closed(31 * DAY)]), NOW, &retention);
        assert_eq!(expired.changesets, vec![ChangesetId(1)]);
        let recent = plan(&facts(vec![closed(29 * DAY)]), NOW, &retention);
        assert!(recent.changesets.is_empty());
        let open = plan(&facts(vec![changeset(1, None, &[1])]), NOW, &retention);
        assert!(open.changesets.is_empty());

        let mut reopened = closed(31 * DAY);
        reopened.has_open_review = true;
        assert!(plan(&facts(vec![reopened]), NOW, &retention).changesets.is_empty());

        let mut with_thread = closed(31 * DAY);
        with_thread.has_open_thread = true;
        assert!(plan(&facts(vec![with_thread]), NOW, &retention).changesets.is_empty());

        let mut with_vote = closed(31 * DAY);
        with_vote.has_pending_vote = true;
        assert!(plan(&facts(vec![with_vote]), NOW, &retention).changesets.is_empty());
    }

    #[test]
    fn a_snapshot_lives_while_a_changeset_that_is_kept_refers_to_it() {
        let f = facts(vec![
            changeset(1, None, &[1, 2]),
            changeset(2, Some(NOW - 31 * DAY), &[3]),
        ]);
        let dead: HashSet<_> = plan(&f, NOW, &Retention::default()).snapshots.into_iter().collect();
        // 1 and 2 belong to the open review; 3 to the expired changeset.
        let expected: HashSet<_> = [3, 4, 5, 6, 7, 8, 9, 10, 11].map(snap).into_iter().collect();
        assert_eq!(dead, expected);
    }

    #[test]
    fn snapshots_younger_than_the_grace_period_are_never_collected() {
        let mut f = facts(vec![]);
        f.snapshots = vec![(snap(1), NOW - 3600 + 1), (snap(2), NOW - 3600)];
        let plan = plan(&f, NOW, &Retention::default());
        assert_eq!(plan.snapshots, vec![snap(2)]);
        assert_eq!(plan.content_grace, Duration::from_secs(3600));
    }
}
