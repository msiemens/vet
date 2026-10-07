//! The native file dialogs, with their results translated to the backend's paths.

use crate::backend::wsl_distro;

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

/// The native folder dialog; `None` if the user cancelled.
#[tauri::command]
pub async fn pick_repository(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(folder) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = folder.into_path().map_err(|e| e.to_string())?;
    backend_path(&path.to_string_lossy(), wsl_distro().as_deref()).map(Some)
}

/// The native file dialog (a language server's executable); `None` if the user cancelled.
#[tauri::command]
pub async fn pick_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(file) = app.dialog().file().blocking_pick_file() else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    backend_path(&path.to_string_lossy(), wsl_distro().as_deref()).map(Some)
}

#[cfg(test)]
mod tests {
    use super::backend_path;

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
}
