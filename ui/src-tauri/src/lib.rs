//! Tauri host (SPEC §4.1): spawns the backend, reads the `Ready` line it prints
//! (port and per-launch token), and gives the webview that address. The
//! webview then talks to the backend directly over the authenticated WebSocket.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tauri::{Manager, RunEvent};
use vet_protocol::Ready;

/// A first run without `pnpm backend` fetches the corpus before printing Ready.
const READY_TIMEOUT: Duration = Duration::from_secs(180);

struct Backend {
    ready: Ready,
    child: Mutex<Child>,
}

/// Makes the backend exit when this process dies (it holds the child's stdin open).
const EXIT_FLAG: &str = "--exit-on-stdin-close";

/// Environment the corpus loader reads; forwarded into WSL through `WSLENV`.
const CORPUS_ENV: [&str; 4] = ["VET_CORPUS_URL", "VET_CORPUS_REPO", "VET_CORPUS_BASE", "VET_CORPUS_HEAD"];

/// The command that starts the backend, and the binary path it runs.
///
/// - `VET_BACKEND`: backend binary; defaults to the workspace's `vet-mock` build.
/// - `VET_WSL_DISTRO` (Windows only): run it in that WSL2 distro via `wsl.exe`
///   (the split topology). `VET_BACKEND` is then required and must be a Linux
///   path inside the distro.
fn backend_command() -> Result<(Command, String), String> {
    let configured = std::env::var("VET_BACKEND").ok();
    let (mut command, binary) = match std::env::var("VET_WSL_DISTRO") {
        Ok(distro) if cfg!(windows) => {
            let binary = configured.ok_or("VET_WSL_DISTRO is set: set VET_BACKEND to the backend's Linux path inside WSL, e.g. /home/me/vet/target/debug/vet-mock")?;
            let mut command = Command::new("wsl.exe");
            // `--exec` skips the login environment, so PATH lacks e.g. NixOS's per-user profile (git).
            command.args(["--distribution", &distro, "--exec", "/bin/sh", "-lc", r#"exec "$0" "$@""#, &binary, EXIT_FLAG]);
            // Env vars cross into WSL only when listed in WSLENV.
            let mut wslenv = std::env::var("WSLENV").unwrap_or_default();
            for name in CORPUS_ENV.iter().filter(|name| std::env::var_os(name).is_some()) {
                wslenv = if wslenv.is_empty() { (*name).to_owned() } else { format!("{wslenv}:{name}") };
            }
            command.env("WSLENV", wslenv);
            (command, format!("{binary} (in WSL distro {distro})"))
        }
        _ => {
            let binary = configured.unwrap_or_else(default_backend);
            if !std::path::Path::new(&binary).exists() {
                return Err(format!("backend binary not found at {binary}; run `pnpm backend` in ui/ or set VET_BACKEND"));
            }
            let mut command = Command::new(&binary);
            command.arg(EXIT_FLAG);
            (command, binary)
        }
    };
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    Ok((command, binary))
}

fn default_backend() -> String {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let exe = if cfg!(windows) { "vet-mock.exe" } else { "vet-mock" };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target").join(profile).join(exe);
    path.canonicalize().unwrap_or(path).to_string_lossy().into_owned()
}

fn start_backend() -> Result<Backend, String> {
    let (mut command, binary) = backend_command()?;
    let mut child = command.spawn().map_err(|error| format!("cannot start the backend {binary}: {error}"))?;
    let stdout = child.stdout.take().ok_or("backend has no stdout")?;

    // Read on a thread so the wait can time out.
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    let line = match receiver.recv_timeout(READY_TIMEOUT) {
        Ok(Ok(line)) if line.is_empty() => return Err(abort(child, format!("the backend {binary} exited before printing its Ready line; see its output above"))),
        Ok(Ok(line)) => line,
        Ok(Err(error)) => return Err(abort(child, format!("cannot read the backend's Ready line: {error}"))),
        Err(_) => return Err(abort(child, "the backend did not print its Ready line in time".into())),
    };
    match serde_json::from_str::<Ready>(&line) {
        Ok(ready) => Ok(Backend { ready, child: Mutex::new(child) }),
        Err(error) => Err(abort(child, format!("malformed Ready line {line:?}: {error}"))),
    }
}

fn abort(mut child: Child, message: String) -> String {
    let _ = child.kill();
    message
}

#[tauri::command]
fn backend_connection(backend: tauri::State<Backend>) -> Ready {
    backend.ready.clone()
}

/// Benchmark mode (SPEC §12.0): `VET_BENCH=1` runs it on startup and exits with the verdict.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchConfig {
    auto: bool,
    scroll_seconds: u32,
    topology: &'static str,
}

#[tauri::command]
fn bench_config() -> BenchConfig {
    BenchConfig {
        auto: std::env::var("VET_BENCH").is_ok_and(|v| v == "1"),
        scroll_seconds: std::env::var("VET_BENCH_SCROLL_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(20),
        topology: if cfg!(windows) && std::env::var_os("VET_WSL_DISTRO").is_some() { "wsl-split" } else { "native" },
    }
}

/// Memory of the webview processes: descendants of this process other than the
/// backend's. Not available on macOS, where WebContent runs outside the
/// process tree.
#[tauri::command]
fn renderer_memory(backend: tauri::State<Backend>) -> Option<u64> {
    if cfg!(target_os = "macos") {
        return None;
    }
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let descendants = |root: Pid| {
        let mut found = vec![root];
        loop {
            let before = found.len();
            for (pid, process) in system.processes() {
                if process.parent().is_some_and(|p| found.contains(&p)) && !found.contains(pid) {
                    found.push(*pid);
                }
            }
            if found.len() == before {
                return found;
            }
        }
    };
    let own = descendants(Pid::from_u32(std::process::id()));
    let backend_pid = backend.child.lock().unwrap_or_else(PoisonError::into_inner).id();
    let excluded = descendants(Pid::from_u32(backend_pid));
    let total = own.iter().skip(1).filter(|pid| !excluded.contains(pid)).filter_map(|pid| system.process(*pid)).map(|p| p.memory()).sum();
    Some(total)
}

/// Writes `<VET_BENCH_OUT>.md` and `.json` (default: `bench-results` in the
/// current directory). In auto mode, exits 0 if no metric failed, else 1.
#[tauri::command]
fn save_results(app: tauri::AppHandle, markdown: String, json: String, ok: bool) -> Result<(), String> {
    let out = std::env::var("VET_BENCH_OUT").unwrap_or_else(|_| "bench-results".to_owned());
    std::fs::write(format!("{out}.md"), markdown).map_err(|e| format!("cannot write {out}.md: {e}"))?;
    std::fs::write(format!("{out}.json"), json).map_err(|e| format!("cannot write {out}.json: {e}"))?;
    if bench_config().auto {
        app.exit(i32::from(!ok));
    }
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            app.manage(start_backend()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![backend_connection, bench_config, renderer_memory, save_results])
        .build(tauri::generate_context!())
        .expect("error while building the Vet shell");

    app.run(|handle, event| {
        if let RunEvent::Exit = event
            && let Some(backend) = handle.try_state::<Backend>()
        {
            let mut child = backend.child.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = child.kill();
        }
    });
}
