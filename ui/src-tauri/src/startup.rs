//! Startup: the backend is started on a thread while the splash window shows which step the
//! shell is in. The main window loads meanwhile and asks for the connection, which waits for
//! the backend; the splash closes when the main window is shown.

use std::sync::{Condvar, Mutex, PoisonError};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::backend::{self, Backend, wsl_distro};
use crate::bench::Bench;

const SPLASH: &str = "splash";
/// Sent to the splash with the new [`Progress`] at every step.
const EVENT: &str = "vet-startup";

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    StartingBackend,
    WaitingForReady,
    /// The backend is up; the main window connects to it and shows.
    LoadingUi,
}

/// Where startup is. A failure keeps the step it happened in.
#[derive(Clone, Serialize)]
pub struct Progress {
    step: Step,
    /// The WSL distro the backend runs in; none for a backend on this machine.
    distro: Option<String>,
    error: Option<String>,
}

impl Progress {
    fn at(step: Step) -> Self {
        Self {
            step,
            distro: wsl_distro(),
            error: None,
        }
    }
}

pub struct Startup {
    progress: Mutex<Progress>,
    changed: Condvar,
}

impl Startup {
    pub fn new() -> Self {
        Self {
            progress: Mutex::new(Progress::at(Step::StartingBackend)),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, app: &AppHandle, progress: Progress) {
        *self.lock() = progress.clone();
        self.changed.notify_all();
        if let Err(e) = app.emit_to(SPLASH, EVENT, progress) {
            eprintln!("cannot report startup progress: {e}");
        }
    }

    pub fn report(&self, app: &AppHandle, step: Step) {
        self.set(app, Progress::at(step));
    }

    fn fail(&self, app: &AppHandle, message: String) {
        let progress = Progress {
            error: Some(message),
            ..self.lock().clone()
        };
        self.set(app, progress);
    }

    /// Back to the first step after a failure; false while startup is running or done.
    fn restart(&self, app: &AppHandle) -> bool {
        if self.lock().error.is_none() {
            return false;
        }
        self.report(app, Step::StartingBackend);
        true
    }

    /// Blocks until the backend is up. A failed start keeps this waiting for a retry.
    fn wait_until_backend_is_up(&self) {
        let mut progress = self.lock();
        while progress.step != Step::LoadingUi {
            progress = self.changed.wait(progress).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Opens the splash. Without it the app starts anyway.
pub fn show_splash(app: &AppHandle) {
    let built = WebviewWindowBuilder::new(app, SPLASH, WebviewUrl::App("splash.html".into()))
        .title("Vet")
        .inner_size(560.0, 340.0)
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .center()
        .build();
    if let Err(e) = built {
        eprintln!("cannot open the splash window: {e}");
    }
}

/// Closes the splash; the main window is shown by then.
pub fn close_splash(app: &AppHandle) {
    if let Some(splash) = app.get_webview_window(SPLASH)
        && let Err(e) = splash.close()
    {
        eprintln!("cannot close the splash window: {e}");
    }
}

/// Starts the backend on a thread; the app keeps it once it is up.
pub fn launch(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let startup = app.state::<Startup>();
        let bench = app.state::<Bench>();
        match backend::start_backend(&bench, |step| startup.report(&app, step)) {
            Ok(backend) => {
                app.manage(backend);
                startup.report(&app, Step::LoadingUi);
            }
            Err(message) => {
                eprintln!("startup failed: {message}");
                startup.fail(&app, message);
            }
        }
    });
}

/// What the splash shows when it loads after the first steps were reported.
#[tauri::command]
pub fn startup_progress(startup: tauri::State<Startup>) -> Progress {
    startup.lock().clone()
}

#[tauri::command]
pub fn retry_startup(app: AppHandle, startup: tauri::State<Startup>) {
    if startup.restart(&app) {
        launch(&app);
    }
}

/// The main window's connection to the backend, once the backend is up.
#[tauri::command]
pub async fn backend_connection(app: AppHandle) -> Result<backend::Connection, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Startup>().wait_until_backend_is_up();
        app.state::<Backend>().connection()
    })
    .await
    .map_err(|e| e.to_string())
}
