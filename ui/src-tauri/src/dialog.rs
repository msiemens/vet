//! The native file dialogs, with their results translated to the backend's paths.

use tauri_plugin_dialog::DialogExt;
use vet_host::backend::wsl_distro;
use vet_host::paths::backend_path;

/// The native folder dialog; `None` if the user cancelled.
#[tauri::command]
pub async fn pick_repository(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let Some(folder) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = folder.into_path().map_err(|e| e.to_string())?;
    backend_path(&path.to_string_lossy(), wsl_distro().as_deref()).map(Some)
}

/// The native file dialog (a language server's executable); `None` if the user cancelled.
#[tauri::command]
pub async fn pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let Some(file) = app.dialog().file().blocking_pick_file() else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    backend_path(&path.to_string_lossy(), wsl_distro().as_deref()).map(Some)
}
