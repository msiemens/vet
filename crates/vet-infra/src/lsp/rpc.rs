//! JSON-RPC over a child process's stdio, with `Content-Length` framing. Only
//! what an LSP client needs: requests, notifications, and a reader task that
//! answers server requests and records progress and diagnostics.

use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lsp_types::notification::Notification;
use lsp_types::request::Request;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Notify, oneshot};

use crate::exec::{Tail, command};

pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const STDERR_LINES: usize = 20;
const CONTENT_MODIFIED: i64 = -32801;
const SERVER_CANCELLED: i64 = -32802;
/// How often a request is repeated after the server dropped it, and how long to wait before.
const TRANSIENT_RETRIES: u32 = 20;
const TRANSIENT_BACKOFF: Duration = Duration::from_millis(100);

pub(super) struct DiagnosticsState {
    /// Bumped on every publish, so a waiter can tell a fresh one.
    pub generation: u64,
    pub items: Vec<lsp_types::Diagnostic>,
}

#[derive(Default)]
pub(super) struct State {
    active_progress: HashSet<String>,
    /// `experimental/serverStatus` `quiescent`, for servers that send it.
    quiescent: Option<bool>,
    last_activity: Option<Instant>,
    /// By [`super::uri::key`] of the file.
    pub diagnostics: HashMap<String, DiagnosticsState>,
    /// `workspace/projectInitializationComplete`, which roslyn sends once it has loaded the projects it was told to.
    pub projects_loaded: bool,
    exited: bool,
}

/// An error response of the server.
struct ResponseError {
    code: i64,
    message: String,
}

impl ResponseError {
    fn from_response(error: &Value) -> Self {
        Self {
            code: error["code"].as_i64().unwrap_or(0),
            message: error["message"].as_str().unwrap_or("unknown error").to_owned(),
        }
    }

    /// The server dropped the request because a document changed under it, or
    /// because it was busy (`ContentModified`, `ServerCancelled`): asking again
    /// is what the protocol expects of a client.
    fn is_transient(&self) -> bool {
        matches!(self.code, CONTENT_MODIFIED | SERVER_CANCELLED)
    }
}

type Pending = Mutex<HashMap<i64, oneshot::Sender<Result<Value, ResponseError>>>>;

struct Shared {
    pending: Pending,
    state: Mutex<State>,
    changed: Notify,
    stderr: Mutex<Tail>,
    workspace_folders: Value,
}

pub(super) struct Connection {
    child: tokio::sync::Mutex<Child>,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    next_id: AtomicI64,
    shared: Arc<Shared>,
}

impl Connection {
    pub fn spawn(
        binary: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &[(String, String)],
        workspace_folders: Value,
    ) -> Result<Self, String> {
        let mut child = command(binary, args, Some(cwd), env)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{binary}: {e}"))?;
        let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take().expect("piped")));
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let shared = Arc::new(Shared {
            pending: Mutex::default(),
            state: Mutex::default(),
            changed: Notify::new(),
            stderr: Mutex::new(Tail::new(STDERR_LINES)),
            workspace_folders,
        });
        tokio::spawn(read_loop(stdout, shared.clone(), stdin.clone()));
        tokio::spawn(collect_stderr(stderr, shared.clone()));
        Ok(Self {
            child: tokio::sync::Mutex::new(child),
            stdin,
            next_id: AtomicI64::new(1),
            shared,
        })
    }

    pub async fn request<R: Request>(&self, params: R::Params) -> Result<R::Result, String> {
        self.request_with_timeout::<R>(params, REQUEST_TIMEOUT).await
    }

    pub async fn request_with_timeout<R: Request>(
        &self,
        params: R::Params,
        timeout: Duration,
    ) -> Result<R::Result, String> {
        let params = serde_json::to_value(params).map_err(|e| e.to_string())?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut retries = 0;
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = oneshot::channel();
            self.shared.pending.lock().expect("not poisoned").insert(id, tx);
            let message = json!({"jsonrpc": "2.0", "id": id, "method": R::METHOD, "params": params});
            if let Err(e) = write_message(&self.stdin, &message).await {
                self.shared.pending.lock().expect("not poisoned").remove(&id);
                return Err(self.describe(format!("{}: {e}", R::METHOD)));
            }
            match tokio::time::timeout_at(deadline, rx).await {
                Ok(Ok(Ok(value))) => {
                    return serde_json::from_value(value).map_err(|e| format!("{}: bad response: {e}", R::METHOD));
                }
                Ok(Ok(Err(error))) if error.is_transient() && retries < TRANSIENT_RETRIES => {
                    retries += 1;
                    tokio::time::sleep(TRANSIENT_BACKOFF).await;
                }
                Ok(Ok(Err(error))) => return Err(format!("{}: {}", R::METHOD, error.message)),
                Ok(Err(_)) => return Err(self.describe(format!("{}: server exited", R::METHOD))),
                Err(_) => {
                    self.shared.pending.lock().expect("not poisoned").remove(&id);
                    return Err(format!("{}: timed out after {}s", R::METHOD, timeout.as_secs()));
                }
            }
        }
    }

    pub async fn notify<N: Notification>(&self, params: N::Params) -> Result<(), String> {
        self.notify_raw(N::METHOD, serde_json::to_value(params).map_err(|e| e.to_string())?)
            .await
    }

    /// A notification outside the LSP types, such as a server's own extension.
    pub async fn notify_raw(&self, method: &str, params: Value) -> Result<(), String> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        write_message(&self.stdin, &message)
            .await
            .map_err(|e| self.describe(format!("{method}: {e}")))
    }

    /// Adds the server's last stderr lines to an error.
    pub fn describe(&self, error: String) -> String {
        let tail = self.shared.stderr.lock().expect("not poisoned");
        if tail.is_empty() {
            error
        } else {
            format!("{error}\nserver stderr:\n{}", tail.text())
        }
    }

    pub fn with_state<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.shared.state.lock().expect("not poisoned"))
    }

    /// Waits until `ready` holds for the state, re-checking on every server
    /// message, until `timeout`. `false` on timeout or if the server exited.
    pub async fn wait_for(&self, timeout: Duration, mut ready: impl FnMut(&State) -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.shared.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.shared.state.lock().expect("not poisoned");
                if ready(&state) {
                    return true;
                }
                if state.exited {
                    return false;
                }
            }
            // Wake periodically too: "quiet for N ms" has no event of its own.
            let wake = deadline.min(tokio::time::Instant::now() + Duration::from_millis(100));
            let _ = tokio::time::timeout_at(wake, notified).await;
            if tokio::time::Instant::now() >= deadline {
                return ready(&self.shared.state.lock().expect("not poisoned"));
            }
        }
    }

    /// `shutdown` then `exit`; kills the process if it lingers.
    pub async fn close(&self) {
        let _ = self
            .request_with_timeout::<lsp_types::request::Shutdown>((), Duration::from_secs(5))
            .await;
        let _ = self.notify::<lsp_types::notification::Exit>(()).await;
        let mut child = self.child.lock().await;
        if tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    }
}

impl State {
    pub fn touch(&mut self) {
        self.last_activity = Some(Instant::now());
    }

    /// Indexing is done: no work in progress, the server (if it reports it) is
    /// quiescent, and nothing has happened for `quiet`.
    pub fn is_idle(&self, quiet: Duration) -> bool {
        self.active_progress.is_empty()
            && self.quiescent != Some(false)
            && self.last_activity.is_some_and(|t| t.elapsed() >= quiet)
    }
}

async fn write_message(stdin: &tokio::sync::Mutex<ChildStdin>, message: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    let mut stdin = stdin.lock().await;
    stdin
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    stdin.write_all(&body).await?;
    stdin.flush().await
}

async fn read_message(reader: &mut BufReader<ChildStdout>) -> std::io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length:") {
            length = n.trim().parse::<usize>().ok();
        }
    }
    let length = length.ok_or_else(|| std::io::Error::other("message without Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    Ok(Some(serde_json::from_slice(&body)?))
}

async fn read_loop(stdout: ChildStdout, shared: Arc<Shared>, stdin: Arc<tokio::sync::Mutex<ChildStdin>>) {
    let mut reader = BufReader::new(stdout);
    while let Ok(Some(message)) = read_message(&mut reader).await {
        let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = message.get("id").cloned();
        match (method, id) {
            (Some(method), Some(id)) => {
                let reply = match server_request(&shared, &method, &message["params"]) {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err(()) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": format!("unsupported request {method}")},
                    }),
                };
                if write_message(&stdin, &reply).await.is_err() {
                    break;
                }
            }
            (Some(method), None) => notification(&shared, &method, &message["params"]),
            (None, Some(id)) => {
                let result = match message.get("error") {
                    Some(error) => Err(ResponseError::from_response(error)),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let sender = id
                    .as_i64()
                    .and_then(|id| shared.pending.lock().expect("not poisoned").remove(&id));
                if let Some(tx) = sender {
                    let _ = tx.send(result);
                }
            }
            (None, None) => {}
        }
    }
    shared.pending.lock().expect("not poisoned").clear();
    shared.state.lock().expect("not poisoned").exited = true;
    shared.changed.notify_waiters();
}

fn server_request(shared: &Shared, method: &str, params: &Value) -> Result<Value, ()> {
    match method {
        "window/workDoneProgress/create" | "client/registerCapability" | "client/unregisterCapability" => {
            Ok(Value::Null)
        }
        "workspace/configuration" => {
            let items = params["items"].as_array().map_or(0, Vec::len);
            Ok(Value::Array(vec![Value::Null; items]))
        }
        "workspace/workspaceFolders" => Ok(shared.workspace_folders.clone()),
        "workspace/applyEdit" => Ok(json!({"applied": false})),
        "window/showMessageRequest" => Ok(Value::Null),
        "window/showDocument" => Ok(json!({"success": false})),
        _ => Err(()),
    }
}

fn notification(shared: &Shared, method: &str, params: &Value) {
    let mut state = shared.state.lock().expect("not poisoned");
    match method {
        "$/progress" => {
            let token = params["token"].to_string();
            match params["value"]["kind"].as_str() {
                Some("begin") => {
                    state.active_progress.insert(token);
                }
                Some("end") => {
                    state.active_progress.remove(&token);
                }
                _ => {}
            }
            state.touch();
        }
        "workspace/projectInitializationComplete" => {
            state.projects_loaded = true;
            state.touch();
        }
        "experimental/serverStatus" => {
            state.quiescent = params["quiescent"].as_bool();
            state.touch();
        }
        "textDocument/publishDiagnostics" => {
            if let Ok(p) = serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params.clone()) {
                let entry = state
                    .diagnostics
                    .entry(super::uri::key(&p.uri))
                    .or_insert(DiagnosticsState {
                        generation: 0,
                        items: Vec::new(),
                    });
                entry.generation += 1;
                entry.items = p.diagnostics;
            }
        }
        _ => {}
    }
    drop(state);
    shared.changed.notify_waiters();
}

async fn collect_stderr(stderr: tokio::process::ChildStderr, shared: Arc<Shared>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        shared.stderr.lock().expect("not poisoned").push(line);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lsp_types::request::Shutdown;

    use super::*;

    /// A shell that answers its first request with an error of `code` and every
    /// later one with `null`, whatever they ask.
    fn server_failing_once_with(code: i64) -> Connection {
        let script = format!(
            r#"n=0
while IFS= read -r line; do
  case "$line" in
    *Content-Length*)
      n=$((n+1))
      if [ $n = 1 ]; then body='{{"jsonrpc":"2.0","id":1,"error":{{"code":{code},"message":"dropped"}}}}'
      else body='{{"jsonrpc":"2.0","id":'$n',"result":null}}'; fi
      printf 'Content-Length: %s\r\n\r\n%s' ${{#body}} "$body";;
  esac
done"#
        );
        Connection::spawn("sh", &["-c".into(), script], Path::new("."), &[], Value::Null).unwrap()
    }

    #[tokio::test]
    async fn a_request_the_server_dropped_because_a_document_changed_is_asked_again() {
        let conn = server_failing_once_with(CONTENT_MODIFIED);
        conn.request::<Shutdown>(()).await.unwrap();
    }

    #[tokio::test]
    async fn any_other_error_is_the_answer() {
        let conn = server_failing_once_with(-32602);
        let error = conn.request::<Shutdown>(()).await.unwrap_err();
        assert_eq!(error, "shutdown: dropped");
    }
}
