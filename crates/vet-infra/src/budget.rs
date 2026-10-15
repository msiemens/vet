//! Every way the backend leaves the async threads draws on a budget: blocking work
//! (file reads, git, hashing) and processes that run to completion. A budget is a
//! bound shared by everything that draws on it, so the number of threads and
//! processes at work is set here, not by how many callers ask.
//!
//! The review lane and the semantic lane have pools of their own. Neither can
//! take the other's permits, so semantic work that saturates its pool leaves the
//! review's I/O untouched. Work nobody waits for (the copy of the working tree's
//! content ahead of a review) has a smaller pool of its own, so it never holds up
//! what a request needs.
//!
//! [`Budget::blocking`] is the only `spawn_blocking` of the workspace and `exec` the only
//! place that spawns processes; `crates/clippy.toml` forbids both elsewhere.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinError;

/// Blocking tasks at once on the review lane.
pub const REVIEW_IO: usize = 4;
/// Blocking tasks at once on the semantic lane.
pub const SEMANTIC_IO: usize = 4;
/// Blocking tasks at once for work done ahead of a request.
pub const BACKGROUND_IO: usize = 2;
/// Blocking git work at once for a fetch, which takes as long as the network does and so must
/// not take the review lane's threads. Each open project has a fetch budget of its own.
pub const FETCH_IO: usize = 4;
/// Probe processes (`--version`, toolchain captures) at once.
pub const PROBES: usize = 2;
/// Restore steps at once, over all trees.
pub const RESTORES: usize = 2;

/// A bound on how much of one kind of work runs at once. Cheap to clone: clones share the bound.
#[derive(Clone)]
pub struct Budget {
    permits: Arc<Semaphore>,
    limit: usize,
}

/// One unit of a budget, given back when dropped.
#[must_use]
pub struct Permit(#[allow(dead_code)] OwnedSemaphorePermit);

impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(limit)),
            limit,
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// How many units are free now.
    pub fn available(&self) -> usize {
        self.permits.available_permits()
    }

    /// A unit of the budget, waiting in line for it: units go to the callers in the order they asked.
    pub async fn acquire(&self) -> Permit {
        let permit = self.permits.clone().acquire_owned().await;
        Permit(permit.expect("a budget is never closed"))
    }

    /// Every unit of the budget at once, for as long as the permit lives.
    pub async fn acquire_all(&self) -> Permit {
        let permit = self.permits.clone().acquire_many_owned(self.limit as u32).await;
        Permit(permit.expect("a budget is never closed"))
    }

    /// Runs `work` on a blocking thread once a unit is free. The unit is held until `work`
    /// has ended, even if the caller stops waiting for it.
    #[allow(clippy::disallowed_methods, reason = "the one place that spawns blocking work")]
    pub async fn blocking<T: Send + 'static>(&self, work: impl FnOnce() -> T + Send + 'static) -> Result<T, JoinError> {
        let permit = self.acquire().await;
        tokio::task::spawn_blocking(move || {
            let _held = permit;
            work()
        })
        .await
    }
}

/// The budgets of a backend.
#[derive(Clone)]
pub struct Budgets {
    /// Git, the content store and the CPU work of the review lane.
    pub review_io: Budget,
    /// Reads of texts, materialization, restore plans and hashing for the semantic lane.
    pub semantic_io: Budget,
    /// Git and the content store for work done ahead of a request.
    pub background_io: Budget,
    /// Processes that only answer a question.
    pub probes: Budget,
    /// Steps that restore a tree's dependencies.
    pub restores: Budget,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            review_io: Budget::new(REVIEW_IO),
            semantic_io: Budget::new(SEMANTIC_IO),
            background_io: Budget::new(BACKGROUND_IO),
            probes: Budget::new(PROBES),
            restores: Budget::new(RESTORES),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn no_more_blocking_tasks_run_than_the_budget_allows() {
        let budget = Budget::new(3);
        let (running, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let tasks = (0..12).map(|_| {
            let (budget, running, peak) = (budget.clone(), running.clone(), peak.clone());
            tokio::spawn(async move {
                budget
                    .blocking(move || {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(20));
                        running.fetch_sub(1, Ordering::SeqCst);
                    })
                    .await
                    .unwrap();
            })
        });
        for task in tasks.collect::<Vec<_>>() {
            task.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 3);
        assert_eq!(budget.available(), 3);
    }

    #[tokio::test]
    async fn a_pool_that_is_exhausted_leaves_the_other_pool_alone() {
        let budgets = Budgets::default();
        let _held = budgets.semantic_io.acquire_all().await;
        let read = tokio::time::timeout(Duration::from_secs(10), budgets.review_io.blocking(|| 7)).await;
        assert_eq!(read.expect("the review pool is free").unwrap(), 7);
        let waiting = tokio::time::timeout(Duration::from_millis(100), budgets.semantic_io.blocking(|| 7)).await;
        assert!(waiting.is_err(), "the semantic pool has no unit left");
    }

    #[tokio::test]
    async fn a_unit_is_held_until_the_work_ends_even_when_the_caller_gave_up() {
        let budget = Budget::new(1);
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let (started, running) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn({
            let budget = budget.clone();
            async move {
                let _ = budget
                    .blocking(move || {
                        let _ = started.send(());
                        let _ = wait.recv();
                    })
                    .await;
            }
        });
        running.await.unwrap();
        caller.abort();
        assert_eq!(budget.available(), 0);
        release.send(()).unwrap();
        let _unit = tokio::time::timeout(Duration::from_secs(10), budget.acquire())
            .await
            .expect("the unit is given back when the work ends");
    }
}
