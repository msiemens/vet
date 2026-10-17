//! The versions of a pull request are the pushes its platform recorded, from the first one: the
//! platform's list of them (`remember_pushes`) says which states of the source branch there were,
//! and each becomes a version when a review starts or the pull request moves on. A push whose
//! commits cannot be had (the platform dropped them after a force-push) is still a version, one
//! without content, so the timeline says it existed and nothing wrong is shown for it.

use std::sync::Arc;

use vet_core::content::{Snapshot, SnapshotStatus};

use super::state::State;
use super::versions::first_change;
use super::{LiveSession, Result};
use crate::base;
use crate::changeset::{CommitId, Snapshots, Source, Version, VersionChange, VersionOrigin};
use crate::platform::Push;
use crate::ports::{GitRepo, LiveStore};

/// The base and the target of a version, held while it is recorded.
pub(super) type Trees = (Arc<Snapshot>, Arc<Snapshot>);

/// A version about to be recorded, with the content it needs stored.
pub(super) struct Recording {
    pub version: Version,
    pub trees: Option<Trees>,
}

impl Recording {
    /// The snapshots to store for the version, with their status.
    pub(super) fn snapshots(&self) -> Vec<(&Snapshot, SnapshotStatus)> {
        self.trees
            .iter()
            .flat_map(|(base, target)| [base, target])
            .map(|s| (&**s, SnapshotStatus::of(s)))
            .collect()
    }
}

/// The pushes that are not versions yet, the last of them the state at `tip`; none if the pushes
/// known do not end at `tip` (they are older than it, or none were asked for).
///
/// What follows the latest push already recorded is new. A platform that lists only some of the
/// states (GitHub names the ones a force-push replaced) may not list the state last recorded; then
/// only `tip` can be told to be new.
pub(super) fn pending(known: &[Push], recorded: &[CommitId], tip: &CommitId) -> Option<Vec<Push>> {
    if known.last()?.head != tip.0 {
        return None;
    }
    let after = known.iter().rposition(|p| recorded.iter().any(|r| r.0 == p.head));
    Some(match after {
        Some(i) => known[i + 1..].to_vec(),
        None if recorded.is_empty() => known.to_vec(),
        None => known[known.len() - 1..].to_vec(),
    })
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The new pushes of the pull request in `st`, whose latest version is built on `tip`, given
    /// the commits already recorded as versions; the last is the state `st` is moving to.
    pub(super) fn new_pushes(&self, st: &State, recorded: &[CommitId], tip: &CommitId) -> Option<Vec<Push>> {
        if !matches!(st.source, Source::PullRequest { .. }) {
            return None;
        }
        pending(&self.exchange.pushes_of(&st.source), recorded, tip)
    }

    /// The versions numbered from `first` that `pushes` stand for, where the content of each is
    /// worked out against the one before it (`previous` for the first). Returns the trees of the
    /// last that has some. A push whose commits cannot be had has no content.
    pub(super) async fn history(
        &self,
        st: &State,
        pushes: &[Push],
        first: u32,
        mut previous: Option<Trees>,
    ) -> Result<(Vec<Recording>, Option<Trees>)> {
        let git = &self.acquisition.git;
        let mut found = Vec::new();
        for (push, n) in pushes.iter().zip(first..) {
            let tip = CommitId(push.head.clone());
            let at = match push.at {
                Some(at) => at,
                None => git
                    .commit_info(&tip)
                    .await
                    .map_or_else(|_| (self.clock)(), |info| info.time),
            };
            let built = async {
                let (_, base_commit) = base::resolve(git, st.base_choice.clone(), &tip).await?;
                let target = Arc::new(git.commit_snapshot(&tip).await?);
                let base = Arc::new(git.commit_snapshot(&base_commit).await?);
                Ok::<_, super::SessionError>((base_commit, base, target))
            }
            .await;
            let recording = match built {
                Ok((base_commit, base, target)) => {
                    let change = match &previous {
                        Some((old_base, old_target)) => self.change_of((old_base, old_target), &base, &target).await?,
                        None => first_change(&*self.facts.diff_of(&base, &target).await?),
                    };
                    self.acquisition.record_commit(base.id, base_commit.clone());
                    self.acquisition.record_commit(target.id, tip.clone());
                    let version = Version {
                        n,
                        snapshots: Some(Snapshots {
                            base: base.id,
                            target: target.id,
                        }),
                        base_commit,
                        tip,
                        at,
                        origin: VersionOrigin::Observed,
                        change,
                    };
                    previous = Some((base.clone(), target.clone()));
                    Recording {
                        version,
                        trees: Some((base, target)),
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, push = %push.head, "a push of the pull request cannot be had");
                    Recording {
                        version: Version {
                            n,
                            snapshots: None,
                            base_commit: CommitId(String::new()),
                            tip,
                            at,
                            origin: VersionOrigin::Observed,
                            change: VersionChange::default(),
                        },
                        trees: None,
                    }
                }
            };
            found.push(recording);
        }
        Ok((found, previous))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(head: &str) -> Push {
        Push {
            head: head.to_owned(),
            at: None,
            forced: false,
        }
    }

    fn pushes(heads: &[&str]) -> Vec<Push> {
        heads.iter().map(|h| push(h)).collect()
    }

    fn commits(ids: &[&str]) -> Vec<CommitId> {
        ids.iter().map(|c| CommitId((*c).to_owned())).collect()
    }

    fn heads(found: Option<Vec<Push>>) -> Option<Vec<String>> {
        found.map(|p| p.into_iter().map(|p| p.head).collect())
    }

    #[test]
    fn a_review_that_starts_has_every_push_as_a_version() {
        let new = pending(&pushes(&["a", "b", "c"]), &[], &CommitId("c".into()));
        assert_eq!(heads(new), Some(vec!["a".into(), "b".into(), "c".into()]));
    }

    #[test]
    fn only_the_pushes_after_the_latest_recorded_one_are_new() {
        let known = pushes(&["a", "b", "c", "d"]);
        let new = pending(&known, &commits(&["a", "b"]), &CommitId("d".into()));
        assert_eq!(heads(new), Some(vec!["c".into(), "d".into()]));
    }

    #[test]
    fn a_recorded_state_the_platform_does_not_list_leaves_only_the_current_one_new() {
        // GitHub lists the states a force-push replaced; an ordinary push on top of "b" is not one.
        let new = pending(&pushes(&["a", "d"]), &commits(&["a", "b"]), &CommitId("d".into()));
        assert_eq!(heads(new), Some(vec!["d".into()]));
        let unrelated = pending(&pushes(&["x", "d"]), &commits(&["b"]), &CommitId("d".into()));
        assert_eq!(heads(unrelated), Some(vec!["d".into()]));
    }

    #[test]
    fn pushes_that_do_not_end_at_the_tip_say_nothing() {
        assert_eq!(pending(&pushes(&["a", "b"]), &[], &CommitId("c".into())), None);
        assert_eq!(pending(&[], &[], &CommitId("c".into())), None);
    }
}
