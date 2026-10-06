//! Where a session's analysis is limited because a dependency restore failed
//! (§7.5). A restore runs when a server starts on a snapshot, long after the
//! request that needed the server, so the outcome is kept and announced.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Backend, RestoreFailure, Restored, Restorer};

/// The failures that currently limit the session. A project's entry is replaced
/// when its step runs again, so fixing the project lifts the limit.
#[derive(Clone, Default)]
pub struct RestoreLimits(Arc<Mutex<Vec<RestoreFailure>>>);

impl RestoreLimits {
    pub fn all(&self) -> Vec<RestoreFailure> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Applies the outcome of a restore; the new list if it differs from the old one.
    pub fn record(&self, restored: &Restored) -> Option<Vec<RestoreFailure>> {
        let mut all = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let before = all.clone();
        all.retain(|f| {
            !restored
                .ran
                .iter()
                .any(|(project, tool)| (project, tool) == (&f.project, &f.tool))
        });
        for failure in &restored.failures {
            if !all.contains(failure) {
                all.push(failure.clone());
            }
        }
        (*all != before).then(|| all.clone())
    }
}

/// Called with the whole list whenever it changes.
pub type Changed = Box<dyn Fn(Vec<RestoreFailure>) + Send + Sync>;

/// Restores with `R` and keeps `limits` current.
pub struct Limiting<R> {
    restorer: R,
    limits: RestoreLimits,
    changed: Changed,
}

impl<R> Limiting<R> {
    pub fn new(restorer: R, limits: RestoreLimits, changed: Changed) -> Self {
        Self {
            restorer,
            limits,
            changed,
        }
    }
}

impl<R: Restorer + Sync> Restorer for Limiting<R> {
    async fn restore(&self, root: &Path, backend: &Backend, policy: AnalysisPolicy) -> Restored {
        let restored = self.restorer.restore(root, backend, policy).await;
        if let Some(all) = self.limits.record(&restored) {
            (self.changed)(all);
        }
        restored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(project: &str) -> RestoreFailure {
        RestoreFailure {
            project: project.into(),
            tool: "pnpm".into(),
            message: "failed".into(),
        }
    }

    fn outcome(failed: &[&str], ran: &[&str]) -> Restored {
        Restored {
            failures: failed.iter().map(|p| failure(p)).collect(),
            ran: ran.iter().map(|p| ((*p).to_owned(), "pnpm".to_owned())).collect(),
            ..Restored::default()
        }
    }

    fn projects(list: Option<Vec<RestoreFailure>>) -> Option<Vec<String>> {
        list.map(|l| l.into_iter().map(|f| f.project).collect())
    }

    #[test]
    fn only_a_change_is_news() {
        let limits = RestoreLimits::default();
        assert_eq!(limits.record(&outcome(&[], &["a"])), None);
        assert_eq!(
            projects(limits.record(&outcome(&["a"], &["a"]))),
            Some(vec!["a".into()])
        );
        assert_eq!(limits.record(&outcome(&["a"], &["a"])), None);
        assert_eq!(
            projects(limits.record(&outcome(&["b"], &["b"]))),
            Some(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn a_later_success_lifts_the_limit_and_an_untouched_project_keeps_it() {
        let limits = RestoreLimits::default();
        limits.record(&outcome(&["a", "b"], &["a", "b"]));
        assert_eq!(projects(limits.record(&outcome(&[], &["a"]))), Some(vec!["b".into()]));
        assert_eq!(projects(limits.record(&outcome(&[], &["b"]))), Some(vec![]));
        assert_eq!(limits.all(), vec![]);
    }
}
