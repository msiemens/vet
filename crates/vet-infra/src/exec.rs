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
/// The process dies with its handle.
pub fn command(program: &str, args: &[String], cwd: Option<&Path>, env: &[(String, String)]) -> TokioCommand {
    let (invoked, args) = invocation(program, args);
    let mut command = TokioCommand::new(invoked);
    command
        .args(args)
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
pub async fn capture(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<Captured, String> {
    let mut child = command(program, args, cwd, env)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let run = async {
        let (mut out, mut err) = (String::new(), String::new());
        let (a, b) = tokio::join!(stdout.read_to_string(&mut out), stderr.read_to_string(&mut err));
        a.and(b).map_err(|e| e.to_string())?;
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

/// How long a killed process gets to wind down before its tree is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// Must be called from within a tokio runtime.
pub struct TokioRunner;

pub struct Process {
    events: mpsc::UnboundedReceiver<ProcessEvent>,
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
        let (tx, events) = mpsc::unbounded_channel();
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
            let _ = tx.send(ProcessEvent::Exit(status.ok().and_then(|s| s.code())));
        });
        Ok(Process {
            events,
            kill,
            exit: None,
        })
    }
}

async fn pump(
    mut stream: impl AsyncRead + Unpin,
    tx: &mpsc::UnboundedSender<ProcessEvent>,
    event: fn(Vec<u8>) -> ProcessEvent,
) {
    let mut buf = vec![0; 16 * 1024];
    while let Ok(n) = stream.read(&mut buf).await {
        if n == 0 || tx.send(event(buf[..n].to_vec())).is_err() {
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
