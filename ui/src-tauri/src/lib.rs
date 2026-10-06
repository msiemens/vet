//! Tauri host (SPEC §4.1): spawns the backend, reads the `Ready` line it prints
//! (port and per-launch token), and gives the webview that address. The
//! webview then talks to the backend directly over the authenticated WebSocket.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Wry};
use vet_protocol::Ready;

/// A first run without `pnpm backend` fetches the corpus before printing Ready.
const READY_TIMEOUT: Duration = Duration::from_secs(180);

struct Backend {
    ready: Ready,
    /// The repository named on the command line or in `VET_REPO`; none shows the start screen.
    repo: Option<String>,
    child: Mutex<Child>,
}

/// What the webview needs to connect: the backend's `Ready` line and the repository to open.
#[derive(serde::Serialize)]
struct Connection {
    #[serde(flatten)]
    ready: Ready,
    repo: Option<String>,
}

/// The backend runs in WSL, so Windows paths mean nothing to it.
fn wsl_distro() -> Option<String> {
    std::env::var("VET_WSL_DISTRO").ok().filter(|_| cfg!(windows))
}

/// The repository to review: the first command-line argument that is not a flag, else
/// `VET_REPO`. The benchmark opens `vet-mock`, which ignores it, without asking.
fn repo_path() -> Option<String> {
    pick_repo(std::env::args().skip(1), std::env::var("VET_REPO").ok())
        .or_else(|| bench_requested().then(|| ".".to_owned()))
}

fn pick_repo(mut args: impl Iterator<Item = String>, env: Option<String>) -> Option<String> {
    args.find(|arg| !arg.starts_with('-'))
        .or(env.filter(|value| !value.is_empty()))
}

/// The backend-side path of a folder the native dialog returned. In the WSL split the
/// dialog shows `\\wsl.localhost\<distro>\…` (or `\\wsl$\<distro>\…`), which is the Linux
/// path `/…` inside that distro; anything else cannot be opened by the backend.
fn backend_path(picked: &str, distro: Option<&str>) -> Result<String, String> {
    let Some(distro) = distro else {
        return Ok(picked.to_owned());
    };
    let normalized = picked.replace('\\', "/");
    let rest = normalized
        .strip_prefix("//wsl.localhost/")
        .or_else(|| normalized.strip_prefix("//wsl$/"));
    match rest {
        Some(rest) => {
            let (from, path) = rest.split_once('/').unwrap_or((rest, ""));
            if from.eq_ignore_ascii_case(distro) {
                Ok(format!("/{path}"))
            } else {
                Err(format!(
                    "{picked} is in the WSL distro {from}, but the backend runs in {distro}; pick a folder inside {distro}"
                ))
            }
        }
        None => Err(format!(
            "{picked} is not inside the WSL distro {distro}, where the backend runs (a Windows drive is not visible to it); pick a folder under \\\\wsl.localhost\\{distro}"
        )),
    }
}

const RECENT_LIMIT: usize = 10;

/// `path` first, without duplicates, at most `RECENT_LIMIT` entries.
fn with_recent(mut list: Vec<String>, path: String) -> Vec<String> {
    list.retain(|entry| *entry != path);
    list.insert(0, path);
    list.truncate(RECENT_LIMIT);
    list
}

/// Recently opened repositories as the backend sees their paths, in the app config directory.
fn recent_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("recent-repositories.json"))
}

fn read_recent(app: &tauri::AppHandle) -> Vec<String> {
    recent_file(app)
        .ok()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_recent(app: &tauri::AppHandle, list: &[String]) -> Result<(), String> {
    let file = recent_file(app)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))
}

#[tauri::command]
fn recent_repositories(app: tauri::AppHandle) -> Vec<String> {
    read_recent(&app)
}

#[tauri::command]
fn remember_repository(app: tauri::AppHandle, path: String) -> Result<(), String> {
    write_recent(&app, &with_recent(read_recent(&app), path))
}

#[tauri::command]
fn forget_repository(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let mut list = read_recent(&app);
    list.retain(|entry| *entry != path);
    write_recent(&app, &list)
}

/// The native folder dialog; `None` if the user cancelled.
#[tauri::command]
async fn pick_repository(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(folder) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = folder.into_path().map_err(|e| e.to_string())?;
    backend_path(&path.to_string_lossy(), wsl_distro().as_deref()).map(Some)
}

/// The native file dialog (a language server's executable); `None` if the user cancelled.
#[tauri::command]
async fn pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(file) = app.dialog().file().blocking_pick_file() else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    backend_path(&path.to_string_lossy(), wsl_distro().as_deref()).map(Some)
}

/// One entry of the main menu the UI defines (`MenuItem` in `ui/src/lib/ui/menuItem.ts`).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MenuEntry {
    id: String,
    label: String,
    /// Secondary text; a native menu has no column for it, so it follows the label.
    detail: Option<String>,
    /// An accelerator such as `Cmd+,`.
    shortcut: Option<String>,
    enabled: Option<bool>,
    checked: Option<bool>,
    #[serde(default)]
    separator: bool,
    children: Option<Vec<MenuEntry>>,
}

fn submenu(app: &AppHandle, label: &str, enabled: bool, entries: &[MenuEntry]) -> tauri::Result<Submenu<Wry>> {
    let mut builder = SubmenuBuilder::new(app, label).enabled(enabled);
    for entry in entries {
        if entry.separator {
            builder = builder.separator();
        } else if let Some(children) = &entry.children {
            builder = builder.item(&submenu(app, &entry.label, entry.enabled.unwrap_or(true), children)?);
        } else {
            let text = match &entry.detail {
                Some(detail) => format!("{}  —  {detail}", entry.label),
                None => entry.label.clone(),
            };
            let enabled = entry.enabled.unwrap_or(true);
            if let Some(checked) = entry.checked {
                let mut item = CheckMenuItemBuilder::with_id(&entry.id, text)
                    .checked(checked)
                    .enabled(enabled);
                if let Some(shortcut) = &entry.shortcut {
                    item = item.accelerator(shortcut);
                }
                builder = builder.item(&item.build(app)?);
            } else {
                let mut item = MenuItemBuilder::with_id(&entry.id, text).enabled(enabled);
                if let Some(shortcut) = &entry.shortcut {
                    item = item.accelerator(shortcut);
                }
                builder = builder.item(&item.build(app)?);
            }
        }
    }
    builder.build()
}

/// Replaces the menu bar (macOS) with the UI's definition. The Edit menu is the host's: without
/// it the webview does not get ⌘C, ⌘V and ⌘Z.
#[tauri::command]
fn set_menu(app: AppHandle, items: Vec<MenuEntry>) -> Result<(), String> {
    let build = || -> tauri::Result<()> {
        let mut bar = MenuBuilder::new(&app);
        for (i, menu) in items.iter().enumerate() {
            let children = menu.children.as_deref().unwrap_or_default();
            bar = bar.item(&submenu(&app, &menu.label, menu.enabled.unwrap_or(true), children)?);
            // After the app menu and File.
            if i == 1 {
                let edit = SubmenuBuilder::new(&app, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                bar = bar.item(&edit);
            }
        }
        app.set_menu(bar.build()?)?;
        Ok(())
    };
    build().map_err(|e| e.to_string())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
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
///   `vet-mock` when `VET_BENCH=1` (the benchmark needs the spike's corpus).
/// - `VET_WSL_DISTRO` (Windows only): run it in that WSL2 distro via `wsl.exe`
///   (the split topology). `VET_BACKEND` is then required and must be a Linux
///   path inside the distro.
fn backend_command() -> Result<(Command, String), String> {
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
            let binary = configured.unwrap_or_else(default_backend);
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

fn default_backend() -> String {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let name = if bench_requested() { "vet-mock" } else { "vet-backend" };
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

fn start_backend() -> Result<Backend, String> {
    let (mut command, binary) = backend_command()?;
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
            repo: repo_path(),
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
fn backend_connection(backend: tauri::State<Backend>) -> Connection {
    Connection {
        ready: backend.ready.clone(),
        repo: backend.repo.clone(),
    }
}

/// Benchmark mode (SPEC §12.0): `VET_BENCH=1` runs it on startup and exits with the verdict.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchConfig {
    auto: bool,
    scroll_seconds: u32,
    topology: &'static str,
}

fn bench_requested() -> bool {
    std::env::var("VET_BENCH").is_ok_and(|v| v == "1")
}

#[tauri::command]
fn bench_config() -> BenchConfig {
    BenchConfig {
        auto: bench_requested(),
        scroll_seconds: std::env::var("VET_BENCH_SCROLL_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20),
        topology: if wsl_distro().is_some() { "wsl-split" } else { "native" },
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
fn save_results(app: tauri::AppHandle, markdown: String, json: String, ok: bool) -> Result<(), String> {
    let out = std::env::var("VET_BENCH_OUT").unwrap_or_else(|_| "bench-results".to_owned());
    std::fs::write(format!("{out}.md"), markdown).map_err(|e| format!("cannot write {out}.md: {e}"))?;
    std::fs::write(format!("{out}.json"), json).map_err(|e| format!("cannot write {out}.json: {e}"))?;
    if bench_config().auto {
        app.exit(i32::from(!ok));
    }
    Ok(())
}

/// How long one activation of the app-drawn title bar may take before the plugin restores the
/// native frame. Now and then its page handshake gets lost and an attempt only ends here; a new
/// attempt challenges the then-loaded page afresh.
const DECORATION_TIMEOUT: Duration = Duration::from_secs(4);
const DECORATION_ATTEMPTS: u32 = 3;
/// The window shows by then even while the app-drawn title bar is still activating.
const SHOW_WITHIN: Duration = Duration::from_secs(2);

/// Replaces the native title bar with the app header (window controls drawn by
/// tauri-plugin-decoration, native traffic lights on macOS) and shows the window. If that
/// fails, the native frame is restored and shown instead. Returns "custom" or "native".
#[tauri::command]
async fn activate_and_show(window: tauri::WebviewWindow) -> Result<&'static str, String> {
    use tauri_plugin_decoration::WebviewWindowExt;

    async fn native(window: &tauri::WebviewWindow, cause: impl std::fmt::Display) -> Result<&'static str, String> {
        eprintln!("app-drawn title bar unavailable, using the native frame: {cause}");
        let restored = window.restore_decoration().await;
        window
            .show()
            .map_err(|e| format!("{cause}; showing the native frame failed: {e}"))?;
        restored
            .map(|()| "native")
            .map_err(|e| format!("{cause}; restoring the native frame failed: {e}"))
    }

    // The window shows while activation is still pending; the app-drawn title bar replaces the
    // frame once it comes.
    let shown = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_WITHIN);
        if let Err(e) = shown.show() {
            eprintln!("cannot show the window: {e}");
        }
    });
    let mut attempt = 1;
    while let Err(e) = window.activate_decoration().await {
        if attempt == DECORATION_ATTEMPTS {
            return native(&window, e).await;
        }
        eprintln!("app-drawn title bar, attempt {attempt} of {DECORATION_ATTEMPTS} failed: {e}");
        attempt += 1;
    }
    // Centres the traffic lights in the 40px header: the plugin centres them in a titlebar of
    // their own height (14pt) plus `y`, so `y` is what makes that titlebar 40pt tall.
    #[cfg(target_os = "macos")]
    if let Err(e) = window.set_traffic_lights_inset(16.0, 26.0).await {
        return native(&window, e).await;
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("VET_DEVTOOLS").is_some() {
        window.open_devtools();
    }
    match window.show() {
        Ok(()) => Ok("custom"),
        Err(e) => native(&window, e).await,
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_decoration::builder()
                .activation_timeout(DECORATION_TIMEOUT)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        // Items of the native menu go to the UI, which owns what they do.
        .on_menu_event(|app, event| {
            if let Err(e) = app.emit("vet-menu", event.id().0.clone()) {
                eprintln!("cannot forward menu event: {e}");
            }
        })
        .setup(|app| {
            app.manage(start_backend()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            activate_and_show,
            backend_connection,
            bench_config,
            forget_repository,
            pick_file,
            pick_repository,
            quit,
            recent_repositories,
            remember_repository,
            renderer_memory,
            save_results,
            set_menu
        ])
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

#[cfg(test)]
mod tests {
    use super::{MenuEntry, backend_path, pick_repo, with_recent};

    #[test]
    fn menu_entries_read_the_uis_definition() {
        let json = r#"[{"id":"file","label":"File","children":[
            {"id":"-","label":"","separator":true},
            {"id":"open","label":"Open","shortcut":"Cmd+O","enabled":false,"checked":true,"detail":"/x"}]}]"#;
        let items: Vec<MenuEntry> = serde_json::from_str(json).unwrap();
        let children = items[0].children.as_deref().unwrap();
        assert!(children[0].separator && children[0].shortcut.is_none());
        assert_eq!(children[1].shortcut.as_deref(), Some("Cmd+O"));
        assert_eq!((children[1].enabled, children[1].checked), (Some(false), Some(true)));
    }

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
    fn wsl_paths_become_linux_paths() {
        let wsl = Some("NixOS");
        assert_eq!(
            backend_path(r"\\wsl.localhost\NixOS\home\me\vet", wsl).as_deref(),
            Ok("/home/me/vet")
        );
        assert_eq!(backend_path(r"\\wsl$\nixos\home\me", wsl).as_deref(), Ok("/home/me"));
        assert_eq!(backend_path(r"\\wsl.localhost\NixOS", wsl).as_deref(), Ok("/"));
        assert_eq!(backend_path("/home/me", None).as_deref(), Ok("/home/me"));
    }

    #[test]
    fn paths_the_wsl_backend_cannot_see_are_refused() {
        let wsl = Some("NixOS");
        let other = backend_path(r"\\wsl.localhost\Ubuntu\home", wsl).unwrap_err();
        assert!(other.contains("Ubuntu") && other.contains("NixOS"), "{other}");
        let drive = backend_path(r"C:\src\vet", wsl).unwrap_err();
        assert!(drive.contains("Windows drive"), "{drive}");
    }

    #[test]
    fn recent_list_is_most_recent_first_without_duplicates_and_capped() {
        let list: Vec<String> = (0..10).map(|i| format!("/r{i}")).collect();
        let list = with_recent(list, "/r5".to_owned());
        assert_eq!(list[0], "/r5");
        assert_eq!(list.len(), 10);
        assert_eq!(list.iter().filter(|p| *p == "/r5").count(), 1);
        assert_eq!(with_recent(list, "/new".to_owned()).len(), 10);
    }
}
