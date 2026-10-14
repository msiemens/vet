//! Showing a web address in the user's browser, and a folder in the file manager. The address comes
//! from the backend (a platform's page for a check), so only web addresses are opened, and of the
//! opener only folders.

use crate::backend::wsl_distro;

/// `url` if it is a web address that is safe to hand to the system's opener as one argument.
fn web_address(url: &str) -> Result<&str, String> {
    let web = url.starts_with("https://") || url.starts_with("http://");
    if web && !url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Ok(url)
    } else {
        Err(format!("{url:?} is not a web address"))
    }
}

/// Opens `url` in the default browser.
#[tauri::command]
pub async fn open_url(url: String) -> Result<(), String> {
    tauri_plugin_opener::open_url(web_address(&url)?, None::<&str>).map_err(|e| format!("cannot open the browser: {e}"))
}

/// `path`, a path of the machine the backend runs on, as this machine names it: in WSL, the
/// share of the distro.
fn host_path(path: &str, distro: Option<&str>) -> String {
    match distro {
        Some(distro) => format!(r"\\wsl.localhost\{distro}{}", path.replace('/', r"\")),
        None => path.to_owned(),
    }
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

#[cfg(test)]
mod tests {
    use super::{host_path, web_address};

    #[test]
    fn a_folder_in_wsl_is_named_by_its_share_and_one_on_this_machine_is_left_alone() {
        assert_eq!(
            host_path("/home/me/.local/share/vet/logs", Some("NixOS")),
            r"\\wsl.localhost\NixOS\home\me\.local\share\vet\logs"
        );
        assert_eq!(
            host_path("/Users/me/Library/Application Support/vet/logs", None),
            "/Users/me/Library/Application Support/vet/logs"
        );
    }

    #[test]
    fn only_web_addresses_without_spaces_are_opened() {
        assert!(web_address("https://github.com/acme/app/commit/abc/checks").is_ok());
        assert!(web_address("http://localhost:3000/x?a=1&b=2").is_ok());
        for refused in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://host",
            "github.com",
            "https://a b",
            "https://a\nb",
            "",
        ] {
            assert!(web_address(refused).is_err(), "{refused:?}");
        }
    }
}
