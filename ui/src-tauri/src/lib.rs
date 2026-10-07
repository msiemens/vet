//! Tauri host (SPEC §4.1): spawns the backend, reads the `Ready` line it prints
//! (port and per-launch token), and gives the webview that address. The
//! webview then talks to the backend directly over the authenticated WebSocket. A splash window
//! shows the startup steps until the main window is shown.

mod backend;
mod bench;
mod dialog;
mod geometry;
mod menu;
mod startup;
mod window;

use tauri::{Emitter, Manager, RunEvent};

use backend::Backend;
use bench::Bench;
use startup::Startup;

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_decoration::builder()
                .activation_timeout(window::DECORATION_TIMEOUT)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        // Items of the native menu go to the UI, which owns what they do.
        .on_menu_event(|app, event| {
            if let Err(e) = app.emit("vet-menu", event.id().0.clone()) {
                eprintln!("cannot forward menu event: {e}");
            }
        })
        .on_window_event(geometry::on_window_event)
        .setup(|app| {
            app.manage(Bench::from_env());
            app.manage(Startup::new());
            geometry::init(app.handle(), &app.state::<Bench>())?;
            startup::show_splash(app.handle());
            startup::launch(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            window::activate_and_show,
            geometry::set_window_mode,
            startup::backend_connection,
            startup::retry_startup,
            startup::startup_progress,
            bench::bench_config,
            dialog::pick_file,
            dialog::pick_repository,
            window::quit,
            bench::renderer_memory,
            bench::save_results,
            menu::set_menu
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Vet shell");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            geometry::persist(handle);
            if let Some(backend) = handle.try_state::<Backend>() {
                backend.kill();
            }
        }
    });
}
