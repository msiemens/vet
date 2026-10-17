//! Shared by the tests that drive the handler like a connection would.
#![allow(dead_code)]
// Tests build their repositories with git.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use vet_backend::{Config, Orchestrator, open_store};
use vet_protocol::{
    Diagnostic, Failure, LocationRows, Locations, ProjectId, Request, Response, ServerInfo, ServerMessage, ServerState,
    Side, Unavailable,
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
    pub inbox: Receiver<ServerMessage>,
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

    pub async fn call(&self, request: Request) -> Result<Response, Failure> {
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
        match self
            .call(Request::Manifest {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .unwrap()
        {
            Response::Manifest(m) => m.revision,
            other => panic!("{other:?}"),
        }
    }

    pub async fn servers(&self, project: ProjectId) -> Vec<ServerInfo> {
        match self.call(Request::ListServers { project }).await.unwrap() {
            Response::Servers { servers, .. } => servers,
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
            Response::Servers { servers, .. } => {
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

    /// Has the server of `path`'s language started and load the project, by asking it
    /// something at `at` on the target side. A test that moves the worktree does this
    /// first: the settle of a move is due after the debounce, and a cold server can take
    /// longer than that to answer, which would leave the question at a revision left
    /// behind. Hover keeps its own answers, so choose a position the test does not ask
    /// about again.
    pub async fn warm_up(&self, project: ProjectId, revision: u32, path: &str, at: (u32, u32)) {
        let (_, unavailable) = self.hover(project, revision, Side::Target, path, at).await;
        assert_eq!(unavailable, None);
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
    ) -> LocationRows {
        self.references_up_to(project, revision, side, path, (line, col), None)
            .await
    }

    pub async fn references_up_to(
        &self,
        project: ProjectId,
        revision: u32,
        side: Side,
        path: &str,
        (line, col): (u32, u32),
        limit: Option<u32>,
    ) -> LocationRows {
        let request = Request::References {
            project,
            revision,
            side,
            path: path.into(),
            line,
            col,
            limit,
        };
        match self.call(request).await.unwrap() {
            Response::References(locations) => locations,
            other => panic!("{other:?}"),
        }
    }

    pub async fn implementations(
        &self,
        project: ProjectId,
        revision: u32,
        side: Side,
        path: &str,
        (line, col): (u32, u32),
    ) -> LocationRows {
        let request = Request::Implementations {
            project,
            revision,
            side,
            path: path.into(),
            line,
            col,
        };
        match self.call(request).await.unwrap() {
            Response::Implementations(rows) => rows,
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
            against_base: false,
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
        prefetch_pace: Duration::ZERO,
        ..Config::default()
    }
}

/// Waits until the project's background task of `kind` has started and is no longer running: the
/// marker that the work it would have caused is over, where a sleep would only hope that it is.
/// The bound is generous because a language server may take a while to answer under load. Only a
/// debug build can see the tasks; in one it is the barrier the tests use instead of time.
#[cfg(debug_assertions)]
pub async fn background_ended(client: &Client, project: ProjectId, kind: &'static str) {
    for _ in 0..6000 {
        let (started, live) = client.handler.background_counts(project, kind);
        if started > 0 && live == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the {kind} task did not end");
}

/// How often the project's background task of `kind` has been started. A debug build only.
#[cfg(debug_assertions)]
pub fn background_started(client: &Client, project: ProjectId, kind: &'static str) -> usize {
    client.handler.background_counts(project, kind).0
}

/// Waits until exactly `live` of the project's background tasks of `kind` are running: the marker
/// that a run a newer one replaced has ended. A debug build only.
#[cfg(debug_assertions)]
pub async fn background_live(client: &Client, project: ProjectId, kind: &'static str, live: usize) {
    for _ in 0..6000 {
        if client.handler.background_counts(project, kind).1 == live {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the {kind} tasks never came to {live} running");
}

/// A handler on a fresh data directory, which the caller keeps alive.
pub async fn handler() -> (Arc<Orchestrator>, tempfile::TempDir) {
    handler_with(config()).await
}

/// As `handler`, on `config`.
pub async fn handler_with(config: Config) -> (Arc<Orchestrator>, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config));
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
