//! Showing a web address in the user's browser. The address comes from the backend (a platform's
//! page for a check), so only web addresses are opened.

use std::process::Command;

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
    let url = web_address(&url)?;
    let mut opener = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        Command::new("xdg-open")
    };
    let status = opener
        .arg(url)
        .status()
        .map_err(|e| format!("cannot open the browser: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the browser did not open {url}"))
    }
}

#[cfg(test)]
mod tests {
    use super::web_address;

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
