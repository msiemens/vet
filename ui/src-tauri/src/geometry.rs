//! Where the main window sits. One window switches between the welcome screen and a project and
//! keeps a position, size and maximized state for each; both are remembered across launches in
//! the app config directory. A saved geometry on a display that is gone gives way to the
//! default, centred on the display the window is on.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, Window, WindowEvent};

use vet_host::backend::repo_path;
use vet_host::files::write_atomic;

/// The welcome screen's size in logical pixels.
const WELCOME_SIZE: (f64, f64) = (960.0, 620.0);
const FILE: &str = "window-geometry.json";
/// The main window's label.
pub const MAIN: &str = "main";
/// A saved window is on a display when the middle of its title bar, this far below its top, is.
const TITLE_BAR_PROBE: i64 = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Welcome,
    Project,
}

/// A rectangle in physical pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl Rect {
    fn contains(&self, x: i64, y: i64) -> bool {
        let (left, top) = (i64::from(self.x), i64::from(self.y));
        (left..left + i64::from(self.width)).contains(&x) && (top..top + i64::from(self.height)).contains(&y)
    }
}

/// A window's geometry: `rect` is its position and inner size while not maximized, `scale` the
/// scale factor of the display it was on.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
struct Placement {
    rect: Rect,
    scale: f64,
    maximized: bool,
}

/// What a window looks like right now; a minimized or full-screen window has none worth keeping.
#[derive(Clone, Copy)]
struct Snapshot {
    rect: Rect,
    scale: f64,
    maximized: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Display {
    /// The part of the display windows can use (without the taskbar or the dock).
    area: Rect,
    scale: f64,
}

impl From<&Monitor> for Display {
    fn from(monitor: &Monitor) -> Self {
        let area = monitor.work_area();
        Self {
            area: Rect {
                x: area.position.x,
                y: area.position.y,
                width: area.size.width,
                height: area.size.height,
            },
            scale: monitor.scale_factor(),
        }
    }
}

/// The geometry of each mode, as last seen.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    welcome: Option<Placement>,
    project: Option<Placement>,
}

impl Saved {
    fn slot(&mut self, mode: Mode) -> &mut Option<Placement> {
        match mode {
            Mode::Welcome => &mut self.welcome,
            Mode::Project => &mut self.project,
        }
    }

    /// A maximized window keeps the geometry it will return to.
    fn observe(&mut self, mode: Mode, now: Snapshot) {
        let slot = self.slot(mode);
        *slot = Some(match (*slot, now.maximized) {
            (Some(before), true) => Placement {
                maximized: true,
                ..before
            },
            _ => Placement {
                rect: now.rect,
                scale: now.scale,
                maximized: now.maximized,
            },
        });
    }

    fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "ignoring the saved window geometry");
            Self::default()
        })
    }
}

/// What to make the window: a rect in physical pixels and whether to maximize it afterwards.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Target {
    rect: Rect,
    maximized: bool,
}

/// The saved geometry if its display is still there, else `default` (logical pixels) centred on
/// display `home`. None without displays.
fn target(saved: Option<Placement>, default: (f64, f64), displays: &[Display], home: usize) -> Option<Target> {
    if let Some(saved) = saved {
        let r = saved.rect;
        let probe = (
            i64::from(r.x) + i64::from(r.width) / 2,
            i64::from(r.y) + TITLE_BAR_PROBE,
        );
        if let Some(display) = displays.iter().find(|d| d.area.contains(probe.0, probe.1)) {
            let ratio = display.scale / saved.scale;
            let sized = Rect {
                width: (f64::from(r.width) * ratio).round() as u32,
                height: (f64::from(r.height) * ratio).round() as u32,
                ..r
            };
            return Some(Target {
                rect: fit(sized, display.area),
                maximized: saved.maximized,
            });
        }
    }
    let display = displays.get(home).or(displays.first())?;
    let width = ((default.0 * display.scale).round() as u32).min(display.area.width);
    let height = ((default.1 * display.scale).round() as u32).min(display.area.height);
    let centred = |start: i32, room: u32, size: u32| start + ((room - size) / 2) as i32;
    Some(Target {
        rect: Rect {
            x: centred(display.area.x, display.area.width, width),
            y: centred(display.area.y, display.area.height, height),
            width,
            height,
        },
        maximized: false,
    })
}

/// `rect`, shrunk and moved to lie within `area`.
fn fit(rect: Rect, area: Rect) -> Rect {
    let width = rect.width.min(area.width);
    let height = rect.height.min(area.height);
    let within = |at: i32, start: i32, room: u32, size: u32| {
        let last = i64::from(start) + i64::from(room) - i64::from(size);
        i64::from(at).clamp(i64::from(start), last) as i32
    };
    Rect {
        x: within(rect.x, area.x, area.width, width),
        y: within(rect.y, area.y, area.height, height),
        width,
        height,
    }
}

struct State {
    mode: Mode,
    saved: Saved,
}

pub struct Windows {
    state: Mutex<State>,
    file: Option<PathBuf>,
    /// The project window's size in logical pixels when nothing is saved: the one the window
    /// configuration asks for.
    project_size: (f64, f64),
}

impl Windows {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn default_size(&self, mode: Mode) -> (f64, f64) {
        match mode {
            Mode::Welcome => WELCOME_SIZE,
            Mode::Project => self.project_size,
        }
    }

    /// Writes the geometry of both modes down. Nothing is lost if this fails.
    fn persist(&self) {
        let Some(file) = &self.file else { return };
        let text = match serde_json::to_string_pretty(&self.lock().saved) {
            Ok(text) => text,
            Err(e) => return tracing::warn!(error = %e, "cannot encode the window geometry"),
        };
        if let Err(e) = write_atomic(file, &text) {
            tracing::warn!(file = %file.display(), error = %e, "cannot save the window geometry");
        }
    }
}

fn snapshot(window: &tauri::WebviewWindow) -> Option<Snapshot> {
    if window.is_minimized().ok()? || window.is_fullscreen().ok()? {
        return None;
    }
    let position = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    Some(Snapshot {
        rect: Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
        scale: window.scale_factor().ok()?,
        maximized: window.is_maximized().ok()?,
    })
}

/// The usable displays, and the index of the one the window is on: its own once it is visible,
/// else the one under the cursor, else the first.
fn displays(window: &tauri::WebviewWindow) -> (Vec<Display>, usize) {
    let monitors = window.available_monitors().unwrap_or_default();
    let displays: Vec<Display> = monitors.iter().map(Display::from).collect();
    let current = if window.is_visible().unwrap_or(false) {
        window.current_monitor().ok().flatten()
    } else {
        let at_cursor = window
            .cursor_position()
            .ok()
            .and_then(|p| window.monitor_from_point(p.x, p.y).ok().flatten());
        at_cursor.or_else(|| window.current_monitor().ok().flatten())
    };
    let home = current
        .map(|m| Display::from(&m))
        .and_then(|home| displays.iter().position(|d| *d == home))
        .unwrap_or(0);
    (displays, home)
}

/// Moves and sizes the window in one go. Run on the main thread (as sync commands and the setup
/// are), the calls reach the system before it draws a frame in between.
fn apply(window: &tauri::WebviewWindow, target: Target) -> tauri::Result<()> {
    if window.is_maximized()? {
        window.unmaximize()?;
    }
    window.set_position(PhysicalPosition::new(target.rect.x, target.rect.y))?;
    window.set_size(PhysicalSize::new(target.rect.width, target.rect.height))?;
    if target.maximized {
        window.maximize()?;
    }
    Ok(())
}

/// Puts the hidden main window where its first mode wants it: a project when the shell is
/// given one to open, else the welcome screen.
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(MAIN) else {
        return Ok(());
    };
    let mode = if repo_path().is_some() {
        Mode::Project
    } else {
        Mode::Welcome
    };
    let file = app.path().app_config_dir().ok().map(|dir| dir.join(FILE));
    let saved = file
        .as_ref()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .map_or_else(Saved::default, |text| Saved::parse(&text));
    let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
    let windows = Windows {
        state: Mutex::new(State { mode, saved }),
        file,
        project_size: (size.width, size.height),
    };
    let (displays, home) = displays(&window);
    let wanted = *windows.lock().saved.slot(mode);
    if let Some(target) = target(wanted, windows.default_size(mode), &displays, home) {
        apply(&window, target)?;
    }
    app.manage(windows);
    Ok(())
}

/// Keeps the main window's geometry current.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN || !matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
        return;
    }
    let (Some(windows), Some(webview)) = (window.try_state::<Windows>(), window.get_webview_window(MAIN)) else {
        return;
    };
    if let Some(now) = snapshot(&webview) {
        let mut state = windows.lock();
        let mode = state.mode;
        state.saved.observe(mode, now);
    }
}

/// Writes the geometry down; the app calls this on its way out.
pub fn persist(app: &AppHandle) {
    if let Some(windows) = app.try_state::<Windows>() {
        windows.persist();
    }
}

/// Switches the window between the welcome screen and a project: the geometry of the mode it
/// leaves is kept, the one of the mode it enters is applied. The page calls this before it
/// changes what it shows, so the content never appears in the other mode's window.
#[tauri::command]
pub fn set_window_mode(window: tauri::WebviewWindow, windows: tauri::State<Windows>, mode: Mode) -> Result<(), String> {
    if windows.lock().mode == mode {
        return Ok(());
    }
    let leaving = snapshot(&window);
    let (displays, home) = displays(&window);
    let wanted = {
        let mut state = windows.lock();
        if let Some(now) = leaving {
            let before = state.mode;
            state.saved.observe(before, now);
        }
        state.mode = mode;
        *state.saved.slot(mode)
    };
    if let Some(target) = target(wanted, windows.default_size(mode), &displays, home) {
        apply(&window, target).map_err(|e| format!("cannot move the window: {e}"))?;
    }
    windows.persist();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    const fn display(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Display {
        Display {
            area: rect(x, y, width, height),
            scale,
        }
    }

    const fn placement(r: Rect, scale: f64, maximized: bool) -> Placement {
        Placement {
            rect: r,
            scale,
            maximized,
        }
    }

    fn snapshot(r: Rect, maximized: bool) -> Snapshot {
        Snapshot {
            rect: r,
            scale: 1.0,
            maximized,
        }
    }

    const MAIN_DISPLAY: Display = display(0, 0, 1920, 1080, 1.0);
    const SECOND: Display = display(1920, 0, 2560, 1440, 2.0);

    #[test]
    fn nothing_saved_centres_the_default_on_the_home_display() {
        let wanted = target(None, (960.0, 620.0), &[MAIN_DISPLAY, SECOND], 1).unwrap();
        // Logical 960x620 at scale 2, centred in the second display.
        assert_eq!(
            wanted.rect,
            rect(1920 + (2560 - 1920) / 2, (1440 - 1240) / 2, 1920, 1240)
        );
        assert!(!wanted.maximized);
    }

    #[test]
    fn a_saved_geometry_on_a_present_display_comes_back_as_it_was() {
        let saved = placement(rect(200, 100, 1000, 700), 1.0, true);
        let wanted = target(Some(saved), (960.0, 620.0), &[MAIN_DISPLAY, SECOND], 0).unwrap();
        assert_eq!(wanted.rect, rect(200, 100, 1000, 700));
        assert!(wanted.maximized);
    }

    #[test]
    fn a_saved_geometry_whose_display_is_gone_falls_back_to_the_default_not_maximized() {
        let saved = placement(rect(2400, 100, 1600, 1000), 2.0, true);
        let wanted = target(Some(saved), (960.0, 620.0), &[MAIN_DISPLAY], 0).unwrap();
        assert_eq!(wanted.rect, rect(480, 230, 960, 620));
        assert!(!wanted.maximized);
    }

    #[test]
    fn a_saved_geometry_is_rescaled_when_its_display_changed_scale() {
        let saved = placement(rect(100, 100, 1000, 600), 1.0, false);
        let wanted = target(Some(saved), (960.0, 620.0), &[display(0, 0, 3000, 2000, 2.0)], 0).unwrap();
        assert_eq!((wanted.rect.width, wanted.rect.height), (2000, 1200));
    }

    #[test]
    fn a_saved_geometry_larger_than_its_display_is_shrunk_and_pulled_inside() {
        let saved = placement(rect(700, 700, 1600, 900), 1.0, false);
        let wanted = target(Some(saved), (960.0, 620.0), &[MAIN_DISPLAY], 0).unwrap();
        assert_eq!(wanted.rect, rect(320, 180, 1600, 900));
        let huge = placement(rect(-1000, 10, 5000, 4000), 1.0, false);
        let wanted = target(Some(huge), (960.0, 620.0), &[MAIN_DISPLAY], 0).unwrap();
        assert_eq!(wanted.rect, rect(0, 0, 1920, 1080));
    }

    #[test]
    fn a_default_larger_than_the_display_fills_it() {
        let wanted = target(None, (1400.0, 900.0), &[display(0, 0, 1280, 800, 1.0)], 0).unwrap();
        assert_eq!(wanted.rect, rect(0, 0, 1280, 800));
    }

    #[test]
    fn without_displays_there_is_nothing_to_place() {
        assert_eq!(target(None, (960.0, 620.0), &[], 0), None);
    }

    #[test]
    fn maximizing_keeps_the_geometry_the_window_returns_to() {
        let mut saved = Saved::default();
        saved.observe(Mode::Project, snapshot(rect(50, 60, 1200, 800), false));
        saved.observe(Mode::Project, snapshot(rect(0, 0, 1920, 1080), true));
        assert_eq!(saved.project, Some(placement(rect(50, 60, 1200, 800), 1.0, true)));
        saved.observe(Mode::Project, snapshot(rect(70, 80, 1100, 700), false));
        assert_eq!(saved.project, Some(placement(rect(70, 80, 1100, 700), 1.0, false)));
    }

    #[test]
    fn the_modes_keep_their_geometry_apart() {
        let mut saved = Saved::default();
        saved.observe(Mode::Project, snapshot(rect(0, 0, 1400, 900), true));
        saved.observe(Mode::Welcome, snapshot(rect(300, 200, 960, 620), false));
        assert!(saved.project.unwrap().maximized);
        assert!(!saved.welcome.unwrap().maximized);
    }

    #[test]
    fn geometry_persisted_to_the_file_is_read_back_after_a_restart_and_a_damaged_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config").join(FILE);
        let windows = Windows {
            state: Mutex::new(State {
                mode: Mode::Welcome,
                saved: Saved::default(),
            }),
            file: Some(file.clone()),
            project_size: (1200.0, 800.0),
        };
        windows
            .lock()
            .saved
            .observe(Mode::Welcome, snapshot(rect(300, 200, 960, 620), false));
        windows.persist();
        windows
            .lock()
            .saved
            .observe(Mode::Welcome, snapshot(rect(310, 210, 900, 600), false));
        windows.persist();

        let restarted = Saved::parse(&std::fs::read_to_string(&file).unwrap());
        assert_eq!(
            restarted.welcome,
            Some(placement(rect(310, 210, 900, 600), 1.0, false)),
            "the later write replaced the earlier"
        );

        std::fs::write(&file, "{\"welcome\": 3").unwrap();
        let damaged = Saved::parse(&std::fs::read_to_string(&file).unwrap());
        assert!(damaged.welcome.is_none() && damaged.project.is_none());
    }
}
