//! A review under load: the symbols panel asks for the analyses of several symbols at
//! once while the language server is slow, and the viewer goes on marking files seen.
//! Runs the backend on the real store with a language server that takes its time over
//! every file; needs `python3` for that server and is skipped without it.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Client, config, repo_with};
use serde_json::Value;
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{Event, Request, Response, ServerMessage, SymbolId, TaskKind, View};

/// Speaks just enough LSP to start. A file's symbols take `DELAY` seconds to answer; every
/// message it receives, and every `--version` it is asked for, is appended to a log (one
/// JSON object per line).
const SLOW_SERVER: &str = r#"#!/usr/bin/env python3
import json, sys, time

LOG = "@LOG@"
DELAY = 0.15

def log(entry):
    with open(LOG, "a") as out:
        out.write(json.dumps(entry) + "\n")

if "--version" in sys.argv:
    log({"method": "--version"})
    print("slow-server 1.0")
    sys.exit(0)

CAPABILITIES = {
    "documentSymbolProvider": True,
    "definitionProvider": True,
    "referencesProvider": True,
    "hoverProvider": True,
    "callHierarchyProvider": True,
    "workspaceSymbolProvider": True,
}
SYMBOL = {
    "name": "f",
    "kind": 12,
    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 2, "character": 1}},
    "selectionRange": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 8}},
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
    log({"method": message.get("method")})
    method = message.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": CAPABILITIES}})
    elif method == "exit":
        break
    elif method == "textDocument/documentSymbol":
        time.sleep(DELAY)
        send({"jsonrpc": "2.0", "id": message["id"], "result": [SYMBOL]})
    elif "id" in message and method:
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
"#;

const FILES: usize = 6;

/// A marking of a file seen answers within this long, however busy the analyses are.
const TOGGLE_BOUND: Duration = Duration::from_millis(200);

fn python_available() -> bool {
    Command::new("python3").arg("--version").output().is_ok()
}

fn count(log: &Path, method: &str) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["method"] == method)
        .count()
}

#[tokio::test]
async fn many_analyses_of_one_revision_share_one_search_and_marking_files_seen_stays_quick() {
    if !python_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let names: Vec<_> = (0..FILES).map(|i| format!("src/m{i}.rs")).collect();
    // Every text is its own, so that every one is asked of the server.
    let text = |value: usize| format!("pub fn f() -> usize {{\n    {value}\n}}\n");
    let base: Vec<_> = names.iter().enumerate().map(|(i, n)| (n.as_str(), text(i))).collect();
    let dirty: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), text(100 + i)))
        .collect();
    let (base, dirty): (Vec<_>, Vec<_>) = (
        base.iter().map(|(n, t)| (*n, t.as_str())).collect(),
        dirty.iter().map(|(n, t)| (*n, t.as_str())).collect(),
    );
    let repo = repo_with(&base, &dirty);

    let tools = tempfile::tempdir().unwrap();
    let log = tools.path().join("received.jsonl");
    let script = tools.path().join("slow-server");
    std::fs::write(&script, SLOW_SERVER.replace("@LOG@", log.to_str().unwrap())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Arc::new(Client::new(&handler));
    let mut observer = Client::new(&handler);
    let project = client.open(repo.path()).await;
    assert_eq!(observer.open(repo.path()).await, project);
    let revision = client.revision(project).await;
    client
        .call(Request::SetServerPath {
            project,
            language: "rust".into(),
            path: Some(script.to_str().unwrap().into()),
        })
        .await
        .unwrap();

    // Four selections of a symbol, three analyses each, and the changed symbols themselves.
    let symbol = SymbolId {
        path: names[0].clone(),
        name: vec!["f".into()],
    };
    let mut analyses = tokio::task::JoinSet::new();
    for _ in 0..4 {
        for request in [
            Request::ChangedSymbols { project, revision },
            Request::Signature {
                project,
                revision,
                symbol: symbol.clone(),
            },
            Request::Impact {
                project,
                revision,
                symbol: symbol.clone(),
            },
            Request::Reachability {
                project,
                revision,
                symbol: symbol.clone(),
            },
        ] {
            let client = client.clone();
            analyses.spawn(async move { client.call(request).await });
        }
    }

    // The viewer marks files seen meanwhile.
    let mut slowest = Duration::ZERO;
    let mut toggles = 0;
    while !analyses.is_empty() {
        let path = names[toggles % FILES].clone();
        let asked = Instant::now();
        let toggled = client
            .call(Request::ToggleFileSeen {
                project,
                view: View::Changeset,
                revision,
                path,
            })
            .await;
        slowest = slowest.max(asked.elapsed());
        assert!(matches!(toggled, Ok(Response::Review(_))), "{toggled:?}");
        toggles += 1;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            Some(done) = analyses.join_next() => {
                let answer = done.unwrap();
                assert!(answer.is_ok(), "{answer:?}");
            }
        }
    }
    while let Some(done) = analyses.join_next().await {
        assert!(done.unwrap().is_ok());
    }
    println!("{toggles} toggles, the slowest took {slowest:?}");

    let mut searches = std::collections::BTreeSet::new();
    while let Ok(message) = observer.inbox.try_recv() {
        if let ServerMessage::Event {
            event: Event::Task(task),
        } = message
            && task.kind == TaskKind::Analyse
        {
            searches.insert(task.id);
        }
    }
    println!("{} analysis tasks", searches.len());
    assert_eq!(searches.len(), 1, "one search, not one per analysis");
    assert!(toggles >= 5, "the analyses were over after {toggles} markings");
    assert!(slowest < TOGGLE_BOUND, "a marking took {slowest:?}");

    // Each text of the changed files is asked of the server once, and its identity is
    // probed once, not for every text.
    assert_eq!(count(&log, "textDocument/documentSymbol"), 2 * FILES);
    assert!(
        count(&log, "--version") <= 2,
        "{} version probes",
        count(&log, "--version")
    );
}
