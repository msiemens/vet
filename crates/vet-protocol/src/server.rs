//! Language servers on the backend's machine.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Side, Unavailable};

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
    /// The language's id, e.g. `rust`.
    pub language: String,
    /// The language's name as a person knows it, e.g. "C#".
    pub title: String,
    /// The server's name as a person knows it, e.g. "Pyright".
    pub server: String,
    pub state: ServerState,
    /// The path the user set; it is what `state` was resolved from if present.
    pub custom_path: Option<String>,
    /// The version `InstallServer` installs; `None` if Vet does not install this server.
    pub pinned_version: Option<String>,
    /// Why `InstallServer` cannot run now.
    pub install_blocker: Option<String>,
    /// `InstallServer` can run now: there is a version to install and no blocker.
    pub can_install: bool,
    /// What the last server started advertised; `None` before one has started.
    pub features: Option<ServerFeatures>,
}

/// The operations a language server advertised. A request for one it lacks is answered with
/// `UnavailableKind::NotOffered`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerFeatures {
    pub document_symbols: bool,
    pub definition: bool,
    pub references: bool,
    pub implementations: bool,
    pub hover: bool,
    pub call_hierarchy: bool,
    pub workspace_symbols: bool,
}

/// Which server serves a file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum Serving {
    /// The server of this language, as `ServerInfo::language` names it, whether it runs or not.
    Language { language: String },
    /// No language is configured for this kind of file; `why.kind` is `Unsupported`.
    Unsupported { why: Unavailable },
}

/// What the server of a language is doing for one side of the changeset on view.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum ServerPhase {
    /// Being started: its dependencies restored, its process launched.
    Starting,
    /// Working on its own: indexing the project after it started, analysing after an edit. It
    /// answers, but not completely.
    Indexing {
        /// How far it says it is, when it says.
        done: Option<u32>,
        total: Option<u32>,
        /// What it says it is doing.
        detail: Option<String>,
    },
    Ready,
    /// There is none; `why.kind` says whether it is missing, failed or not allowed to run.
    Failed {
        why: Unavailable,
    },
}

/// A language's server for the base or the target side. A side and language that have no entry
/// have no server: none was needed yet, or it was stopped to make room.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerRuntime {
    /// The language's id, as in `ServerInfo`.
    pub language: String,
    pub side: Side,
    pub phase: ServerPhase,
}

/// What the servers of the changeset on view are doing. A client keeps the highest `serial` it
/// has heard, whether it came as an event or with a list of the servers.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerRuntimes {
    #[ts(type = "number")]
    pub serial: u64,
    pub runtimes: Vec<ServerRuntime>,
}

/// How a restore step ended short of plain success.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RestoreFailureKind {
    /// The step failed: what that project's analysis cannot resolve is missing, not wrong.
    Failed,
    /// A stand-in step installed the dependencies after the planned one was refused.
    StoodIn,
}

/// A dependency restore step that failed or had to be stood in for (§7.2).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreFailure {
    pub kind: RestoreFailureKind,
    /// The project directory, relative to the repository with `/`; `.` is the root.
    pub project: String,
    /// The tool that failed, e.g. `pnpm`.
    pub tool: String,
    /// Fit to show.
    pub message: String,
}
