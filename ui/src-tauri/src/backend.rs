//! The backend process: spawning it (natively or in WSL) and reading its `Ready` line.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use vet_protocol::Ready;

use crate::bench::Bench;

/// A first run without `pnpm backend` fetches the corpus before printing Ready.
const READY_TIMEOUT: Duration = Duration::from_secs(180);

pub struct Backend {
    ready: Ready,
    /// The repository named on the command line or in `VET_REPO`; none shows the start screen.
    repo: Option<String>,
    pub child: Mutex<Child>,
}

impl Backend {
    pub fn kill(&self) {
        let _ = self.child.lock().unwrap_or_else(PoisonError::into_inner).kill();
    }
}

/// What the webview needs to connect: the backend's `Ready` line and the repository to open.
#[derive(serde::Serialize)]
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
/// `VET_REPO`. The benchmark opens `vet-mock`, which ignores it, without asking.
fn repo_path(bench: &Bench) -> Option<String> {
    pick_repo(std::env::args().skip(1), std::env::var("VET_REPO").ok())
        .or_else(|| bench.requested().then(|| ".".to_owned()))
}

fn pick_repo(mut args: impl Iterator<Item = String>, env: Option<String>) -> Option<String> {
    args.find(|arg| !arg.starts_with('-'))
        .or(env.filter(|value| !value.is_empty()))
}

/// Makes the backend exit when this process dies (it holds the child's stdin open).
const EXIT_FLAG: &str = "--exit-on-stdin-close";

/// Environment the corpus loader reads; forwarded into WSL through `WSLENV`.
const CORPUS_ENV: [&str; 4] = [
    "VET_CORPUS_URL",
    "VET_CORPUS_REPO",
    "VET_CORPUS_BASE",
    "VET_CORPUS_HEAD",
];

/// The command that starts the backend, and the binary path it runs.
///
/// - `VET_BACKEND`: backend binary; defaults to the workspace's `vet-backend` build, or
///   `vet-mock` when the benchmark runs (it needs the spike's corpus).
/// - `VET_WSL_DISTRO` (Windows only): run it in that WSL2 distro via `wsl.exe`
///   (the split topology). `VET_BACKEND` is then required and must be a Linux
///   path inside the distro.
fn backend_command(bench: &Bench) -> Result<(Command, String), String> {
    let configured = std::env::var("VET_BACKEND").ok();
    let (mut command, binary) = match wsl_distro() {
        Some(distro) => {
            let binary = configured.ok_or("VET_WSL_DISTRO is set: set VET_BACKEND to the backend's Linux path inside WSL, e.g. /home/me/vet/target/debug/vet-backend")?;
            let mut command = Command::new("wsl.exe");
            // `--exec` skips the login environment, so PATH lacks e.g. NixOS's per-user profile (git).
            command.args([
                "--distribution",
                &distro,
                "--exec",
                "/bin/sh",
                "-lc",
                r#"exec "$0" "$@""#,
                &binary,
                EXIT_FLAG,
            ]);
            // Env vars cross into WSL only when listed in WSLENV.
            let mut wslenv = std::env::var("WSLENV").unwrap_or_default();
            for name in CORPUS_ENV.iter().filter(|name| std::env::var_os(name).is_some()) {
                wslenv = if wslenv.is_empty() {
                    (*name).to_owned()
                } else {
                    format!("{wslenv}:{name}")
                };
            }
            command.env("WSLENV", wslenv);
            (command, format!("{binary} (in WSL distro {distro})"))
        }
        _ => {
            let binary = configured.unwrap_or_else(|| default_backend(bench));
            if !std::path::Path::new(&binary).exists() {
                return Err(format!(
                    "backend binary not found at {binary}; run `pnpm backend` in ui/ or set VET_BACKEND"
                ));
            }
            let mut command = Command::new(&binary);
            command.arg(EXIT_FLAG);
            (command, binary)
        }
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    Ok((command, binary))
}

fn default_backend(bench: &Bench) -> String {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let name = if bench.requested() { "vet-mock" } else { "vet-backend" };
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join(exe);
    path.canonicalize().unwrap_or(path).to_string_lossy().into_owned()
}

pub fn start_backend(bench: &Bench) -> Result<Backend, String> {
    let (mut command, binary) = backend_command(bench)?;
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start the backend {binary}: {error}"))?;
    let stdout = child.stdout.take().ok_or("backend has no stdout")?;

    // Read on a thread so the wait can time out.
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = match receiver.recv_timeout(READY_TIMEOUT) {
        Ok(Ok(line)) if line.is_empty() => {
            return Err(abort(
                child,
                format!("the backend {binary} exited before printing its Ready line; see its output above"),
            ));
        }
        Ok(Ok(line)) => line,
        Ok(Err(error)) => return Err(abort(child, format!("cannot read the backend's Ready line: {error}"))),
        Err(_) => return Err(abort(child, "the backend did not print its Ready line in time".into())),
    };
    match serde_json::from_str::<Ready>(&line) {
        Ok(ready) => Ok(Backend {
            ready,
            repo: repo_path(bench),
            child: Mutex::new(child),
        }),
        Err(error) => Err(abort(child, format!("malformed Ready line {line:?}: {error}"))),
    }
}

fn abort(mut child: Child, message: String) -> String {
    let _ = child.kill();
    message
}

#[tauri::command]
pub fn backend_connection(backend: tauri::State<Backend>) -> Connection {
    Connection {
        ready: backend.ready.clone(),
        repo: backend.repo.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::pick_repo;

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
}
