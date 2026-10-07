//! Tauri host (SPEC §4.1): spawns the backend, reads the `Ready` line it prints
//! (port and per-launch token), and gives the webview that address. The
//! webview then talks to the backend directly over the authenticated WebSocket.

mod backend;
mod bench;
mod dialog;
mod menu;
mod recent;
mod window;

use tauri::{Emitter, Manager, RunEvent};

use backend::Backend;
use bench::Bench;

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
        .setup(|app| {
            let bench = Bench::from_env();
            app.manage(backend::start_backend(&bench)?);
            app.manage(bench);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            window::activate_and_show,
            backend::backend_connection,
            bench::bench_config,
            recent::forget_repository,
            dialog::pick_file,
            dialog::pick_repository,
            window::quit,
            recent::recent_repositories,
            recent::remember_repository,
            bench::renderer_memory,
            bench::save_results,
            menu::set_menu
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Vet shell");

    app.run(|handle, event| {
        if let RunEvent::Exit = event
            && let Some(backend) = handle.try_state::<Backend>()
        {
            backend.kill();
        }
    });
}
