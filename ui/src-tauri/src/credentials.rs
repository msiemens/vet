//! The personal access tokens the backend holds in memory only, kept in the operating system's
//! keychain (the macOS Keychain, the Windows Credential Manager) under `vet:<host>`. The shell owns
//! them in every topology: with the backend in WSL it is the Windows shell that holds them and the
//! page hands them to the backend.
//!
//! A keychain cannot be listed portably, so the hosts that have a token are listed in
//! `credential-hosts.json` of the app config directory (hosts only, never a token), which is how
//! the page finds them all when it connects.

use keyring::Entry;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use vet_host::credential_hosts::CredentialHosts;

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

fn hosts(app: &AppHandle) -> Result<CredentialHosts, String> {
    app.path()
        .app_config_dir()
        .map(|dir| CredentialHosts::new(dir.join(FILE)))
        .map_err(|e| format!("no config directory: {e}"))
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
    let hosts = hosts(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut found = Vec::new();
        for host in hosts.list() {
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
    let hosts = hosts(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        entry(&host)?
            .set_password(&token)
            .map_err(|e| format!("cannot store the token of {host} in the keychain: {e}"))?;
        hosts.add(&host)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn remove_credential(app: AppHandle, host: String) -> Result<(), String> {
    let hosts = hosts(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        match entry(&host)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(format!("cannot remove the token of {host} from the keychain: {e}")),
        }
        hosts.remove(&host)
    })
    .await
    .map_err(|e| e.to_string())?
}
