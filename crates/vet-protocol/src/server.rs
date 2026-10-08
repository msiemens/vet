//! Language servers on the backend's machine.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServerOrigin {
    /// The path the user set.
    Custom,
    /// Part of the project: its Rust toolchain.
    Project,
    /// Installed by Vet into its data directory.
    Installed,
    /// On `PATH`.
    Path,
}

/// The `tsserver` a TypeScript language server answers through.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Tsserver {
    Using {
        /// Which `typescript` it is, e.g. "the project's typescript 5.9.2". Fit to show.
        whose: String,
        /// Its `lib/tsserver.js`.
        path: String,
    },
    Missing {
        /// Why there is none. Fit to show.
        reason: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ServerState {
    Found {
        path: String,
        origin: ServerOrigin,
        /// If it is cheap to learn.
        version: Option<String>,
        /// The `tsserver` it uses; only a TypeScript server has one.
        tsserver: Option<Tsserver>,
    },
    Missing {
        /// What was looked for and where, and what to do. Fit to show.
        reason: String,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerInfo {
    /// `rust`, `typescript`, `python` or `csharp`.
    pub language: String,
    /// The language's name as a person knows it, e.g. "C#".
    pub title: String,
    /// The server's name as a person knows it, e.g. "Pyright".
    pub server: String,
    /// The file extensions (without the dot) the server serves.
    pub extensions: Vec<String>,
    pub state: ServerState,
    /// The path the user set; it is what `state` was resolved from if present.
    pub custom_path: Option<String>,
    /// The version `InstallServer` installs; `None` if Vet does not install this server.
    pub pinned_version: Option<String>,
    /// Why `InstallServer` cannot run now.
    pub install_blocker: Option<String>,
    /// `InstallServer` can run now: there is a version to install and no blocker.
    pub can_install: bool,
}

/// A dependency restore step that failed (§7.2): what that project's analysis
/// cannot resolve is missing, not wrong.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreFailure {
    /// The project directory, relative to the repository with `/`; `.` is the root.
    pub project: String,
    /// The tool that failed, e.g. `pnpm`.
    pub tool: String,
    /// Fit to show.
    pub message: String,
}
