//! Showing a web address in the user's browser, and a folder in the file manager. The address comes
//! from the backend (a platform's page for a check), so only web addresses are opened, and of the
//! opener only folders.

use vet_host::backend::wsl_distro;
use vet_host::paths::host_path;
use vet_host::web::web_address;

/// Opens `url` in the default browser.
#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    tauri_plugin_opener::open_url(web_address(&url)?, None::<&str>).map_err(|e| format!("cannot open the browser: {e}"))
}

/// Shows a folder of the backend's machine (its log folder) in the system's file manager.
#[tauri::command]
pub async fn show_folder(path: String) -> Result<(), String> {
    let path = host_path(&path, wsl_distro().as_deref());
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| format!("cannot open the folder: {e}"))
}
