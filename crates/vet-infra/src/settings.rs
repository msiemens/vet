//! The backend's own settings (SPEC §8): one JSON file in the data directory, apart from
//! the review database, so that a new setting never recreates the database.
//!
//! A file that is missing or cannot be read gives the defaults; a field it lacks takes its
//! default. It is written whole, to a temporary file renamed over the old one.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Megabytes the semantic cache's answers may take.
    pub semantic_cache_mb: u32,
    /// Days a changeset is kept after its last review was finished.
    pub finished_review_days: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            semantic_cache_mb: 256,
            finished_review_days: 30,
        }
    }
}

impl Settings {
    pub fn semantic_cache_bytes(&self) -> u64 {
        u64::from(self.semantic_cache_mb) << 20
    }

    pub fn finished_review_retention(&self) -> Duration {
        Duration::from_secs(u64::from(self.finished_review_days) * 24 * 3600)
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// The settings kept in `dir`, the data directory.
pub fn load(dir: &Path) -> Settings {
    let Ok(text) = std::fs::read_to_string(path(dir)) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "the settings file cannot be read; using the defaults");
        Settings::default()
    })
}

/// Keeps `settings` in `dir`.
pub fn save(dir: &Path, settings: &Settings) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    let temporary = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&temporary, text)?;
    std::fs::rename(temporary, path(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_read_back_and_a_missing_or_broken_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Settings::default());

        let changed = Settings {
            semantic_cache_mb: 64,
            finished_review_days: 7,
        };
        save(dir.path(), &changed).unwrap();
        assert_eq!(load(dir.path()), changed);

        // A field the file lacks takes its default.
        std::fs::write(dir.path().join(FILE), "{}").unwrap();
        assert_eq!(load(dir.path()), Settings::default());

        std::fs::write(dir.path().join(FILE), "not json").unwrap();
        assert_eq!(load(dir.path()), Settings::default());
    }
}
