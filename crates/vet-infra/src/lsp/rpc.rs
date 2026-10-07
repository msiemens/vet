//! JSON-RPC over a child process's stdio, with `Content-Length` framing. Only
//! what an LSP client needs: requests, notifications, and a reader task that
//! answers server requests and records progress and diagnostics.
//!
//! Messages leave through one writer task, in the order they were sent, so
//! sending never awaits and cannot be cut short by a dropped caller. A request
//! whose future is dropped before its answer arrived is cancelled
//! (`$/cancelRequest`) and forgotten.

use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lsp_types::notification::Notification;
use lsp_types::request::Request;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::exec::{Tail, command};

pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const STDERR_LINES: usize = 20;
const CONTENT_MODIFIED: i64 = -32801;
const SERVER_CANCELLED: i64 = -32802;

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

/// Why a request has no result.
#[derive(Debug)]
pub(super) struct RequestError {
    pub message: String,
    transient: bool,
}

impl RequestError {
    fn fatal(message: String) -> Self {
        Self {
            message,
            transient: false,
        }
    }

    /// The server dropped the request and would take it again (see
    /// [`ResponseError::is_transient`]). Only the caller knows what to redo first.
    pub fn is_transient(&self) -> bool {
        self.transient
    }
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
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

/// Where messages to the server are queued; a task writes them in order.
#[derive(Clone)]
struct Outbox(mpsc::UnboundedSender<Vec<u8>>);

impl Outbox {
    fn send(&self, message: &Value) -> std::io::Result<()> {
        let body = serde_json::to_vec(message)?;
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend(body);
        self.0
            .send(frame)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the server no longer reads"))
    }
}

/// A request that has been sent and not answered. Dropping it (the caller gave
/// up, or the request timed out) tells the server to stop working on it.
struct InFlight<'a> {
    shared: &'a Shared,
    outbox: &'a Outbox,
    id: i64,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        // An answer takes the entry out first, and then there is nothing to cancel.
        let unanswered = self.shared.pending.lock().expect("not poisoned").remove(&self.id);
        if unanswered.is_some() {
            let _ = self
                .outbox
                .send(&json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": self.id}}));
        }
    }
}

pub(super) struct Connection {
    child: tokio::sync::Mutex<Option<Child>>,
    outbox: Outbox,
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
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let conn = Self::over(stdout, stdin, Some(child), workspace_folders);
        tokio::spawn(collect_stderr(stderr, conn.shared.clone()));
        Ok(conn)
    }

    /// A connection over any byte streams; `child` is the process behind them, if any.
    pub fn over(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
        child: Option<Child>,
        workspace_folders: Value,
    ) -> Self {
        let (queue, queued) = mpsc::unbounded_channel();
        let outbox = Outbox(queue);
        let shared = Arc::new(Shared {
            pending: Mutex::default(),
            state: Mutex::default(),
            changed: Notify::new(),
            stderr: Mutex::new(Tail::new(STDERR_LINES)),
            workspace_folders,
        });
        tokio::spawn(write_loop(writer, queued));
        tokio::spawn(read_loop(reader, shared.clone(), outbox.clone()));
        Self {
            child: tokio::sync::Mutex::new(child),
            outbox,
            next_id: AtomicI64::new(1),
            shared,
        }
    }

    /// One request. A server that drops it as transient is not asked again here:
    /// what has to be redone first is the caller's to know.
    pub async fn request<R: Request>(&self, params: R::Params) -> Result<R::Result, RequestError> {
        self.request_with_timeout::<R>(params, REQUEST_TIMEOUT).await
    }

    pub async fn request_with_timeout<R: Request>(
        &self,
        params: R::Params,
        timeout: Duration,
    ) -> Result<R::Result, RequestError> {
        let params = serde_json::to_value(params).map_err(|e| RequestError::fatal(e.to_string()))?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().expect("not poisoned").insert(id, tx);
        let _in_flight = InFlight {
            shared: &self.shared,
            outbox: &self.outbox,
            id,
        };
        let message = json!({"jsonrpc": "2.0", "id": id, "method": R::METHOD, "params": params});
        if let Err(e) = self.outbox.send(&message) {
            return Err(RequestError::fatal(self.describe(format!("{}: {e}", R::METHOD))));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => serde_json::from_value(value)
                .map_err(|e| RequestError::fatal(format!("{}: bad response: {e}", R::METHOD))),
            Ok(Ok(Err(error))) => Err(RequestError {
                transient: error.is_transient(),
                message: format!("{}: {}", R::METHOD, error.message),
            }),
            Ok(Err(_)) => Err(RequestError::fatal(
                self.describe(format!("{}: server exited", R::METHOD)),
            )),
            Err(_) => Err(RequestError::fatal(format!(
                "{}: timed out after {}s",
                R::METHOD,
                timeout.as_secs()
            ))),
        }
    }

    pub fn notify<N: Notification>(&self, params: N::Params) -> Result<(), String> {
        self.notify_raw(N::METHOD, serde_json::to_value(params).map_err(|e| e.to_string())?)
    }

    /// A notification outside the LSP types, such as a server's own extension.
    pub fn notify_raw(&self, method: &str, params: Value) -> Result<(), String> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.outbox
            .send(&message)
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
        let _ = self.notify::<lsp_types::notification::Exit>(());
        let mut child = self.child.lock().await;
        let Some(child) = child.as_mut() else { return };
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

/// Writes the queued frames until the queue closes or the server stops reading.
async fn write_loop(mut writer: impl AsyncWrite + Unpin, mut queued: mpsc::UnboundedReceiver<Vec<u8>>) {
    while let Some(frame) = queued.recv().await {
        if writer.write_all(&frame).await.is_err() || writer.flush().await.is_err() {
            break;
        }
    }
}

pub(super) async fn read_message(reader: &mut BufReader<impl AsyncRead + Unpin>) -> std::io::Result<Option<Value>> {
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

async fn read_loop(reader: impl AsyncRead + Unpin, shared: Arc<Shared>, outbox: Outbox) {
    let mut reader = BufReader::new(reader);
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
                if outbox.send(&reply).is_err() {
                    break;
                }
            }
            (Some(method), None) => notification(&shared, &method, &message["params"]),
            (None, Some(id)) => {
                let result = match message.get("error") {
                    Some(error) => Err(ResponseError::from_response(error)),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                // The answer to a cancelled request (`RequestCancelled`, or a result that
                // got there first) finds no entry: nobody is waiting for it.
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
    use std::sync::atomic::AtomicUsize;

    use lsp_types::request::Shutdown;

    use super::*;
    use crate::lsp::fake::{self, Reply};

    #[tokio::test]
    async fn only_the_errors_that_ask_for_a_repeat_are_transient() {
        for (code, transient) in [(CONTENT_MODIFIED, true), (SERVER_CANCELLED, true), (-32602, false)] {
            let (conn, _fake) = fake::start(move |_| Reply::Error(code));
            let error = conn.request::<Shutdown>(()).await.unwrap_err();
            assert_eq!(error.message, "shutdown: failed");
            assert_eq!(error.is_transient(), transient, "{code}");
        }
    }

    fn cancels(log: &[Value]) -> Vec<Value> {
        log.iter()
            .filter(|m| m["method"] == "$/cancelRequest")
            .map(|m| m["params"]["id"].clone())
            .collect()
    }

    fn pending(conn: &Connection) -> usize {
        conn.shared.pending.lock().unwrap().len()
    }

    #[tokio::test]
    async fn a_dropped_request_is_cancelled_at_the_server_and_forgotten() {
        let (conn, fake) = fake::start(|_| Reply::Hold);
        let gave_up = tokio::time::timeout(Duration::from_millis(20), conn.request::<Shutdown>(())).await;
        assert!(gave_up.is_err());
        fake.until("the cancellation", |log| !cancels(log).is_empty()).await;
        let sent = fake.messages("shutdown")[0]["id"].clone();
        assert_eq!(cancels(&fake.log()), [sent]);
        assert_eq!(pending(&conn), 0);
    }

    #[tokio::test]
    async fn a_request_that_times_out_is_cancelled_too() {
        let (conn, fake) = fake::start(|_| Reply::Hold);
        let error = conn
            .request_with_timeout::<Shutdown>((), Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(error.message.contains("timed out"), "{error}");
        fake.until("the cancellation", |log| !cancels(log).is_empty()).await;
        assert_eq!(pending(&conn), 0);
    }

    #[tokio::test]
    async fn an_answered_request_is_not_cancelled() {
        let (conn, fake) = fake::start(|_| Reply::Result(Value::Null));
        conn.request::<Shutdown>(()).await.unwrap();
        // The request is the last thing the server could have heard before a cancel.
        conn.notify_raw("probe", Value::Null).unwrap();
        fake.until("the probe", |log| log.iter().any(|m| m["method"] == "probe"))
            .await;
        assert!(cancels(&fake.log()).is_empty());
    }

    #[tokio::test]
    async fn a_late_answer_to_a_cancelled_request_is_ignored() {
        let asked = AtomicUsize::new(0);
        let (conn, fake) = fake::start(move |_| match asked.fetch_add(1, Ordering::SeqCst) {
            0 => Reply::Hold,
            _ => Reply::Result(Value::Null),
        });
        let gave_up = tokio::time::timeout(Duration::from_millis(20), conn.request::<Shutdown>(())).await;
        assert!(gave_up.is_err());
        fake.until("the cancellation", |log| !cancels(log).is_empty()).await;
        let first = cancels(&fake.log())[0].as_i64().unwrap();

        // The server answers the cancelled request, with the error it is meant to
        // use and with a result; the connection carries on.
        fake.reply(first, Reply::Error(-32800)).await;
        fake.reply(first, Reply::Result(json!("stale"))).await;
        conn.request::<Shutdown>(()).await.unwrap();
        assert_eq!(pending(&conn), 0);
        assert!(!conn.with_state(|s| s.exited));
    }
}
