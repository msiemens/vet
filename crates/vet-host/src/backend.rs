//! The backend process: reading its `Ready` line and starting it again when it exits.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use vet_protocol::Ready;

/// How long the backend gets to print its Ready line.
const READY_TIMEOUT: Duration = Duration::from_secs(180);

/// How often the backend may exit within [`RESPAWN_WINDOW`] and be started again; after that it
/// stays down until the viewer restarts it.
const RESPAWNS: usize = 3;
const RESPAWN_WINDOW: Duration = Duration::from_secs(60);

/// How long a backend that closed its stdout gets to exit before it is killed.
const EXIT_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    StartingBackend,
    WaitingForReady,
    /// The backend is up; the main window connects to it and shows.
    LoadingUi,
}

/// A started backend, before the shell keeps it.
pub struct Spawned {
    child: Child,
    ready: Ready,
    stdout: BufReader<ChildStdout>,
}

/// Starts a backend, telling the callback which startup step it is in.
pub type Spawn = dyn Fn(&dyn Fn(Step)) -> Result<Spawned, String> + Send + Sync;

/// The backend the shell runs. When it exits on its own it is started again, on a new port with a
/// new token, which [`Backend::connection`] then answers with; the webview connects again by itself.
#[derive(Clone)]
pub struct Backend(Arc<Inner>);

struct Inner {
    /// The repository named on the command line or in `VET_REPO`; none shows the welcome screen.
    repo: Option<String>,
    spawn: Box<Spawn>,
    state: Mutex<State>,
    /// Signalled when a start ends.
    started: Condvar,
}

#[derive(Default)]
struct State {
    /// None while the backend is being started again, and after it was given up on.
    running: Option<Running>,
    /// Counts the starts, so that the watcher of a backend that was replaced leaves the new one be.
    generation: u64,
    /// A start is underway.
    starting: bool,
    /// The app is exiting: nothing is started any more.
    stopped: bool,
    exits: RecentExits,
}

struct Running {
    ready: Ready,
    child: Child,
    generation: u64,
}

impl Backend {
    /// Starts the backend and waits for its `Ready` line, telling `report` which step it is in.
    pub fn launch(repo: Option<String>, spawn: Box<Spawn>, report: impl Fn(Step)) -> Result<Self, String> {
        let spawned = spawn(&report)?;
        let backend = Self(Arc::new(Inner {
            repo,
            spawn,
            state: Mutex::default(),
            started: Condvar::new(),
        }));
        backend.keep(spawned)?;
        Ok(backend)
    }

    #[allow(clippy::disallowed_methods, reason = "the one place that locks the backend's state")]
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The address of the backend running now; none while it is being started again or was given up on.
    pub fn connection(&self) -> Result<Connection, String> {
        let state = self.lock();
        let running = state.running.as_ref().ok_or("the backend is not running")?;
        Ok(Connection {
            ready: running.ready.clone(),
            repo: self.0.repo.clone(),
        })
    }

    /// Ends the backend for good: the app exits.
    pub fn stop(&self) {
        let mut state = self.lock();
        state.stopped = true;
        if let Some(mut running) = state.running.take() {
            let _ = running.child.kill();
        }
    }

    /// Ends the backend, if one runs, and starts another, also after it was given up on. Returns once
    /// the new one printed its `Ready` line.
    pub fn restart(&self) -> Result<(), String> {
        let mut state = self.lock();
        while state.starting {
            state = self.0.started.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        if state.stopped {
            return Err("the app is exiting".into());
        }
        state.exits = RecentExits::default();
        if let Some(mut running) = state.running.take() {
            tracing::info!("restarting the backend");
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
        state.starting = true;
        drop(state);
        self.start()
    }

    /// Spawns a backend and keeps it; the caller set `starting`.
    fn start(&self) -> Result<(), String> {
        let spawned = (self.0.spawn)(&|_| {});
        let mut state = self.lock();
        state.starting = false;
        self.0.started.notify_all();
        drop(state);
        self.keep(spawned?)
    }

    /// Keeps a started backend and watches it for exiting.
    #[allow(
        clippy::disallowed_methods,
        reason = "the backend's output is drained on a thread of its own"
    )]
    fn keep(&self, spawned: Spawned) -> Result<(), String> {
        let Spawned {
            mut child,
            ready,
            stdout,
        } = spawned;
        let mut state = self.lock();
        if state.stopped {
            let _ = child.kill();
            return Err("the app is exiting".into());
        }
        state.generation += 1;
        let generation = state.generation;
        state.running = Some(Running {
            ready,
            child,
            generation,
        });
        drop(state);
        let backend = self.clone();
        std::thread::spawn(move || backend.watch(stdout, generation));
        Ok(())
    }

    /// Waits for the backend of `generation` to exit, says how it ended, and starts another unless it
    /// exited too often lately.
    fn watch(&self, mut stdout: BufReader<ChildStdout>, generation: u64) {
        // The backend prints nothing after its Ready line: the pipe ends when it exits.
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
        let Some(mut running) = self.lock().running.take_if(|r| r.generation == generation) else {
            // Stopped or restarted on purpose.
            return;
        };
        match exit_status(&mut running.child) {
            Ok(status) => tracing::error!(%status, "the backend exited"),
            Err(error) => tracing::error!(%error, "the backend closed its output and cannot be waited for"),
        }
        let mut state = self.lock();
        if state.stopped || state.starting {
            return;
        }
        if !state.exits.admit(Instant::now()) {
            tracing::error!(
                "the backend exited {RESPAWNS} times within {}s: it is not started again until the viewer restarts it",
                RESPAWN_WINDOW.as_secs()
            );
            return;
        }
        state.starting = true;
        drop(state);
        match self.start() {
            Ok(()) => tracing::info!("the backend was started again"),
            Err(message) => tracing::error!(%message, "cannot start the backend again"),
        }
    }
}

/// How a backend whose output ended exited; one that does not exit in a moment is killed.
fn exit_status(child: &mut Child) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now() + EXIT_GRACE;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill()?;
    child.wait()
}

/// The times the backend exited on its own lately.
#[derive(Default)]
struct RecentExits(VecDeque<Instant>);

impl RecentExits {
    /// Records an exit at `now`; whether the backend may be started again after it.
    fn admit(&mut self, now: Instant) -> bool {
        while self
            .0
            .front()
            .is_some_and(|at| now.duration_since(*at) >= RESPAWN_WINDOW)
        {
            self.0.pop_front();
        }
        if self.0.len() >= RESPAWNS {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

/// What the webview needs to connect: the backend's `Ready` line and the repository to open.
#[derive(Serialize)]
pub struct Connection {
    #[serde(flatten)]
    ready: Ready,
    repo: Option<String>,
}

/// The backend runs in WSL, so Windows paths mean nothing to it.
pub fn wsl_distro() -> Option<String> {
    std::env::var("VET_WSL_DISTRO").ok().filter(|_| cfg!(windows))
}

/// The repository to review: the first command-line argument that is not a flag, else
/// `VET_REPO`.
pub fn repo_path() -> Option<String> {
    pick_repo(std::env::args().skip(1), std::env::var("VET_REPO").ok())
}

fn pick_repo(mut args: impl Iterator<Item = String>, env: Option<String>) -> Option<String> {
    args.find(|arg| !arg.starts_with('-'))
        .or(env.filter(|value| !value.is_empty()))
}

/// Runs `command` and waits for the `Ready` line it prints.
#[allow(
    clippy::disallowed_methods,
    reason = "the Ready line is read on a thread of its own, so the wait can time out"
)]
pub fn spawn_command(mut command: Command, binary: &str, report: &dyn Fn(Step)) -> Result<Spawned, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start the backend {binary}: {error}"))?;
    let stdout = child.stdout.take().ok_or("backend has no stdout")?;
    report(Step::WaitingForReady);

    // Read on a thread so the wait can time out; the reader comes back for watching the backend.
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let result = reader.read_line(&mut line).map(|_| line);
        let _ = sender.send((result, reader));
    });
    let (line, stdout) = match receiver.recv_timeout(READY_TIMEOUT) {
        Ok((Ok(line), _)) if line.is_empty() => {
            return Err(abort(
                child,
                format!("the backend {binary} exited before printing its Ready line; see its output above"),
            ));
        }
        Ok((Ok(line), reader)) => (line, reader),
        Ok((Err(error), _)) => return Err(abort(child, format!("cannot read the backend's Ready line: {error}"))),
        Err(_) => return Err(abort(child, "the backend did not print its Ready line in time".into())),
    };
    match serde_json::from_str::<Ready>(&line) {
        Ok(ready) => Ok(Spawned { child, ready, stdout }),
        Err(error) => Err(abort(child, format!("malformed Ready line {line:?}: {error}"))),
    }
}

fn abort(mut child: Child, message: String) -> String {
    let _ = child.kill();
    message
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::sync::atomic::{AtomicU16, Ordering};

    use super::*;

    fn pick(args: &[&str], env: Option<&str>) -> Option<String> {
        pick_repo(args.iter().map(ToString::to_string), env.map(ToOwned::to_owned))
    }

    #[test]
    fn argument_beats_environment_and_neither_shows_the_start_screen() {
        assert_eq!(pick(&["--flag", "/a"], Some("/b")).as_deref(), Some("/a"));
        assert_eq!(pick(&["--flag"], Some("/b")).as_deref(), Some("/b"));
        assert_eq!(pick(&[], Some("")), None);
        assert_eq!(pick(&[], None), None);
    }

    #[test]
    fn three_exits_a_minute_are_started_again_and_a_fourth_is_not() {
        let mut exits = RecentExits::default();
        let start = Instant::now();
        let at = |secs| start + Duration::from_secs(secs);
        assert!(exits.admit(at(0)));
        assert!(exits.admit(at(10)));
        assert!(exits.admit(at(20)));
        assert!(!exits.admit(at(30)));
        // The first exit is a minute old by then.
        assert!(exits.admit(at(60)));
    }

    /// A fake backend: the `n`th start (from 1) prints a Ready line with port `n`, then runs `script(n)`.
    #[cfg(unix)]
    fn fake(script: impl Fn(u16) -> String + Send + Sync + 'static) -> (Box<Spawn>, Arc<AtomicU16>) {
        let starts = Arc::new(AtomicU16::new(0));
        let counter = starts.clone();
        let spawn = Box::new(move |report: &dyn Fn(Step)| {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let mut command = Command::new("/bin/sh");
            command.args(["-c", &format!(r#"echo '{{"port":{n},"token":"t{n}"}}'; {}"#, script(n))]);
            spawn_command(command, "sh", report)
        });
        (spawn, starts)
    }

    /// Waits until `done` holds, for at most five seconds.
    #[cfg(unix)]
    fn eventually(mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[cfg(unix)]
    fn port(backend: &Backend) -> Option<u16> {
        backend.connection().ok().map(|connection| connection.ready.port)
    }

    #[cfg(unix)]
    #[test]
    fn a_backend_that_exits_is_started_again_and_its_new_address_is_handed_out() {
        let (spawn, _) = fake(|n| {
            if n == 1 {
                "exit 3".into()
            } else {
                "exec sleep 30".into()
            }
        });
        let backend = Backend::launch(None, spawn, |_| {}).unwrap();
        assert!(eventually(|| port(&backend) == Some(2)));
        backend.stop();
    }

    #[cfg(unix)]
    #[test]
    fn a_backend_that_keeps_exiting_is_given_up_on_until_it_is_restarted() {
        let (spawn, starts) = fake(|_| "exit 1".into());
        let backend = Backend::launch(None, spawn, |_| {}).unwrap();
        assert!(eventually(
            || starts.load(Ordering::SeqCst) == 1 + RESPAWNS as u16 && backend.connection().is_err()
        ));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(starts.load(Ordering::SeqCst), 1 + RESPAWNS as u16);

        backend.restart().unwrap();
        assert!(starts.load(Ordering::SeqCst) > 1 + RESPAWNS as u16);
        backend.stop();
    }

    #[cfg(unix)]
    #[test]
    fn a_stopped_or_restarted_backend_is_not_started_again_by_its_watcher() {
        let (spawn, starts) = fake(|_| "exec sleep 30".into());
        let backend = Backend::launch(None, spawn, |_| {}).unwrap();
        backend.restart().unwrap();
        assert_eq!(port(&backend), Some(2));
        backend.stop();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert!(backend.connection().is_err());
    }
}
