//! The command that starts the backend, natively or in WSL.

use std::process::Command;
use std::sync::OnceLock;

use vet_host::backend::{Spawn, Step, spawn_command, wsl_distro};
use vet_host::login_path;

/// The user's login-shell PATH, which the backend (and so every program it runs) gets in place of
/// the minimal one of an app started from the Finder.
static LOGIN_PATH: OnceLock<String> = OnceLock::new();

/// Asks the login shell for its PATH, before the backend starts. Keeps the current PATH when the
/// shell cannot tell. Nothing to do on Windows.
pub fn resolve_login_path() {
    if cfg!(windows) {
        return;
    }
    match login_path::from_login_shell(&login_path::default_shell(), login_path::TIMEOUT) {
        Ok(path) => {
            let _ = LOGIN_PATH.set(path);
        }
        Err(message) => tracing::warn!(%message, "keeping the PATH this process started with"),
    }
}

/// Makes the backend exit when this process dies (it holds the child's stdin open).
const EXIT_FLAG: &str = "--exit-on-stdin-close";

/// The command that starts the backend, and the binary path it runs.
///
/// - `VET_BACKEND`: backend binary; defaults to the workspace's `vet-backend` build.
/// - `VET_WSL_DISTRO` (Windows only): run it in that WSL2 distro via `wsl.exe`
///   (the split topology). `VET_BACKEND` is then required and must be a Linux
///   path inside the distro.
fn backend_command() -> Result<(Command, String), String> {
    let configured = std::env::var("VET_BACKEND").ok();
    let (command, binary) = match wsl_distro() {
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
            (command, format!("{binary} (in WSL distro {distro})"))
        }
        _ => {
            let binary = configured.unwrap_or_else(default_backend);
            if !std::path::Path::new(&binary).exists() {
                return Err(format!(
                    "backend binary not found at {binary}; run `pnpm backend` in ui/ or set VET_BACKEND"
                ));
            }
            let mut command = Command::new(&binary);
            command.arg(EXIT_FLAG);
            if let Some(path) = LOGIN_PATH.get() {
                command.env("PATH", path);
            }
            (command, binary)
        }
    };
    Ok((command, binary))
}

fn default_backend() -> String {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let exe = if cfg!(windows) {
        "vet-backend.exe"
    } else {
        "vet-backend"
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join(exe);
    path.canonicalize().unwrap_or(path).to_string_lossy().into_owned()
}

/// How the app starts its backend.
pub fn spawner() -> Box<Spawn> {
    Box::new(|report| {
        report(Step::StartingBackend);
        let (command, binary) = backend_command()?;
        spawn_command(command, &binary, report)
    })
}
