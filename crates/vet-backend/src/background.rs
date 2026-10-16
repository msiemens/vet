//! Counting the background tasks of the projects, so `Request::WaitForQuiet` and the tests can see
//! them end.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use vet_protocol::ProjectId;

use crate::Open;
use crate::mutex::lock;

type Counts = HashMap<(ProjectId, &'static str), (usize, usize)>;

/// How often each background task of each project was started, and how many of them are running
/// now. The live count is what `Request::WaitForQuiet` waits on; the started count is for the
/// tests that watch one kind end.
#[derive(Default)]
pub(crate) struct Background(Mutex<Counts>);

impl Background {
    /// Counts a task of `kind` that is starting; dropping the guard says it ended, whether it
    /// returned or was aborted.
    pub(crate) fn start(self: &Arc<Self>, project: ProjectId, kind: &'static str) -> LiveTask {
        {
            let mut all = lock(&self.0);
            let counts = all.entry((project, kind)).or_default();
            counts.0 += 1;
            counts.1 += 1;
        }
        LiveTask {
            background: self.clone(),
            project,
            kind,
        }
    }

    /// How many background tasks of the project are running now.
    pub(crate) fn live(&self, project: ProjectId) -> usize {
        lock(&self.0)
            .iter()
            .filter(|((at, _), _)| *at == project)
            .map(|(_, (_, live))| live)
            .sum()
    }

    /// How often the task was started and how many are running now. For tests only.
    #[cfg(debug_assertions)]
    pub(crate) fn counts(&self, project: ProjectId, kind: &'static str) -> (usize, usize) {
        lock(&self.0).get(&(project, kind)).copied().unwrap_or_default()
    }
}

/// The end of one counted background task.
pub(crate) struct LiveTask {
    background: Arc<Background>,
    project: ProjectId,
    kind: &'static str,
}

impl Drop for LiveTask {
    fn drop(&mut self) {
        if let Some(counts) = lock(&self.background.0).get_mut(&(self.project, self.kind)) {
            counts.1 = counts.1.saturating_sub(1);
        }
    }
}

/// `fut` as a background task of `kind` that `Request::WaitForQuiet`, and the tests, can see end.
pub(crate) fn tracked<F: std::future::Future>(
    open: &Arc<Open>,
    kind: &'static str,
    fut: F,
) -> impl std::future::Future<Output = F::Output> + use<F> {
    let live = open.background.start(open.id, kind);
    async move {
        let _live = live;
        fut.await
    }
}
