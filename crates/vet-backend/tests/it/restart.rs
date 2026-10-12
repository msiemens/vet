//! Restarting a language's server: the backend with a language server that names its process in
//! every hover, leaves a hover on line 1 unanswered, and can be made to fail its start. Needs
//! `python3` for that server and is skipped without it.

#![allow(clippy::disallowed_methods)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use vet_backend::{Orchestrator, open_store};
use vet_protocol::{Request, Side, UnavailableKind};

use crate::common::{Client, config, repo_with};

const FAKE_SERVER: &str = r#"#!/usr/bin/env python3
import json, os, sys

if "--version" in sys.argv:
    print("fake-server 1.0")
    sys.exit(0)

if os.path.exists("@BROKEN@"):
    sys.stderr.write("the fake server is broken\n")
    sys.exit(3)

CAPABILITIES = {
    "documentSymbolProvider": True,
    "definitionProvider": True,
    "referencesProvider": True,
    "hoverProvider": True,
    "callHierarchyProvider": True,
    "workspaceSymbolProvider": True,
}

def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(length))

def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()

while True:
    message = read()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": CAPABILITIES}})
    elif method == "exit":
        break
    elif method == "textDocument/hover":
        if message["params"]["position"]["line"] == 0:
            text = "pid %d" % os.getpid()
            send({"jsonrpc": "2.0", "id": message["id"], "result": {"contents": {"kind": "plaintext", "value": text}}})
    elif "id" in message and method:
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
"#;

struct Rig {
    client: Arc<Client>,
    project: u32,
    /// While this file exists the server exits at once.
    broken: PathBuf,
    _keep: (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
}

fn python_available() -> bool {
    Command::new("python3").arg("--version").output().is_ok()
}

impl Rig {
    async fn start() -> Self {
        Self::start_with(false).await
    }

    /// A project whose rust server is the fake one, which fails its start while `broken`.
    async fn start_with(broken: bool) -> Self {
        let repo = repo_with(
            &[
                (
                    "Cargo.toml",
                    "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                ),
                ("src/lib.rs", "pub fn f() -> u32 {\n    1\n}\n"),
            ],
            &[("src/lib.rs", "pub fn f() -> u32 {\n    2\n}\n")],
        );
        let tools = tempfile::tempdir().unwrap();
        let flag = tools.path().join("broken");
        if broken {
            std::fs::write(&flag, "").unwrap();
        }
        let broken = flag;
        let script = tools.path().join("fake-server");
        std::fs::write(&script, FAKE_SERVER.replace("@BROKEN@", broken.to_str().unwrap())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let data = tempfile::tempdir().unwrap();
        let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
        let client = Arc::new(Client::new(&handler));
        let project = client.open(repo.path()).await;
        let rig = Self {
            client,
            project,
            broken,
            _keep: (data, tools, repo),
        };
        rig.call(Request::SetServerPath {
            project,
            language: "rust".into(),
            path: Some(script.to_str().unwrap().into()),
        })
        .await
        .unwrap();
        rig
    }

    async fn call(&self, request: Request) -> Result<vet_protocol::Response, String> {
        self.client.call(request).await
    }

    async fn restart(&self) -> Result<vet_protocol::Response, String> {
        self.call(Request::RestartServer {
            project: self.project,
            language: "rust".into(),
        })
        .await
    }

    /// The hover at `line`: the text and why there is none.
    async fn hover(&self, line: u32) -> (Option<String>, Option<vet_protocol::Unavailable>) {
        let revision = self.client.revision(self.project).await;
        self.client
            .hover(self.project, revision, Side::Target, "src/lib.rs", (line, 7))
            .await
    }

    /// The process that answers a hover now.
    async fn pid(&self) -> u32 {
        let (text, unavailable) = self.hover(0).await;
        let text = text.unwrap_or_else(|| panic!("no answer: {unavailable:?}"));
        text.rsplit(' ')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{text}"))
    }
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|o| o.status.success())
}

async fn gone(pid: u32) -> bool {
    for _ in 0..300 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn restarting_a_running_server_replaces_its_process() {
    if !python_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let rig = Rig::start().await;
    let before = rig.pid().await;
    assert_eq!(rig.pid().await, before, "one process serves the hovers");

    rig.restart().await.unwrap();

    let after = rig.pid().await;
    assert_ne!(after, before);
    assert!(gone(before).await, "the old process is still running");
}

#[tokio::test]
async fn a_server_that_failed_to_start_is_started_again_by_a_restart() {
    if !python_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let rig = Rig::start_with(true).await;
    let (text, unavailable) = rig.hover(0).await;
    assert_eq!(text, None);
    assert_eq!(unavailable.unwrap().kind, UnavailableKind::ServerFailed);

    // Still broken: the restart says why instead of answering.
    let error = rig.restart().await.unwrap_err();
    assert!(error.contains("the fake server is broken"), "{error}");

    std::fs::remove_file(&rig.broken).unwrap();
    rig.restart().await.unwrap();
    rig.pid().await;
}

#[tokio::test]
async fn requests_in_flight_on_the_old_server_fail_as_restarted() {
    if !python_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let rig = Rig::start().await;
    let before = rig.pid().await;

    let (client, project) = (rig.client.clone(), rig.project);
    let revision = client.revision(project).await;
    let hanging = tokio::spawn(async move {
        client
            .hover(project, revision, Side::Target, "src/lib.rs", (1, 7))
            .await
    });
    // The server never answers line 1; the restart must not wait for it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!hanging.is_finished());

    rig.restart().await.unwrap();

    let (text, unavailable) = tokio::time::timeout(Duration::from_secs(20), hanging)
        .await
        .expect("the hover outlived the restart")
        .unwrap();
    assert_eq!(text, None);
    let unavailable = unavailable.expect("the hover failed");
    assert_eq!(unavailable.kind, UnavailableKind::QueryFailed);
    assert!(unavailable.message.contains("restarted"), "{}", unavailable.message);

    assert_ne!(rig.pid().await, before);
}
