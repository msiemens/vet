//! Run requests (§7.5, §9): the run check, the approvals waiting for the user
//! and the processes a session started.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::sync::Notify;
use vet_app::execution::check::{Approval, RunCheckConfig};
use vet_app::execution::targets::RunTarget;
use vet_app::execution::{Execution, LaunchOutcome};
use vet_app::ports::{ProcessEvent, RunningProcess};
use vet_core::content::Hash;
use vet_infra::exec::{Process, TokioRunner};
use vet_infra::git::GixRepo;
use vet_infra::store::{SqliteStore, unix_now};
use vet_protocol::{Event, Response, RunStream};

use super::{Open, Subscribers, live, mapping, message};

static RUNNER: TokioRunner = TokioRunner;

/// Bytes of output one run sends to clients, both streams together. Later output
/// is drained and dropped; `RunExited::dropped_bytes` says how much.
pub const OUTPUT_CAP: usize = 4 * 1024 * 1024;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a session keeps about running.
#[derive(Default)]
pub struct Runs {
    next: AtomicU32,
    /// Approvals the user was asked about, by hash, until the run they guard starts.
    pending: Mutex<HashMap<Hash, Approval>>,
    /// Running processes, each with the signal that kills it.
    running: Arc<Mutex<HashMap<u32, Arc<Notify>>>>,
}

impl Runs {
    /// Kills every running process.
    pub fn stop_all(&self) {
        for stop in locked(&self.running).values() {
            stop.notify_one();
        }
    }
}

impl Open {
    fn execution(&self) -> Execution<'_, GixRepo, SqliteStore, TokioRunner> {
        self.session.execution(
            &RUNNER,
            self.repo.to_string_lossy().into_owned(),
            self.repo.clone(),
            RunCheckConfig::default(),
        )
    }

    pub async fn run_targets(&self) -> Result<Response, String> {
        let reviewed = self.session.target_snapshot().await;
        let listed = self.execution().targets(&reviewed).await.map_err(message)?;
        Ok(mapping::run_targets(&listed))
    }

    pub async fn save_run_target(&self, target: RunTarget) -> Result<(), String> {
        self.execution().save_target(target).await.map_err(message)
    }

    pub async fn reset_run_target(&self, id: String) -> Result<(), String> {
        self.execution().reset_target(id).await.map_err(message)
    }

    pub async fn run(&self, target_id: &str, approved: Option<&str>) -> Result<Response, String> {
        let approved = approved
            .map(|hex| Hash::from_hex(hex).ok_or("not an approval hash"))
            .transpose()?;
        let reviewed = self.session.target_snapshot().await;
        let outcome = self
            .execution()
            .run(target_id, &reviewed, approved, unix_now())
            .await
            .map_err(message)?;
        match outcome {
            LaunchOutcome::NeedsApproval(approval) => {
                let response = mapping::needs_approval(&approval);
                locked(&self.runs.pending).insert(approval.hash, approval);
                Ok(response)
            }
            LaunchOutcome::Started(started) => {
                if let Some(hash) = approved {
                    locked(&self.runs.pending).remove(&hash);
                }
                let run_id = self.runs.next.fetch_add(1, Ordering::Relaxed) + 1;
                self.pump(run_id, started.process);
                Ok(Response::RunStarted {
                    run_id,
                    snapshot: started.snapshot.0.to_hex(),
                    changed_since_review: u32::try_from(started.changed_since_review).unwrap_or(u32::MAX),
                })
            }
        }
    }

    pub async fn run_approval_diff(&self, hash: &str) -> Result<Response, String> {
        let approval = Hash::from_hex(hash)
            .and_then(|hash| locked(&self.runs.pending).get(&hash).cloned())
            .ok_or("no pending approval with this hash")?;
        let files = self.execution().approval_files(&approval).await.map_err(message)?;
        Ok(mapping::approval_files(&files))
    }

    /// Asks the run to end; a no-op once it has.
    pub fn stop_run(&self, run_id: u32) {
        if let Some(stop) = locked(&self.runs.running).get(&run_id) {
            stop.notify_one();
        }
    }

    /// Forwards the process's output and exit to the session's subscribers.
    fn pump(&self, run_id: u32, mut process: Process) {
        let stop = Arc::new(Notify::new());
        locked(&self.runs.running).insert(run_id, stop.clone());
        let running = self.runs.running.clone();
        let (session, subscribers) = (self.id, self.subscribers.clone());
        tokio::spawn(async move {
            let mut stopping = false;
            let mut sent = 0;
            let mut dropped = 0usize;
            let code = loop {
                let event = tokio::select! {
                    event = process.next() => event,
                    () = stop.notified(), if !stopping => {
                        stopping = true;
                        process.kill();
                        continue;
                    }
                };
                let (stream, bytes) = match event {
                    ProcessEvent::Stdout(bytes) => (RunStream::Stdout, bytes),
                    ProcessEvent::Stderr(bytes) => (RunStream::Stderr, bytes),
                    ProcessEvent::Exit(code) => break code,
                };
                let keep = bytes.len().min(OUTPUT_CAP - sent);
                sent += keep;
                dropped += bytes.len() - keep;
                if keep == 0 {
                    continue;
                }
                let bytes = STANDARD.encode(&bytes[..keep]);
                broadcast(
                    &subscribers,
                    Event::RunOutput {
                        session,
                        run_id,
                        stream,
                        bytes,
                    },
                );
            };
            locked(&running).remove(&run_id);
            let exited = Event::RunExited {
                session,
                run_id,
                code,
                dropped_bytes: u32::try_from(dropped).unwrap_or(u32::MAX),
            };
            broadcast(&subscribers, exited);
        });
    }
}

fn broadcast(subscribers: &Subscribers, event: Event) {
    for events in live(subscribers).iter() {
        events.send(event.clone());
    }
}
