//! Background tasks as protocol DTOs.

use vet_app::progress::{Task, TaskKind, TaskState};
use vet_protocol as p;

pub fn task(project: p::ProjectId, t: &Task) -> p::Task {
    p::Task {
        project,
        id: t.id,
        kind: match t.kind {
            TaskKind::Scan => p::TaskKind::Scan,
            TaskKind::Diff => p::TaskKind::Diff,
            TaskKind::Restore => p::TaskKind::Restore,
            TaskKind::ServerStart => p::TaskKind::ServerStart,
            TaskKind::ServerIndex => p::TaskKind::ServerIndex,
            TaskKind::Fetch => p::TaskKind::Fetch,
            TaskKind::Store => p::TaskKind::Store,
            TaskKind::Sync => p::TaskKind::Sync,
            TaskKind::Checks => p::TaskKind::Checks,
            TaskKind::RemoteFetch => p::TaskKind::RemoteFetch,
        },
        title: t.title.clone(),
        done: t.done,
        total: t.total,
        detail: t.detail.clone(),
        state: match &t.state {
            TaskState::Running => p::TaskState::Running,
            TaskState::Finished => p::TaskState::Finished,
            TaskState::Failed(message) => p::TaskState::Failed {
                message: message.clone(),
            },
        },
    }
}
