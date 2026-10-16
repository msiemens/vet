//! The supervisors of the projects' background runs, by project, so the tests can watch one
//! project's runs from the orchestrator.

use std::collections::HashMap;
use std::sync::Mutex;

use vet_app::supervise::Supervisor;
use vet_protocol::ProjectId;

use crate::mutex::lock;

#[derive(Default)]
pub(crate) struct Background(Mutex<HashMap<ProjectId, Supervisor>>);

impl Background {
    /// The supervisor of the project's runs, made on first ask.
    pub(crate) fn supervisor(&self, project: ProjectId) -> Supervisor {
        lock(&self.0).entry(project).or_default().clone()
    }

    /// How often the project's runs of `kind` were started and how many are running now. For tests
    /// only.
    #[cfg(debug_assertions)]
    pub(crate) fn counts(&self, project: ProjectId, kind: &'static str) -> (usize, usize) {
        lock(&self.0).get(&project).map(|s| s.counts(kind)).unwrap_or_default()
    }
}
