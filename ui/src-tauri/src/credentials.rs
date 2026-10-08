//! The personal access tokens the backend holds in memory only, kept in the operating system's
//! keychain (the macOS Keychain, the Windows Credential Manager) under `vet:<host>`. The shell owns
//! them in every topology: with the backend in WSL it is the Windows shell that holds them and the
//! page hands them to the backend.
//!
//! A keychain cannot be listed portably, so the hosts that have a token are listed in
//! `credential-hosts.json` of the app config directory (hosts only, never a token), which is how
//! the page finds them all when it connects.

use std::path::{Path, PathBuf};

use keyring::Entry;
use serde::Serialize;
use tauri::{AppHandle, Manager};

const FILE: &str = "credential-hosts.json";

/// The keychain entry's account name; the service is `vet:<host>`.
const ACCOUNT: &str = "personal-access-token";

/// A token and the host it is for. Deliberately no `Debug`.
#[derive(Serialize)]
pub struct StoredCredential {
    host: String,
    token: String,
}

fn entry(host: &str) -> Result<Entry, String> {
    Entry::new(&format!("vet:{host}"), ACCOUNT).map_err(|e| format!("cannot open the keychain: {e}"))
}

fn index_file(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(FILE))
        .map_err(|e| format!("no config directory: {e}"))
}

/// The hosts listed in `file`; none if it is missing or damaged, in which case the keychain's
/// tokens are still there but are not found until they are stored again.
fn read_hosts(file: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "ignoring the list of hosts with a token");
        Vec::new()
    })
}

fn write_hosts(file: &Path, hosts: &[String]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(hosts).map_err(|e| e.to_string())?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))
}

/// `hosts` with `host` in it, once.
fn with_host(mut hosts: Vec<String>, host: &str) -> Vec<String> {
    if !hosts.iter().any(|h| h == host) {
        hosts.push(host.to_owned());
    }
    hosts
}

fn without_host(mut hosts: Vec<String>, host: &str) -> Vec<String> {
    hosts.retain(|h| h != host);
    hosts
}

/// The token of one host, if the keychain has it.
fn token_of(host: &str) -> Result<Option<String>, String> {
    match entry(host)?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("cannot read the token of {host} from the keychain: {e}")),
    }
}

/// Every token the keychain holds for a host in the list.
#[tauri::command]
pub async fn stored_credentials(app: AppHandle) -> Result<Vec<StoredCredential>, String> {
    let file = index_file(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut found = Vec::new();
        for host in read_hosts(&file) {
            if let Some(token) = token_of(&host)? {
                found.push(StoredCredential { host, token });
            }
        }
        Ok(found)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The token of `host`, if the keychain has it.
#[tauri::command]
pub async fn stored_credential(host: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || token_of(&host))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn store_credential(app: AppHandle, host: String, token: String) -> Result<(), String> {
    let file = index_file(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        entry(&host)?
            .set_password(&token)
            .map_err(|e| format!("cannot store the token of {host} in the keychain: {e}"))?;
        write_hosts(&file, &with_host(read_hosts(&file), &host))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn remove_credential(app: AppHandle, host: String) -> Result<(), String> {
    let file = index_file(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        match entry(&host)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(format!("cannot remove the token of {host} from the keychain: {e}")),
        }
        write_hosts(&file, &without_host(read_hosts(&file), &host))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_listed_once_and_taken_off_when_removed() {
        let hosts = with_host(with_host(Vec::new(), "github.com"), "github.com");
        assert_eq!(hosts, ["github.com"]);
        let hosts = with_host(hosts, "dev.azure.com");
        assert_eq!(without_host(hosts, "github.com"), ["dev.azure.com"]);
    }

    #[test]
    fn the_list_survives_a_restart_and_a_missing_or_damaged_file_means_no_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config").join(FILE);
        assert!(read_hosts(&file).is_empty());

        write_hosts(&file, &["github.com".into(), "ghe.corp.example".into()]).unwrap();
        assert_eq!(read_hosts(&file), ["github.com", "ghe.corp.example"]);

        std::fs::write(&file, "[\"github.com\"").unwrap();
        assert!(read_hosts(&file).is_empty());
    }
}
