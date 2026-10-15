//! The hosts that have a token in the keychain. A keychain cannot be listed portably, so they are
//! listed in a file of the app config directory (hosts only, never a token), which is how the page
//! finds all tokens when it connects.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::files::write_atomic;

/// Serializes the read-change-write of the list, which commands on different threads run.
static INDEX: Mutex<()> = Mutex::new(());

pub struct CredentialHosts {
    file: PathBuf,
}

impl CredentialHosts {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// The hosts listed; none if the file is missing or damaged, in which case the keychain's
    /// tokens are still there but are not found until they are stored again.
    pub fn list(&self) -> Vec<String> {
        read_hosts(&self.file)
    }

    /// Lists `host`, once.
    pub fn add(&self, host: &str) -> Result<(), String> {
        self.update(|mut hosts| {
            if !hosts.iter().any(|h| h == host) {
                hosts.push(host.to_owned());
            }
            hosts
        })
    }

    pub fn remove(&self, host: &str) -> Result<(), String> {
        self.update(|mut hosts| {
            hosts.retain(|h| h != host);
            hosts
        })
    }

    #[allow(clippy::disallowed_methods, reason = "the one place that locks the host index")]
    fn update(&self, change: impl FnOnce(Vec<String>) -> Vec<String>) -> Result<(), String> {
        let _held = INDEX.lock().unwrap_or_else(PoisonError::into_inner);
        let text = serde_json::to_string_pretty(&change(read_hosts(&self.file))).map_err(|e| e.to_string())?;
        write_atomic(&self.file, &text).map_err(|e| format!("cannot write {}: {e}", self.file.display()))
    }
}

fn read_hosts(file: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "ignoring the list of hosts with a token");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_listed_once_and_taken_off_when_removed() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = CredentialHosts::new(dir.path().join("hosts.json"));
        hosts.add("github.com").unwrap();
        hosts.add("github.com").unwrap();
        assert_eq!(hosts.list(), ["github.com"]);
        hosts.add("dev.azure.com").unwrap();
        hosts.remove("github.com").unwrap();
        assert_eq!(hosts.list(), ["dev.azure.com"]);
    }

    #[test]
    fn the_list_survives_a_restart_and_a_missing_or_damaged_file_means_no_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config").join("hosts.json");
        assert!(CredentialHosts::new(file.clone()).list().is_empty());

        CredentialHosts::new(file.clone()).add("github.com").unwrap();
        CredentialHosts::new(file.clone()).add("ghe.corp.example").unwrap();
        assert_eq!(
            CredentialHosts::new(file.clone()).list(),
            ["github.com", "ghe.corp.example"]
        );

        std::fs::write(&file, "[\"github.com\"").unwrap();
        assert!(CredentialHosts::new(file).list().is_empty());
    }

    #[test]
    fn hosts_stored_at_the_same_time_are_all_listed() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = CredentialHosts::new(dir.path().join("hosts.json"));
        std::thread::scope(|scope| {
            for n in 0..16 {
                let hosts = &hosts;
                scope.spawn(move || hosts.add(&format!("host{n}")).unwrap());
            }
        });
        assert_eq!(hosts.list().len(), 16);
    }
}
