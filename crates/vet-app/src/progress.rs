//! Background work that a person waits for or wonders about (SPEC §8.11):
//! scans, diffs, restores, servers starting and indexing. Each step is a task with a stable id,
//! a title, an optional fraction and detail, and its own end. A step reports through the
//! [`Tasks`] it is handed and knows nothing about who listens.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::mutex::lock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskKind {
    Scan,
    Diff,
    Restore,
    ServerStart,
    ServerIndex,
    Fetch,
    Store,
    Sync,
    Checks,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TaskState {
    Running,
    Finished,
    /// The step ended without its result; the message is fit to show.
    Failed(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Task {
    /// Unique among the tasks of one [`Tasks`].
    pub id: u32,
    pub kind: TaskKind,
    /// Fit to show: names the step, like "Indexing rust-analyzer".
    pub title: String,
    /// Determinate when both are set.
    pub done: Option<u32>,
    pub total: Option<u32>,
    /// The current step, when the tool says one.
    pub detail: Option<String>,
    pub state: TaskState,
}

/// Receives every announced change of a task. It runs under the reporter's lock,
/// so announcements arrive in order; it must not call back into [`Tasks`].
pub type Sink = Box<dyn Fn(Task) + Send + Sync>;

struct Entry {
    task: Task,
    /// When the task was last announced.
    announced: Instant,
}

struct Shared {
    next: AtomicU32,
    running: Mutex<HashMap<u32, Entry>>,
    sink: Sink,
    /// The least time between two announced updates of one task.
    interval: Duration,
}

/// The tasks of one session. Cheap to clone; a clone reports to the same listeners.
#[derive(Clone)]
pub struct Tasks(Arc<Shared>);

impl Default for Tasks {
    /// Reports to nobody.
    fn default() -> Self {
        Self::new(Duration::ZERO, Box::new(|_| {}))
    }
}

impl Tasks {
    /// Announces a task's start and end at once and its updates at most once per
    /// `interval`, so a step that reports per file does not flood its listeners.
    pub fn new(interval: Duration, sink: Sink) -> Self {
        Self(Arc::new(Shared {
            next: AtomicU32::new(1),
            running: Mutex::default(),
            sink,
            interval,
        }))
    }

    fn running_map(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Entry>> {
        lock(&self.0.running)
    }

    /// Starts a task, announcing it, and returns the handle that updates and ends it.
    pub fn start(&self, kind: TaskKind, title: impl Into<String>) -> Running {
        let id = self.0.next.fetch_add(1, Ordering::Relaxed);
        let task = Task {
            id,
            kind,
            title: title.into(),
            done: None,
            total: None,
            detail: None,
            state: TaskState::Running,
        };
        let mut running = self.running_map();
        (self.0.sink)(task.clone());
        running.insert(
            id,
            Entry {
                task,
                announced: Instant::now(),
            },
        );
        Running {
            tasks: self.clone(),
            id,
            ended: AtomicBool::new(false),
        }
    }

    /// The tasks running now, oldest first: what a connection that joined late missed.
    pub fn running(&self) -> Vec<Task> {
        let mut tasks: Vec<Task> = self.running_map().values().map(|e| e.task.clone()).collect();
        tasks.sort_by_key(|t| t.id);
        tasks
    }

    /// Whether any task is running now, without copying them.
    pub fn any_running(&self) -> bool {
        !self.running_map().is_empty()
    }

    /// Runs `work` as a task: finished when it succeeds, failed with its error
    /// when it does not, and finished if it is dropped half-way.
    pub async fn track<T, E: Display>(
        &self,
        kind: TaskKind,
        title: impl Into<String>,
        work: impl Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        let task = self.start(kind, title);
        let result = work.await;
        task.conclude(&result);
        result
    }

    fn update(&self, id: u32, change: impl FnOnce(&mut Task)) {
        let mut running = self.running_map();
        let Some(entry) = running.get_mut(&id) else {
            return;
        };
        change(&mut entry.task);
        if entry.announced.elapsed() >= self.0.interval {
            entry.announced = Instant::now();
            (self.0.sink)(entry.task.clone());
        }
    }

    fn end(&self, id: u32, state: TaskState) {
        let mut running = self.running_map();
        if let Some(mut entry) = running.remove(&id) {
            entry.task.state = state;
            (self.0.sink)(entry.task);
        }
    }
}

/// One task in progress. Dropping it ends the task as finished, so a step that is
/// cancelled does not stay on screen.
pub struct Running {
    tasks: Tasks,
    id: u32,
    ended: AtomicBool,
}

impl Running {
    /// `done` of `total` units are complete.
    pub fn progress(&self, done: u32, total: u32) {
        self.tasks.update(self.id, |t| {
            t.done = Some(done);
            t.total = Some(total);
        });
    }

    /// What the step is doing now; `None` clears it.
    pub fn detail(&self, detail: Option<String>) {
        self.tasks.update(self.id, |t| t.detail = detail);
    }

    /// Fraction and detail together, so one update carries both.
    pub fn report(&self, done_of_total: Option<(u32, u32)>, detail: Option<String>) {
        self.tasks.update(self.id, |t| {
            (t.done, t.total) = done_of_total.map_or((None, None), |(d, n)| (Some(d), Some(n)));
            t.detail = detail;
        });
    }

    pub fn finish(self) {}

    /// Ends the task as finished if `result` is a success, else as failed with its error.
    pub fn conclude<T, E: Display>(self, result: &Result<T, E>) {
        match result {
            Ok(_) => self.finish(),
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// Ends the task as failed; `message` is fit to show.
    pub fn fail(self, message: String) {
        if !self.ended.swap(true, Ordering::SeqCst) {
            self.tasks.end(self.id, TaskState::Failed(message));
        }
    }
}

/// The task of a computation of the engine: it ends with the computation.
impl vet_engine::Task for Running {
    fn report(&self, done_of_total: Option<(u32, u32)>, detail: Option<String>) {
        Running::report(self, done_of_total, detail);
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.ended.swap(true, Ordering::SeqCst) {
            self.tasks.end(self.id, TaskState::Finished);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    type Heard = Arc<Mutex<Vec<Task>>>;

    fn listening(interval: Duration) -> (Tasks, Heard) {
        let heard = Heard::default();
        let sink = heard.clone();
        let tasks = Tasks::new(interval, Box::new(move |t| sink.lock().unwrap().push(t)));
        (tasks, heard)
    }

    fn states(heard: &Heard) -> Vec<(u32, TaskState)> {
        heard.lock().unwrap().iter().map(|t| (t.id, t.state.clone())).collect()
    }

    #[test]
    fn a_task_is_announced_when_it_starts_updates_and_ends() {
        let (tasks, heard) = listening(Duration::ZERO);
        let scan = tasks.start(TaskKind::Scan, "Scanning");
        scan.progress(1, 4);
        scan.report(Some((2, 4)), Some("src/".into()));
        scan.finish();

        let heard = heard.lock().unwrap();
        let shown: Vec<_> = heard
            .iter()
            .map(|t| (t.title.as_str(), t.done, t.total, t.detail.as_deref(), &t.state))
            .collect();
        assert_eq!(
            shown,
            [
                ("Scanning", None, None, None, &TaskState::Running),
                ("Scanning", Some(1), Some(4), None, &TaskState::Running),
                ("Scanning", Some(2), Some(4), Some("src/"), &TaskState::Running),
                ("Scanning", Some(2), Some(4), Some("src/"), &TaskState::Finished),
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_step_ends_its_task_with_the_error() {
        let (tasks, heard) = listening(Duration::ZERO);
        let result: Result<(), String> = tasks
            .track(TaskKind::Diff, "Diffing", async { Err("git: broken".to_owned()) })
            .await;
        assert_eq!(result, Err("git: broken".into()));
        assert_eq!(
            states(&heard),
            [(1, TaskState::Running), (1, TaskState::Failed("git: broken".into()))]
        );
        assert!(tasks.running().is_empty());
    }

    #[tokio::test]
    async fn a_step_that_succeeds_finishes_and_one_that_is_dropped_does_not_stay() {
        let (tasks, heard) = listening(Duration::ZERO);
        let done: Result<u8, String> = tasks.track(TaskKind::Scan, "Scanning", async { Ok(7) }).await;
        assert_eq!(done, Ok(7));

        let abandoned = tasks.start(TaskKind::Restore, "Restoring");
        drop(abandoned);
        assert_eq!(
            states(&heard),
            [
                (1, TaskState::Running),
                (1, TaskState::Finished),
                (2, TaskState::Running),
                (2, TaskState::Finished),
            ]
        );
    }

    #[test]
    fn running_lists_the_tasks_in_progress_with_their_latest_state() {
        let (tasks, _) = listening(Duration::from_secs(3600));
        let first = tasks.start(TaskKind::ServerStart, "Starting");
        let second = tasks.start(TaskKind::ServerIndex, "Indexing");
        second.report(Some((30, 100)), Some("crates".into()));
        first.finish();

        let running = tasks.running();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].title, "Indexing");
        assert_eq!((running[0].done, running[0].total), (Some(30), Some(100)));
        assert_eq!(running[0].detail.as_deref(), Some("crates"));
    }

    #[test]
    fn updates_within_the_interval_are_held_back_but_the_start_and_end_are_not() {
        let (tasks, heard) = listening(Duration::from_secs(3600));
        let index = tasks.start(TaskKind::ServerIndex, "Indexing");
        for done in 1..=100 {
            index.progress(done, 100);
        }
        index.finish();

        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 2);
        assert_eq!(heard[0].state, TaskState::Running);
        assert_eq!(heard[1].state, TaskState::Finished);
        assert_eq!(heard[1].done, Some(100));
    }
}
