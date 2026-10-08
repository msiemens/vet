//! A review under load: the symbols panel asks for the analyses of several symbols at
//! once while the language server is slow, and the viewer goes on marking files seen.
//! Runs the backend on the real store with a language server that takes its time over
//! every file; needs `python3` for that server and is skipped without it.

#![allow(clippy::disallowed_methods)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Client, config, repo_with};
use serde_json::Value;
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{Event, Request, Response, ServerMessage, Side, SymbolId, TaskKind, ThreadTarget, View};

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
async fn many_analyses_of_one_revision_share_one_search_while_files_are_marked_seen() {
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

    // Each text of the changed files is asked of the server once, and its identity is
    // probed once, not for every text.
    assert_eq!(count(&log, "textDocument/documentSymbol"), 2 * FILES);
    assert!(
        count(&log, "--version") <= 2,
        "{} version probes",
        count(&log, "--version")
    );
}

/// A server that answers nothing until a gate file exists: `--version` and `initialize` wait
/// for the start gate, every other request (not notification) for the request gate. What it receives is appended
/// to a log, as `SLOW_SERVER` does.
const GATED_SERVER: &str = r#"#!/usr/bin/env python3
import json, os, sys, time

LOG = "@LOG@"
START_GATE = "@START_GATE@"
REQUEST_GATE = "@REQUEST_GATE@"

def log(entry):
    with open(LOG, "a") as out:
        out.write(json.dumps(entry) + "\n")

def wait_for_gate(gate):
    while not os.path.exists(gate):
        time.sleep(0.02)

if "--version" in sys.argv:
    log({"method": "--version"})
    wait_for_gate(START_GATE)
    print("gated-server 1.0")
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
    method = message.get("method")
    log({"method": method})
    if "id" in message:
        wait_for_gate(START_GATE if method == "initialize" else REQUEST_GATE)
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": CAPABILITIES}})
    elif method == "exit":
        break
    elif method == "textDocument/documentSymbol":
        send({"jsonrpc": "2.0", "id": message["id"], "result": [SYMBOL]})
    elif "id" in message and method:
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
"#;

/// The files the gated servers wait for. Dropping the gates opens them, so that servers of a
/// test that failed do not wait forever.
struct Gates {
    start: PathBuf,
    requests: PathBuf,
}

impl Gates {
    fn open_start(&self) {
        std::fs::write(&self.start, "").unwrap();
    }

    fn open_requests(&self) {
        std::fs::write(&self.requests, "").unwrap();
    }
}

impl Drop for Gates {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.start, "");
        let _ = std::fs::write(&self.requests, "");
    }
}

const CHANGED_FILES: usize = 300;

fn borrowed(files: &[(String, String)]) -> Vec<(&str, &str)> {
    files.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect()
}

/// A review request answers within this long. It is not a latency target (`< 100 ms` is
/// SPEC §13's): it only turns a request that waits for stuck semantic work into a failure
/// and not a hang. Work that is stuck on a gate never ends by itself, so the bound is far
/// above what the first mark costs (the diff of 300 files, seconds in a debug build, more
/// in a parallel run), and the result does not depend on the machine's load.
const ANSWERED: Duration = Duration::from_secs(120);

/// Where the semantic requests are stuck.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stuck {
    /// The servers' identity probes and starts wait on a gate.
    Starts,
    /// The servers have started and their requests wait on a gate.
    Requests,
    /// Every permit of the semantic lane's budgets is held: nothing semantic gets a thread
    /// to read with or a process to probe with.
    Permits,
}

/// The servers, their identity probes and their starts wait on a gate, and 36 analyses and
/// hovers wait on them. The store, the repository and the backend are the real ones.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_actions_answer_while_the_servers_and_their_probes_are_stuck() {
    review_actions_answer_while_semantic_work_is_stuck(Stuck::Starts).await;
}

/// The servers have started, and the 36 requests are past reading the revision's sides and
/// wait on the servers' answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_actions_answer_while_the_servers_are_started_and_every_request_is_stuck() {
    review_actions_answer_while_semantic_work_is_stuck(Stuck::Requests).await;
}

/// Every permit of `probes` and `semantic_io` is taken. The review lane has budgets of its own,
/// so no review request needs anything the semantic work holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_actions_answer_while_every_semantic_permit_is_taken() {
    review_actions_answer_while_semantic_work_is_stuck(Stuck::Permits).await;
}

async fn review_actions_answer_while_semantic_work_is_stuck(stuck: Stuck) {
    if !python_available() {
        eprintln!("skipped: python3 is not installed");
        return;
    }
    let names: Vec<_> = (0..CHANGED_FILES).map(|i| format!("src/m{i}.rs")).collect();
    let text = |value: usize| format!("pub fn f() -> usize {{\n    {value}\n}}\n");
    let base: Vec<_> = names.iter().enumerate().map(|(i, n)| (n.clone(), text(i))).collect();
    let dirty: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), text(1000 + i)))
        .collect();
    let repo = repo_with(&borrowed(&base), &borrowed(&dirty));

    let tools = tempfile::tempdir().unwrap();
    let log = tools.path().join("received.jsonl");
    let gates = Gates {
        start: tools.path().join("start-gate"),
        requests: tools.path().join("request-gate"),
    };
    if stuck != Stuck::Starts {
        gates.open_start();
    }
    if stuck == Stuck::Permits {
        gates.open_requests();
    }
    let script = tools.path().join("gated-server");
    std::fs::write(
        &script,
        GATED_SERVER
            .replace("@LOG@", log.to_str().unwrap())
            .replace("@START_GATE@", gates.start.to_str().unwrap())
            .replace("@REQUEST_GATE@", gates.requests.to_str().unwrap()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Arc::new(Client::new(&handler));
    let project = client.open(repo.path()).await;
    let revision = client.revision(project).await;
    client
        .call(Request::SetServerPath {
            project,
            language: "rust".into(),
            path: Some(script.to_str().unwrap().into()),
        })
        .await
        .unwrap();

    let held = if stuck == Stuck::Permits {
        let budgets = handler.budgets();
        Some((
            budgets.semantic_io.acquire_all().await,
            budgets.probes.acquire_all().await,
        ))
    } else {
        None
    };

    let symbol = SymbolId {
        path: names[0].clone(),
        name: vec!["f".into()],
    };
    let mut semantic = tokio::task::JoinSet::new();
    for _ in 0..12 {
        for request in [
            Request::ChangedSymbols { project, revision },
            Request::Hover {
                project,
                revision,
                side: Side::Target,
                path: names[0].clone(),
                line: 0,
                col: 7,
            },
            Request::Impact {
                project,
                revision,
                symbol: symbol.clone(),
            },
        ] {
            let client = client.clone();
            semantic.spawn(async move { client.call(request).await });
        }
    }
    // Until the probes have been asked for, or the servers their first request.
    let first = match stuck {
        Stuck::Starts => Some("--version"),
        Stuck::Requests => Some("textDocument/hover"),
        Stuck::Permits => None,
    };
    if let Some(first) = first {
        let started = async {
            while count(&log, first) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(ANSWERED, started)
            .await
            .expect("nothing semantic started");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(semantic.try_join_next().is_none(), "the semantic requests are stuck");
    let slowest = std::cell::Cell::new(Duration::ZERO);
    let review = async |request: Request| {
        let asked = Instant::now();
        let answer = tokio::time::timeout(ANSWERED, client.call(request))
            .await
            .expect("a review request waited for semantic work");
        slowest.set(slowest.get().max(asked.elapsed()));
        answer
    };
    for (marked, path) in names.iter().take(16).enumerate() {
        let Ok(Response::Review(update)) = review(Request::ToggleHunk {
            project,
            view: View::Changeset,
            revision,
            path: path.clone(),
            hunk: 0,
        })
        .await
        else {
            panic!("{path} was not marked");
        };
        assert_eq!(update.changeset.seen_hunks as usize, marked + 1, "after marking {path}");
    }
    let Ok(Response::Manifest(manifest)) = review(Request::Manifest {
        project,
        view: View::Changeset,
    })
    .await
    else {
        panic!("no manifest");
    };
    assert_eq!(manifest.files.len(), CHANGED_FILES);
    assert_eq!(manifest.completion.seen_hunks, 16, "the marks are stored");

    let Ok(Response::Thread(thread)) = review(Request::CreateThread {
        project,
        view: View::Changeset,
        revision,
        target: ThreadTarget::Range {
            side: Side::Target,
            path: names[0].clone(),
            start: 1,
            len: 1,
        },
        body: "why?".into(),
    })
    .await
    else {
        panic!("no thread");
    };
    let Ok(Response::Threads { threads, .. }) = review(Request::Threads {
        project,
        view: View::Changeset,
    })
    .await
    else {
        panic!("no threads");
    };
    assert_eq!(threads.iter().map(|t| t.id).collect::<Vec<_>>(), [thread.id]);
    println!("the slowest review request took {:?}", slowest.get());

    // Nothing was dropped on the way: with the servers and the permits released, the analyses
    // answer too.
    drop(held);
    gates.open_start();
    gates.open_requests();
    let answered = async {
        while let Some(done) = semantic.join_next().await {
            let answer = done.unwrap();
            assert!(answer.is_ok(), "{answer:?}");
        }
    };
    tokio::time::timeout(Duration::from_secs(120), answered)
        .await
        .expect("the semantic requests did not finish after the gate opened");
}
