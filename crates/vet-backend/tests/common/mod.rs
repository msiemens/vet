//! Shared by the tests that drive the handler like a connection would.
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use vet_backend::{Config, Orchestrator, open_store};
use vet_protocol::{
    Diagnostic, Locations, ProjectId, Request, Response, ServerInfo, ServerMessage, ServerState, Side, Unavailable,
};
use vet_transport::{Events, Handler};

/// Git as the tests use it: nothing of the user's configuration (identity,
/// signing, hooks, autocrlf, templates) leaks in. Returns trimmed stdout.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// `git init` plus the repository-local settings that keep Vet itself, which reads
/// the user's global git configuration in-process, from seeing it: a global
/// `core.autocrlf` or `core.excludesFile` would change snapshots.
pub fn init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    git(dir, &["config", "core.excludesFile", "/dev/null"]);
}

pub fn write(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Runs a program in `dir`; it must succeed.
pub fn run(dir: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program).args(args).current_dir(dir).output().unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Line and column of the `nth` occurrence of `needle`, `inside` columns in.
pub fn find(text: &str, needle: &str, nth: usize, inside: u32) -> (u32, u32) {
    let (line, col) = text
        .lines()
        .enumerate()
        .flat_map(|(l, s)| s.match_indices(needle).map(move |(c, _)| (l, c)))
        .nth(nth)
        .expect("needle present");
    (line as u32, col as u32 + inside)
}

pub struct Client {
    pub handler: Arc<Orchestrator>,
    pub events: Events,
    pub inbox: UnboundedReceiver<ServerMessage>,
}

impl Client {
    pub fn new(handler: &Arc<Orchestrator>) -> Self {
        let (events, inbox) = Events::channel();
        Self {
            handler: handler.clone(),
            events,
            inbox,
        }
    }

    pub async fn call(&self, request: Request) -> Result<Response, String> {
        self.handler.handle(request, self.events.clone()).await
    }

    pub async fn open(&self, repo: &Path) -> ProjectId {
        let request = Request::OpenProject {
            repo: repo.to_str().unwrap().into(),
        };
        match self.call(request).await.unwrap() {
            Response::Project { project, .. } => project,
            other => panic!("{other:?}"),
        }
    }

    pub async fn revision(&self, project: ProjectId) -> u32 {
        match self.call(Request::Manifest { project }).await.unwrap() {
            Response::Manifest(m) => m.revision,
            other => panic!("{other:?}"),
        }
    }

    pub async fn servers(&self, project: ProjectId) -> Vec<ServerInfo> {
        match self.call(Request::ListServers { project }).await.unwrap() {
            Response::Servers { servers } => servers,
            other => panic!("{other:?}"),
        }
    }

    /// Installs the pinned server of `language`; it must work.
    pub async fn install(&self, project: ProjectId, language: &str) {
        let request = Request::InstallServer {
            project,
            language: language.into(),
        };
        match self.call(request).await.unwrap() {
            Response::Servers { servers } => {
                let server = servers.iter().find(|s| s.language == language).unwrap();
                assert!(matches!(server.state, ServerState::Found { .. }), "{server:?}");
            }
            other => panic!("{other:?}"),
        }
    }

    pub async fn hover(
        &self,
        project: ProjectId,
        revision: u32,
        side: Side,
        path: &str,
        (line, col): (u32, u32),
    ) -> (Option<String>, Option<Unavailable>) {
        let request = Request::Hover {
            project,
            revision,
            side,
            path: path.into(),
            line,
            col,
        };
        match self.call(request).await.unwrap() {
            Response::Hover { text, unavailable, .. } => (text, unavailable),
            other => panic!("{other:?}"),
        }
    }

    pub async fn definition(
        &self,
        project: ProjectId,
        revision: u32,
        side: Side,
        path: &str,
        (line, col): (u32, u32),
    ) -> Locations {
        let request = Request::Definition {
            project,
            revision,
            side,
            path: path.into(),
            line,
            col,
        };
        match self.call(request).await.unwrap() {
            Response::Definition(locations) => locations,
            other => panic!("{other:?}"),
        }
    }

    pub async fn references(
        &self,
        project: ProjectId,
        revision: u32,
        side: Side,
        path: &str,
        (line, col): (u32, u32),
    ) -> Locations {
        let request = Request::References {
            project,
            revision,
            side,
            path: path.into(),
            line,
            col,
        };
        match self.call(request).await.unwrap() {
            Response::References(locations) => locations,
            other => panic!("{other:?}"),
        }
    }

    pub async fn diagnostics(
        &self,
        project: ProjectId,
        revision: u32,
        path: &str,
    ) -> (Vec<Diagnostic>, Option<Unavailable>) {
        let request = Request::Diagnostics {
            project,
            revision,
            path: path.into(),
        };
        match self.call(request).await.unwrap() {
            Response::Diagnostics {
                diagnostics,
                unavailable,
                ..
            } => (diagnostics, unavailable),
            other => panic!("{other:?}"),
        }
    }
}

pub fn config() -> Config {
    Config {
        debounce: Duration::from_millis(150),
        poll: Duration::from_millis(300),
        orphan_grace: Duration::from_secs(60),
    }
}

/// A handler on a fresh data directory, which the caller keeps alive.
pub async fn handler() -> (Arc<Orchestrator>, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    (handler, data)
}

/// A repository whose `main` commit holds `base`, with `dirty` written on top.
pub fn repo_with(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    write(repo.path(), ".gitignore", "target/\n");
    for (name, content) in base {
        write(repo.path(), name, content);
    }
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    for (name, content) in dirty {
        write(repo.path(), name, content);
    }
    repo
}
