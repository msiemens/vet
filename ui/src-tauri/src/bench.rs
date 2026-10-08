//! Benchmark mode (SPEC §12.0): `VET_BENCH=1` runs it on startup and exits with the
//! verdict; `VET_BENCH=manual` opens the same corpus and leaves starting it to the Dev panel.

use crate::backend::{Backend, wsl_distro};

enum Mode {
    Auto,
    Manual,
}

/// The benchmark settings, read from the environment once at startup.
pub struct Bench {
    mode: Option<Mode>,
    scroll_seconds: u32,
    /// Prefix of the result files.
    out: String,
}

impl Bench {
    pub fn from_env() -> Self {
        let mode = match std::env::var("VET_BENCH").as_deref() {
            Ok("1") => Some(Mode::Auto),
            Ok("manual") => Some(Mode::Manual),
            _ => None,
        };
        Self {
            mode,
            scroll_seconds: std::env::var("VET_BENCH_SCROLL_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            out: std::env::var("VET_BENCH_OUT").unwrap_or_else(|_| "bench-results".to_owned()),
        }
    }

    pub fn requested(&self) -> bool {
        self.mode.is_some()
    }

    fn auto(&self) -> bool {
        matches!(self.mode, Some(Mode::Auto))
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchConfig {
    auto: bool,
    scroll_seconds: u32,
    topology: &'static str,
}

#[tauri::command]
pub fn bench_config(bench: tauri::State<Bench>) -> BenchConfig {
    BenchConfig {
        auto: bench.auto(),
        scroll_seconds: bench.scroll_seconds,
        topology: if wsl_distro().is_some() { "wsl-split" } else { "native" },
    }
}

/// Memory of the webview processes: descendants of this process other than the
/// backend's. Not available on macOS, where WebContent runs outside the
/// process tree.
#[tauri::command]
pub fn renderer_memory(backend: tauri::State<Backend>) -> Option<u64> {
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
    let excluded = backend
        .pid()
        .map_or_else(Vec::new, |pid| descendants(Pid::from_u32(pid)));
    let total = own
        .iter()
        .skip(1)
        .filter(|pid| !excluded.contains(pid))
        .filter_map(|pid| system.process(*pid))
        .map(|p| p.memory())
        .sum();
    Some(total)
}

/// Writes `<VET_BENCH_OUT>.md` and `.json` (default: `bench-results` in the
/// current directory). In auto mode, exits 0 if no metric failed, else 1.
#[tauri::command]
pub fn save_results(
    app: tauri::AppHandle,
    bench: tauri::State<Bench>,
    markdown: String,
    json: String,
    ok: bool,
) -> Result<(), String> {
    let out = &bench.out;
    std::fs::write(format!("{out}.md"), markdown).map_err(|e| format!("cannot write {out}.md: {e}"))?;
    std::fs::write(format!("{out}.json"), json).map_err(|e| format!("cannot write {out}.json: {e}"))?;
    if bench.auto() {
        app.exit(i32::from(!ok));
    }
    Ok(())
}
