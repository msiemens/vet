//! `vet_app::ports::ProcessRunner` over tokio processes, and the one place
//! that finds and invokes external programs.

mod program;

use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command as TokioCommand;
use tokio::sync::{Notify, mpsc};
use vet_app::ports::{Command, ProcessEvent, ProcessRunner, RunningProcess};

pub use program::{Host, invocation, is_file, parse_pathext};

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
        let (program, args) = invocation(&command.program, &command.args);
        let mut child = TokioCommand::new(program)
            .args(args)
            .current_dir(&command.cwd)
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("{}: {e}", command.program))?;
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx, events) = mpsc::unbounded_channel();
        let kill = Arc::new(Notify::new());
        let killed = kill.clone();
        tokio::spawn(async move {
            let wait = async {
                tokio::select! {
                    status = child.wait() => status,
                    () = killed.notified() => {
                        let _ = child.kill().await;
                        child.wait().await
                    }
                }
            };
            let (_, _, status) = tokio::join!(
                pump(stdout, &tx, ProcessEvent::Stdout),
                pump(stderr, &tx, ProcessEvent::Stderr),
                wait
            );
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
