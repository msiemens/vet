//! Cancellation from the connection down to the language server: the backend served
//! over its WebSocket, with a language server that never answers a hover and records
//! what it is sent. Needs `python3` for that server and is skipped without it.

#![allow(clippy::disallowed_methods)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use common::{config, repo_with};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{ClientMessage, PROTOCOL_VERSION, ProjectId, Request, Response, ServerMessage, Side};
use vet_transport::Server;

/// Speaks just enough LSP to start, appends every message it receives to a log
/// (one JSON object per line), and leaves `textDocument/hover` unanswered.
const FAKE_SERVER: &str = r#"#!/usr/bin/env python3
import json, sys

if "--version" in sys.argv:
    print("fake-server 1.0")
    sys.exit(0)

LOG = "@LOG@"
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
    with open(LOG, "a") as log:
        log.write(json.dumps(message) + "\n")
    method = message.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": CAPABILITIES}})
    elif method == "exit":
        break
    elif method == "textDocument/hover":
        pass
    elif "id" in message and method:
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
"#;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Rig {
    socket: Socket,
    /// What the language server received.
    log: PathBuf,
    project: ProjectId,
    revision: u32,
    next_id: u32,
    _keep: (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
}

fn python_available() -> bool {
    Command::new("python3").arg("--version").output().is_ok()
}

async fn send(socket: &mut Socket, message: &ClientMessage) {
    socket
        .send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

/// The next message that `wanted` picks, within thirty seconds; the events in between are skipped.
async fn receive<T>(socket: &mut Socket, wanted: impl Fn(ServerMessage) -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let Some(Ok(Message::Text(text))) = socket.next().await else {
                panic!("connection closed");
            };
            if let Some(found) = wanted(serde_json::from_str(text.as_str()).unwrap()) {
                return found;
            }
        }
    })
    .await
    .expect("the message never came")
}

impl Rig {
    async fn start() -> Self {
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
        let log = tools.path().join("received.jsonl");
        let script = tools.path().join("fake-server");
        std::fs::write(&script, FAKE_SERVER.replace("@LOG@", log.to_str().unwrap())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let data = tempfile::tempdir().unwrap();
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), config());
        let server = Server::bind().await.unwrap();
        let ready = server.ready().unwrap();
        tokio::spawn(server.serve(orchestrator));
        let mut socket = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", ready.port))
            .await
            .unwrap()
            .0;
        let hello = ClientMessage::Hello {
            token: ready.token,
            protocol_version: PROTOCOL_VERSION,
        };
        send(&mut socket, &hello).await;
        receive(&mut socket, |m| {
            matches!(m, ServerMessage::HelloAck { .. }).then_some(())
        })
        .await;

        let mut rig = Self {
            socket,
            log,
            project: 0,
            revision: 0,
            next_id: 0,
            _keep: (data, tools, repo),
        };
        let repo = rig._keep.2.path().to_str().unwrap().to_owned();
        let Response::Project { project, .. } = rig.call(Request::OpenProject { repo }).await else {
            panic!("not a project");
        };
        rig.project = project;
        rig.call(Request::SetServerPath {
            project,
            language: "rust".into(),
            path: Some(script.to_str().unwrap().into()),
        })
        .await;
        let Response::Manifest(manifest) = rig
            .call(Request::Manifest {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
        else {
            panic!("not a manifest");
        };
        rig.revision = manifest.revision;
        rig
    }

    async fn call(&mut self, request: Request) -> Response {
        let id = self.request(request).await;
        receive(&mut self.socket, |m| match m {
            ServerMessage::Response { id: answered, response } if answered == id => Some(response),
            ServerMessage::Error {
                id: Some(answered),
                message,
            } if answered == id => panic!("{message}"),
            _ => None,
        })
        .await
    }

    /// Sends a request and returns its id without waiting for the answer.
    async fn request(&mut self, request: Request) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        send(&mut self.socket, &ClientMessage::Request { id, request }).await;
        id
    }

    async fn hover(&mut self) -> u32 {
        let request = Request::Hover {
            project: self.project,
            revision: self.revision,
            side: Side::Target,
            path: "src/lib.rs".into(),
            line: 0,
            col: 7,
        };
        self.request(request).await
    }

    fn received(&self) -> Vec<Value> {
        read_log(&self.log)
    }

    /// Waits until the language server has received a message `matching` says yes to.
    async fn server_received(&self, what: &str, matching: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..3000 {
            if let Some(found) = self.received().into_iter().find(&matching) {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the language server never received {what}:\n{:#?}", self.received());
    }
}

fn read_log(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        // The server may be in the middle of writing the last line.
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[tokio::test]
async fn cancelling_a_hover_answers_cancelled_and_cancels_the_server_request() {
    if !python_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let mut rig = Rig::start().await;
    let id = rig.hover().await;
    let hover = rig
        .server_received("the hover", |m| m["method"] == "textDocument/hover")
        .await;

    send(&mut rig.socket, &ClientMessage::Cancel { id }).await;
    receive(&mut rig.socket, |m| {
        matches!(m, ServerMessage::Cancelled { id: cancelled } if cancelled == id).then_some(())
    })
    .await;
    let cancel = rig
        .server_received("$/cancelRequest", |m| m["method"] == "$/cancelRequest")
        .await;
    assert_eq!(cancel["params"]["id"], hover["id"]);

    // The connection and the server serve on.
    let next = rig.hover().await;
    rig.server_received("a second hover", |m| {
        m["method"] == "textDocument/hover" && m["id"] != hover["id"]
    })
    .await;
    send(&mut rig.socket, &ClientMessage::Cancel { id: next }).await;
}

#[tokio::test]
async fn a_disconnect_cancels_the_server_requests_of_the_handlers_it_aborts() {
    if !python_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let mut rig = Rig::start().await;
    rig.hover().await;
    let hover = rig
        .server_received("the hover", |m| m["method"] == "textDocument/hover")
        .await;

    drop(rig.socket);
    let cancel = {
        let log = rig.log.clone();
        tokio::time::timeout(Duration::from_secs(30), async move {
            loop {
                if let Some(found) = read_log(&log).into_iter().find(|m| m["method"] == "$/cancelRequest") {
                    return found;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the abandoned hover was never cancelled")
    };
    assert_eq!(cancel["params"]["id"], hover["id"]);
}
