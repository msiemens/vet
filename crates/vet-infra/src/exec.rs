//! `vet_app::ports::ProcessRunner` over tokio processes, and the one place
//! that finds and invokes external programs.

mod output;
mod program;
mod tree;

use std::convert::Infallible;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command as TokioCommand;
use tokio::sync::{Notify, mpsc};
use vet_app::ports::{Command, ProcessEvent, ProcessRunner, RunningProcess};

use crate::budget::Budget;

pub use output::{LineSplitter, Tail};
pub use program::{Host, invocation, is_file, parse_pathext};

/// How many lines of a failed tool's output an error message carries.
pub const OUTPUT_TAIL_LINES: usize = 12;

/// The environment that keeps `dotnet` quiet and from phoning home.
pub fn dotnet_env() -> Vec<(String, String)> {
    vec![
        ("DOTNET_NOLOGO".into(), "1".into()),
        ("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into()),
    ]
}

/// `program` as this host runs it, with `env` added, piped output and no input.
/// The process dies with its handle. A repository's `rust-toolchain.toml` must not make
/// rustup download toolchains behind a probe, so auto-install is off for every process.
#[allow(clippy::disallowed_methods, reason = "the one place that spawns processes")]
pub fn command(program: &str, args: &[String], cwd: Option<&Path>, env: &[(String, String)]) -> TokioCommand {
    command_resolved(&invocation(program), args, cwd, env)
}

/// `command` for a program whose location is known: the same, without the `PATH` lookup of
/// `command`, which a caller has already run.
#[allow(clippy::disallowed_methods, reason = "the one place that spawns processes")]
pub fn command_resolved(
    resolved: &Path,
    args: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
) -> TokioCommand {
    let mut command = TokioCommand::new(resolved);
    command
        .args(args)
        .env("RUSTUP_AUTO_INSTALL", "0")
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command
}

pub struct Captured {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a program to completion with a time limit; it is killed when the limit passes.
/// It runs once `budget` has a unit free, and the time limit counts from then.
pub async fn capture(
    budget: &Budget,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<Captured, String> {
    // The lookup of `program` stats every candidate on the `PATH`, however long or slow each
    // one is, so it waits on a blocking thread like the command's own output would. Before
    // the turn: the lookup's unit is given back, and the process that follows runs once
    // a unit is free, as before.
    let named = program.to_owned();
    let resolved = budget
        .blocking(move || invocation(&named))
        .await
        .map_err(|e| e.to_string())?;
    let _turn = budget.acquire().await;
    let mut child = command_resolved(&resolved, args, cwd, env)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let run = async {
        let (out, err) = tokio::join!(
            read_tail(&mut stdout, CAPTURE_LIMIT),
            read_tail(&mut stderr, CAPTURE_LIMIT)
        );
        let (out, err) = (out.map_err(|e| e.to_string())?, err.map_err(|e| e.to_string())?);
        let status = child.wait().await.map_err(|e| e.to_string())?;
        Ok(Captured {
            ok: status.success(),
            stdout: out,
            stderr: err,
        })
    };
    tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| format!("{program} did not finish within {}s", timeout.as_secs()))?
}

/// How many bytes of each output stream `capture` keeps.
const CAPTURE_LIMIT: usize = 8 * 1024 * 1024;

/// Reads `stream` to its end and decodes the last `limit` bytes lossily. Failure messages
/// quote the end of the output, so that is the part kept; the rest is read and dropped.
async fn read_tail(stream: &mut (impl AsyncRead + Unpin), limit: usize) -> std::io::Result<String> {
    let mut kept = Vec::new();
    let mut buf = vec![0; 16 * 1024];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        kept.extend_from_slice(&buf[..n]);
        if kept.len() > 2 * limit {
            kept.drain(..kept.len() - limit);
        }
    }
    let start = kept.len().saturating_sub(limit);
    Ok(String::from_utf8_lossy(&kept[start..]).into_owned())
}

/// How long a killed process gets to wind down before its tree is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// How many output events of one process may be waiting: a process that writes faster than the
/// consumer reads blocks on its pipe rather than filling memory.
const EVENT_QUEUE: usize = 64;

/// Must be called from within a tokio runtime.
pub struct TokioRunner;

pub struct Process {
    events: mpsc::Receiver<ProcessEvent>,
    kill: Arc<Notify>,
    exit: Option<ProcessEvent>,
}

impl ProcessRunner for TokioRunner {
    type Process = Process;

    fn spawn(&self, command: &Command) -> Result<Process, String> {
        let mut spawn = self::command(&command.program, &command.args, Some(&command.cwd), &command.env);
        tree::prepare(&mut spawn);
        let mut child = spawn.spawn().map_err(|e| format!("{}: {e}", command.program))?;
        // On failure `child` drops and takes the leader with it.
        let tree = tree::Tree::attach(&child).map_err(|e| format!("{}: {e}", command.program))?;
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx, events) = mpsc::channel(EVENT_QUEUE);
        let kill = Arc::new(Notify::new());
        let killed = kill.clone();
        tokio::spawn(async move {
            let run = async {
                let (_, _, status) = tokio::join!(
                    pump(stdout, &tx, ProcessEvent::Stdout),
                    pump(stderr, &tx, ProcessEvent::Stderr),
                    child.wait()
                );
                status
            };
            // Stop is not tied to the leader: descendants can keep the output pipes
            // open after it exited, and the run is not over until they close.
            let stop = async {
                killed.notified().await;
                tree.terminate();
                tokio::time::sleep(STOP_GRACE).await;
                tree.kill();
                std::future::pending::<Infallible>().await
            };
            let status = tokio::select! {
                status = run => status,
                never = stop => match never {},
            };
            // Whatever the run left behind ends with it, by `tree` dropping.
            drop(tree);
            let _ = tx.send(ProcessEvent::Exit(status.ok().and_then(|s| s.code()))).await;
        });
        Ok(Process {
            events,
            kill,
            exit: None,
        })
    }
}

async fn pump(mut stream: impl AsyncRead + Unpin, tx: &mpsc::Sender<ProcessEvent>, event: fn(Vec<u8>) -> ProcessEvent) {
    let mut buf = vec![0; 16 * 1024];
    while let Ok(n) = stream.read(&mut buf).await {
        if n == 0 || tx.send(event(buf[..n].to_vec())).await.is_err() {
            break;
        }
    }
}

impl RunningProcess for Process {
    async fn next(&mut self) -> ProcessEvent {
        if let Some(exit) = &self.exit {
            return exit.clone();
        }
        // The task always sends `Exit` last; a closed channel means it was aborted.
        let event = self.events.recv().await.unwrap_or(ProcessEvent::Exit(None));
        if matches!(event, ProcessEvent::Exit(_)) {
            self.exit = Some(event.clone());
        }
        event
    }

    fn kill(&self) {
        self.kill.notify_one();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        Command {
            program: "sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: vec![("GREETING".into(), "hi".into())],
        }
    }

    async fn drain(mut p: Process) -> (Vec<u8>, Vec<u8>, ProcessEvent) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        loop {
            match p.next().await {
                ProcessEvent::Stdout(b) => out.extend(b),
                ProcessEvent::Stderr(b) => err.extend(b),
                exit => return (out, err, exit),
            }
        }
    }

    #[tokio::test]
    async fn streams_output_and_reports_the_exit_code() {
        let p = TokioRunner.spawn(&sh("echo $GREETING; echo oops >&2; exit 3")).unwrap();
        let (out, err, exit) = drain(p).await;
        assert_eq!(out, b"hi\n");
        assert_eq!(err, b"oops\n");
        assert_eq!(exit, ProcessEvent::Exit(Some(3)));
    }

    #[tokio::test]
    async fn a_noisy_process_stalls_at_the_queue_bound_rather_than_buffering_without_one() {
        // Writes far more than the queue holds. Nothing reads it, so the process must stall on its
        // pipe at the bound instead of the whole output piling up in memory.
        let p = TokioRunner.spawn(&sh("seq 1 200000")).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(p.events.len() <= EVENT_QUEUE, "{} events buffered", p.events.len());
        // Nothing is lost once it is read: the queue throttles the process, it does not drop.
        let (out, err, exit) = drain(p).await;
        assert!(err.is_empty());
        assert_eq!(exit, ProcessEvent::Exit(Some(0)));
        assert_eq!(out.iter().filter(|&&c| c == b'\n').count(), 200000);
        assert!(out.ends_with(b"200000\n"));
    }

    #[tokio::test]
    async fn capture_decodes_output_that_is_not_utf8() {
        let args = ["-c".to_owned(), r"printf 'ok \377 end'".to_owned()];
        let captured = capture(&Budget::new(1), "sh", &args, None, &[], Duration::from_secs(10))
            .await
            .unwrap();
        assert!(captured.ok);
        assert_eq!(captured.stdout, "ok \u{fffd} end");
    }

    #[tokio::test]
    async fn the_end_of_long_output_is_kept() {
        let mut stream: &[u8] = b"head-0123456789-tail";
        assert_eq!(read_tail(&mut stream, 4).await.unwrap(), "tail");
        let mut short: &[u8] = b"abc";
        assert_eq!(read_tail(&mut short, 4).await.unwrap(), "abc");
    }

    #[tokio::test]
    async fn rustup_may_not_install_toolchains_for_a_spawned_process() {
        let p = TokioRunner.spawn(&sh("echo $RUSTUP_AUTO_INSTALL")).unwrap();
        let (out, _, _) = drain(p).await;
        assert_eq!(out, b"0\n");
    }

    #[tokio::test]
    async fn kill_ends_the_process() {
        let mut p = TokioRunner.spawn(&sh("sleep 30")).unwrap();
        p.kill();
        assert_eq!(p.next().await, ProcessEvent::Exit(None));
        assert_eq!(p.next().await, ProcessEvent::Exit(None), "exit is sticky");
    }

    #[tokio::test]
    async fn a_missing_program_fails_to_spawn() {
        let command = Command {
            program: "definitely-not-a-program".into(),
            args: vec![],
            cwd: std::env::temp_dir(),
            env: vec![],
        };
        assert!(TokioRunner.spawn(&command).is_err());
    }
}
