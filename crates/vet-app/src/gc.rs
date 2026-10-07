//! What the store keeps alive (§8). The store reads the facts and carries out
//! the plan; what is live is decided here.

use std::collections::HashSet;
use std::time::Duration;

use vet_core::content::SnapshotId;

use crate::pin::PinId;
use crate::ports::{GcReport, GcStore, StoreError};
use crate::review::SessionId;

#[derive(Clone, Copy, Debug)]
pub struct Retention {
    /// How long a closed session is kept (§8).
    pub closed_sessions: Duration,
    /// Content and snapshots younger than this are never collected, so a blob
    /// written just before the metadata that references it survives a GC.
    pub grace: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            closed_sessions: Duration::from_secs(30 * 24 * 3600),
            grace: Duration::from_secs(3600),
        }
    }
}

/// A session and everything it keeps alive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionFacts {
    pub id: SessionId,
    pub pin: PinId,
    /// Unix seconds; `None` while open.
    pub closed_at: Option<i64>,
    pub has_open_thread: bool,
    /// Its base and target, its timeline and the snapshots its anchors name.
    pub snapshots: Vec<SnapshotId>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GcFacts {
    pub sessions: Vec<SessionFacts>,
    /// Every stored pin and the snapshot of its tree.
    pub pins: Vec<(PinId, SnapshotId)>,
    /// Every stored snapshot and when it was stored (unix seconds).
    pub snapshots: Vec<(SnapshotId, i64)>,
}

/// What to delete. Tree nodes and content follow from what remains.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GcPlan {
    pub sessions: Vec<SessionId>,
    pub pins: Vec<PinId>,
    pub snapshots: Vec<SnapshotId>,
    /// Unreferenced content younger than this stays.
    pub content_grace: Duration,
}

/// Closed sessions expire after the retention unless an open thread keeps them;
/// pins live while a session holds them; snapshots live while a session or a
/// pin refers to them, once they are past the grace period.
pub fn plan(facts: &GcFacts, now: i64, retention: &Retention) -> GcPlan {
    let expired_before = now - retention.closed_sessions.as_secs() as i64;
    let (dead, kept): (Vec<_>, Vec<_>) = facts
        .sessions
        .iter()
        .partition(|s| s.closed_at.is_some_and(|at| at <= expired_before) && !s.has_open_thread);

    let held: HashSet<PinId> = kept.iter().map(|s| s.pin).collect();
    let (live_pins, dead_pins): (Vec<_>, Vec<_>) = facts.pins.iter().partition(|(id, _)| held.contains(id));

    let live: HashSet<SnapshotId> = kept
        .iter()
        .flat_map(|s| s.snapshots.iter().copied())
        .chain(live_pins.iter().map(|&&(_, tree)| tree))
        .collect();
    let young_after = now - retention.grace.as_secs() as i64;
    GcPlan {
        sessions: dead.iter().map(|s| s.id).collect(),
        pins: dead_pins.iter().map(|&&(id, _)| id).collect(),
        snapshots: facts
            .snapshots
            .iter()
            .filter(|(id, created)| *created <= young_after && !live.contains(id))
            .map(|&(id, _)| id)
            .collect(),
        content_grace: retention.grace,
    }
}

/// Deletes expired sessions, then the snapshots, tree nodes and content nothing
/// retained refers to. `now` is unix seconds.
pub async fn collect<S: GcStore>(store: &S, now: i64, retention: &Retention) -> Result<GcReport, StoreError> {
    store.gc(now, &|facts| plan(facts, now, retention)).await
}

#[cfg(test)]
mod tests {
    use vet_core::content::Hash;

    use super::*;

    const DAY: i64 = 24 * 3600;
    const NOW: i64 = 100 * DAY;

    fn snap(n: u8) -> SnapshotId {
        SnapshotId(Hash([n; 32]))
    }

    fn session(id: u64, pin: u64, closed_at: Option<i64>, snapshots: &[u8]) -> SessionFacts {
        SessionFacts {
            id: SessionId(id),
            pin: PinId(pin),
            closed_at,
            has_open_thread: false,
            snapshots: snapshots.iter().map(|&n| snap(n)).collect(),
        }
    }

    fn facts(sessions: Vec<SessionFacts>) -> GcFacts {
        GcFacts {
            sessions,
            pins: vec![(PinId(1), snap(10)), (PinId(2), snap(11))],
            snapshots: (1..=11).map(|n| (snap(n), NOW - 2 * DAY)).collect(),
        }
    }

    #[test]
    fn a_closed_session_expires_after_the_retention_and_an_open_thread_keeps_it() {
        let retention = Retention::default();
        let closed = |ago: i64| session(1, 1, Some(NOW - ago), &[1]);
        let expired = plan(&facts(vec![closed(31 * DAY)]), NOW, &retention);
        assert_eq!(expired.sessions, vec![SessionId(1)]);
        let recent = plan(&facts(vec![closed(29 * DAY)]), NOW, &retention);
        assert!(recent.sessions.is_empty());
        let open = plan(&facts(vec![session(1, 1, None, &[1])]), NOW, &retention);
        assert!(open.sessions.is_empty());

        let mut kept = closed(31 * DAY);
        kept.has_open_thread = true;
        assert!(plan(&facts(vec![kept]), NOW, &retention).sessions.is_empty());
    }

    #[test]
    fn a_pin_lives_while_a_session_holds_it() {
        let both = facts(vec![session(1, 1, None, &[]), session(2, 2, Some(NOW - 31 * DAY), &[])]);
        assert_eq!(plan(&both, NOW, &Retention::default()).pins, vec![PinId(2)]);
    }

    #[test]
    fn a_snapshot_lives_while_a_session_or_a_pin_refers_to_it() {
        let f = facts(vec![
            session(1, 1, None, &[1, 2]),
            session(2, 2, Some(NOW - 31 * DAY), &[3]),
        ]);
        let dead: HashSet<_> = plan(&f, NOW, &Retention::default()).snapshots.into_iter().collect();
        // 1 and 2: the live session; 10: its pin. 3 and 11 belong to the expired session.
        let expected: HashSet<_> = [3, 4, 5, 6, 7, 8, 9, 11].map(snap).into_iter().collect();
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
