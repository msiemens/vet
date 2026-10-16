//! The supervisors of the projects' background runs, by project, so the tests can watch one
//! project's runs from the orchestrator.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use vet_app::supervise::Supervisor;
use vet_protocol::ProjectId;

use crate::mutex::lock;

pub(crate) struct Background {
    /// How long a supervisor's shutdown waits for the runs that cooperate.
    bound: Duration,
    supervisors: Mutex<HashMap<ProjectId, Supervisor>>,
}

impl Background {
    pub(crate) fn new(bound: Duration) -> Self {
        Self {
            bound,
            supervisors: Mutex::default(),
        }
    }

    /// The supervisor of the project's runs, made on first ask.
    pub(crate) fn supervisor(&self, project: ProjectId) -> Supervisor {
        lock(&self.supervisors)
            .entry(project)
            .or_insert_with(|| Supervisor::within(self.bound))
            .clone()
    }

    /// How often the project's runs of `kind` were started and how many are running now. For tests
    /// only.
    #[cfg(debug_assertions)]
    pub(crate) fn counts(&self, project: ProjectId, kind: &'static str) -> (usize, usize) {
        lock(&self.supervisors)
            .get(&project)
            .map(|s| s.counts(kind))
            .unwrap_or_default()
    }
}
