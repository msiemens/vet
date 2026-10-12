//! Tauri host (SPEC §4.1): spawns the backend, reads the `Ready` line it prints
//! (port and per-launch token), and gives the webview that address. The
//! webview then talks to the backend directly over the authenticated WebSocket. A splash window
//! shows the startup steps until the main window is shown.

mod backend;
mod browser;
mod credentials;
mod dialog;
mod geometry;
mod input;
mod menu;
mod startup;
mod window;

use tauri::{Emitter, Manager, RunEvent};

use backend::Backend;
use startup::Startup;

pub fn run() {
    let mut log = vet_log::data_dir().map(|dir| vet_log::init(&dir.join(vet_log::LOGS)));
    let builder = tauri::Builder::default();
    // Until the UI sends its menu the bar is the host's own, not Tauri's default one.
    #[cfg(target_os = "macos")]
    let builder = builder.enable_macos_default_menu(false).menu(menu::startup_menu);
    let app = builder
        .plugin(
            tauri_plugin_decoration::builder()
                .activation_timeout(window::DECORATION_TIMEOUT)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        // An item of a context menu answers the request that popped it up; one of the menu bar goes
        // to the UI, which owns what it does.
        .on_menu_event(|app, event| {
            if menu::answer_context_menu(app, &event.id().0) {
                return;
            }
            if menu::raised_by_window_key(app, &event.id().0) {
                return;
            }
            if let Err(e) = app.emit("vet-menu", event.id().0.clone()) {
                tracing::warn!(error = %e, "cannot forward menu event");
            }
        })
        .on_window_event(geometry::on_window_event)
        .setup(|app| {
            app.manage(menu::ContextMenuChoice::default());
            app.manage(menu::KeyOnlyItems::default());
            app.manage(Startup::new());
            geometry::init(app.handle())?;
            startup::show_splash(app.handle());
            startup::launch(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            window::activate_and_show,
            geometry::set_window_mode,
            startup::backend_connection,
            startup::restart_backend,
            startup::retry_startup,
            startup::startup_progress,
            browser::open_url,
            browser::show_folder,
            credentials::stored_credentials,
            credentials::stored_credential,
            credentials::store_credential,
            credentials::remove_credential,
            dialog::pick_file,
            dialog::pick_repository,
            input::double_click_ms,
            window::quit,
            menu::set_menu,
            menu::show_context_menu
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Vet shell");

    app.run(move |handle, event| {
        if let RunEvent::Exit = event {
            geometry::persist(handle);
            if let Some(backend) = handle.try_state::<Backend>() {
                backend.stop();
            }
            // The process ends after this: the log file takes what is buffered first.
            drop(log.take());
        }
    });
}
