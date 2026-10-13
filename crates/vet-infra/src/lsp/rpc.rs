//! JSON-RPC over a child process's stdio, with `Content-Length` framing. Only
//! what an LSP client needs: requests, notifications, and a reader task that
//! answers server requests and records progress and diagnostics.
//!
//! Messages leave through one writer task, in the order they were sent, so
//! sending never awaits and cannot be cut short by a dropped caller. A request
//! whose future is dropped before its answer arrived is cancelled
//! (`$/cancelRequest`) and forgotten.

use std::collections::HashMap;
use std::pin::Pin;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lsp_types::Uri;
use lsp_types::notification::Notification;
use lsp_types::request::Request;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::{Notify, mpsc, oneshot};
use vet_app::progress::Running;
use vet_semantic::ports::Readiness;

use super::uri::path_of;
use crate::exec::{Tail, command};

pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const STDERR_LINES: usize = 20;
const STDERR_SETTLE: Duration = Duration::from_millis(100);
const CONTENT_MODIFIED: i64 = -32801;
const SERVER_CANCELLED: i64 = -32802;

pub(super) struct DiagnosticsState {
    /// Bumped on every publish, so a waiter can tell a fresh one.
    pub generation: u64,
    pub items: Vec<lsp_types::Diagnostic>,
}

/// One piece of work a server reports with `$/progress`.
struct Work {
    token: String,
    title: String,
    message: Option<String>,
    percentage: Option<u32>,
}

#[derive(Default)]
pub(super) struct State {
    /// Oldest first; the last is the one that reported most recently.
    active_progress: Vec<Work>,
    /// Told what the server is working on, until the launcher takes it.
    pub index: Option<Running>,
    /// `experimental/serverStatus` `quiescent`, for servers that send it.
    quiescent: Option<bool>,
    /// Bumped whenever the server begins work of its own: progress that begins, or a
    /// status that is no longer quiescent.
    work_epoch: u64,
    last_activity: Option<Instant>,
    /// By [`super::uri::key`] of the file.
    pub diagnostics: HashMap<String, DiagnosticsState>,
    /// `workspace/projectInitializationComplete`, which roslyn sends once it has loaded the projects it was told to.
    pub projects_loaded: bool,
    /// The server has asked for its settings (`workspace/configuration`) and been answered.
    pub configuration_pulled: bool,
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
    pub(super) fn is_transient(&self) -> bool {
        self.transient
    }
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

type Pending = Mutex<HashMap<i64, oneshot::Sender<Result<Value, ResponseError>>>>;

/// What the client answers a server's requests with, beyond the plain acknowledgements.
#[derive(Default)]
pub(super) struct Replies {
    /// The workspace folders, for `workspace/workspaceFolders`.
    pub workspace_folders: Value,
    /// By section: the answers to `workspace/configuration`.
    pub settings: Value,
    /// `vscode/content` is answered with the bundled schema at the address, or refused.
    pub bundled_schemas: bool,
    /// `custom/schema/request` is answered with the schemas the file name has (YAML).
    pub associate_schemas: bool,
}

struct Shared {
    pending: Pending,
    state: Mutex<State>,
    changed: Notify,
    stderr: Mutex<Tail>,
    replies: Replies,
}

impl Shared {
    fn describe(&self, error: String) -> String {
        let tail = self.stderr.lock().expect("not poisoned");
        if tail.is_empty() {
            error
        } else {
            format!("{error}\nserver stderr:\n{}", tail.text())
        }
    }
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
    pub(super) fn spawn(
        binary: &str,
        args: &[String],
        cwd: &std::path::Path,
        env: &[(String, String)],
        replies: Replies,
    ) -> Result<Self, String> {
        let mut child = command(binary, args, Some(cwd), env)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{binary}: {e}"))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let conn = Self::over(stdout, stdin, Some(child), replies);
        tokio::spawn(collect_stderr(stderr, conn.shared.clone()));
        Ok(conn)
    }

    /// A connection over any byte streams; `child` is the process behind them, if any.
    pub(super) fn over(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
        child: Option<Child>,
        replies: Replies,
    ) -> Self {
        let (queue, queued) = mpsc::unbounded_channel();
        let outbox = Outbox(queue);
        let shared = Arc::new(Shared {
            pending: Mutex::default(),
            state: Mutex::default(),
            changed: Notify::new(),
            stderr: Mutex::new(Tail::new(STDERR_LINES)),
            replies,
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
    pub(super) async fn request<R: Request>(&self, params: R::Params) -> Result<R::Result, RequestError> {
        self.request_with_timeout::<R>(params, REQUEST_TIMEOUT).await
    }

    pub(super) async fn request_with_timeout<R: Request>(
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

    pub(super) fn notify<N: Notification>(&self, params: N::Params) -> Result<(), String> {
        self.notify_raw(N::METHOD, serde_json::to_value(params).map_err(|e| e.to_string())?)
    }

    /// A notification outside the LSP types, such as a server's own extension.
    pub(super) fn notify_raw(&self, method: &str, params: Value) -> Result<(), String> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.outbox
            .send(&message)
            .map_err(|e| self.describe(format!("{method}: {e}")))
    }

    /// Adds the server's last stderr lines to an error.
    pub(super) fn describe(&self, error: String) -> String {
        self.shared.describe(error)
    }

    /// Completes when the server's output has ended, with why. Owns what it waits on.
    pub(super) fn exited(&self) -> Pin<Box<dyn Future<Output = String> + Send>> {
        let shared = self.shared.clone();
        Box::pin(async move {
            loop {
                let notified = shared.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if shared.state.lock().expect("not poisoned").exited {
                    break;
                }
                notified.await;
            }
            // Stderr is collected by a task of its own, which may not have read the last lines yet.
            tokio::time::sleep(STDERR_SETTLE).await;
            shared.describe("the language server exited".into())
        })
    }

    pub(super) fn with_state<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.shared.state.lock().expect("not poisoned"))
    }

    /// Waits until `ready` holds for the state, re-checking on every server
    /// message, until `timeout`. `false` on timeout or if the server exited.
    pub(super) async fn wait_for(&self, timeout: Duration, mut ready: impl FnMut(&State) -> bool) -> bool {
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
    pub(super) async fn close(&self) {
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
    pub(super) fn touch(&mut self) {
        self.last_activity = Some(Instant::now());
    }

    /// Applies a `$/progress` notification and tells `index` what the server is
    /// doing now: the work that reported last, with its percentage if it has one.
    fn progress(&mut self, params: &Value) {
        let token = params["token"].to_string();
        let value = &params["value"];
        let text = |key: &str| value[key].as_str().map(str::to_owned);
        let percentage = value["percentage"].as_u64().map(|p| p.min(100) as u32);
        let at = self.active_progress.iter().position(|w| w.token == token);
        match value["kind"].as_str() {
            Some("begin") => {
                self.work_epoch += 1;
                self.active_progress.retain(|w| w.token != token);
                self.active_progress.push(Work {
                    token,
                    title: text("title").unwrap_or_default(),
                    message: text("message"),
                    percentage,
                });
            }
            Some("report") => {
                if let Some(at) = at {
                    let mut work = self.active_progress.remove(at);
                    work.message = text("message").or(work.message);
                    work.percentage = percentage.or(work.percentage);
                    self.active_progress.push(work);
                }
            }
            Some("end") => {
                if let Some(at) = at {
                    self.active_progress.remove(at);
                }
            }
            _ => {}
        }
        if let Some(index) = &self.index {
            match self.active_progress.last() {
                Some(work) => {
                    let detail = match (&work.title, &work.message) {
                        (title, Some(message)) if !title.is_empty() => format!("{title}: {message}"),
                        (title, None) => title.clone(),
                        (_, Some(message)) => message.clone(),
                    };
                    index.report(work.percentage.map(|p| (p, 100)), Some(detail));
                }
                None => index.report(None, None),
            }
        }
    }

    /// Indexing is done: no work in progress, the server (if it reports it) is
    /// quiescent, and nothing has happened for `quiet`.
    pub(super) fn is_idle(&self, quiet: Duration) -> bool {
        !self.at_work() && self.last_activity.is_some_and(|t| t.elapsed() >= quiet)
    }

    /// The server reports work of its own in progress.
    fn at_work(&self) -> bool {
        !self.active_progress.is_empty() || self.quiescent == Some(false)
    }

    /// Whether answers given now are complete, and the count of work begun so far.
    pub(super) fn readiness(&self) -> Readiness {
        Readiness {
            idle: !self.at_work(),
            epoch: self.work_epoch,
        }
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
                    Err(error) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": error.code, "message": error.message},
                    }),
                };
                if outbox.send(&reply).is_err() {
                    break;
                }
                if method == "workspace/configuration" {
                    let mut state = shared.state.lock().expect("not poisoned");
                    state.configuration_pulled = true;
                    state.touch();
                    drop(state);
                    shared.changed.notify_waiters();
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

/// The error response to a request of the server.
struct Refusal {
    code: i64,
    message: String,
}

const METHOD_NOT_FOUND: i64 = -32601;
const REQUEST_FAILED: i64 = -32803;

/// The part of `settings` that `section` names (`yaml`, `yaml.schemas`), `null` if there is none.
fn section(settings: &Value, section: &str) -> Value {
    section
        .split('.')
        .try_fold(settings, |value, key| value.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

fn server_request(shared: &Shared, method: &str, params: &Value) -> Result<Value, Refusal> {
    let replies = &shared.replies;
    match method {
        "window/workDoneProgress/create" | "client/registerCapability" | "client/unregisterCapability" => {
            Ok(Value::Null)
        }
        "workspace/configuration" => {
            let items = params["items"].as_array().map_or(&[][..], Vec::as_slice);
            Ok(Value::Array(
                items
                    .iter()
                    .map(|item| {
                        item["section"]
                            .as_str()
                            .map_or(Value::Null, |s| section(&replies.settings, s))
                    })
                    .collect(),
            ))
        }
        "workspace/workspaceFolders" => Ok(replies.workspace_folders.clone()),
        "workspace/applyEdit" => Ok(json!({"applied": false})),
        "window/showMessageRequest" => Ok(Value::Null),
        "window/showDocument" => Ok(json!({"success": false})),
        "custom/schema/request" if replies.associate_schemas => {
            let resource = params.as_str().or_else(|| params[0].as_str()).unwrap_or_default();
            let path = Uri::from_str(resource).ok().and_then(|uri| path_of(&uri));
            let schemas = path.map_or_else(Vec::new, |p| crate::schemas::yaml_schemas(&p.to_string_lossy()));
            Ok(json!(schemas))
        }
        "vscode/content" if replies.bundled_schemas => {
            // The address is the only parameter, as a string or as a list of one.
            let url = params.as_str().or_else(|| params[0].as_str()).unwrap_or_default();
            crate::schemas::serve(url)
                .map(Value::String)
                .map_err(|message| Refusal {
                    code: REQUEST_FAILED,
                    message,
                })
        }
        _ => Err(Refusal {
            code: METHOD_NOT_FOUND,
            message: format!("unsupported request {method}"),
        }),
    }
}

fn notification(shared: &Shared, method: &str, params: &Value) {
    let mut state = shared.state.lock().expect("not poisoned");
    match method {
        "$/progress" => {
            state.progress(params);
            state.touch();
        }
        "workspace/projectInitializationComplete" => {
            state.projects_loaded = true;
            state.touch();
        }
        "experimental/serverStatus" => {
            let quiescent = params["quiescent"].as_bool();
            if quiescent == Some(false) && state.quiescent != Some(false) {
                state.work_epoch += 1;
            }
            state.quiescent = quiescent;
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

    fn progress(kind: &str, token: &str, more: Value) -> Value {
        let mut value = json!({"kind": kind});
        value.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
        json!({"token": token, "value": value})
    }

    #[test]
    fn the_task_follows_the_work_the_server_reported_last() {
        let tasks = vet_app::progress::Tasks::default();
        let mut state = State {
            index: Some(tasks.start(vet_app::progress::TaskKind::ServerIndex, "Indexing")),
            ..State::default()
        };
        let shown = || {
            let task = tasks.running().remove(0);
            (task.done, task.detail)
        };

        state.progress(&progress(
            "begin",
            "roots",
            json!({"title": "Roots Scanned", "message": "3/10", "percentage": 30}),
        ));
        assert_eq!(shown(), (Some(30), Some("Roots Scanned: 3/10".into())));

        state.progress(&progress("begin", "cache", json!({"title": "Priming caches"})));
        assert_eq!(shown(), (None, Some("Priming caches".into())));

        state.progress(&progress(
            "report",
            "roots",
            json!({"message": "4/10", "percentage": 40}),
        ));
        assert_eq!(shown(), (Some(40), Some("Roots Scanned: 4/10".into())));

        state.progress(&progress("end", "roots", json!({})));
        assert_eq!(shown(), (None, Some("Priming caches".into())));

        state.progress(&progress("end", "cache", json!({})));
        assert_eq!(shown(), (None, None));
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

    fn shared_with(replies: Replies) -> Shared {
        Shared {
            pending: Mutex::default(),
            state: Mutex::default(),
            changed: Notify::new(),
            stderr: Mutex::new(Tail::new(STDERR_LINES)),
            replies,
        }
    }

    #[test]
    fn configuration_is_answered_by_section_and_unknown_sections_by_null() {
        let shared = shared_with(Replies {
            settings: json!({"yaml": {"validate": true, "schemas": {"k": ["*.yml"]}}}),
            ..Replies::default()
        });
        let items = json!({"items": [{"section": "yaml"}, {"section": "yaml.schemas"}, {"section": "json"}, {}]});
        let answer = server_request(&shared, "workspace/configuration", &items).ok().unwrap();
        assert_eq!(
            answer,
            json!([{"validate": true, "schemas": {"k": ["*.yml"]}}, {"k": ["*.yml"]}, null, null])
        );
    }

    #[test]
    fn schema_addresses_are_answered_from_the_bundle_or_refused_by_name() {
        let shared = shared_with(Replies {
            bundled_schemas: true,
            ..Replies::default()
        });
        let served = "https://json.schemastore.org/github-workflow.json";
        for params in [json!(served), json!([served])] {
            let body = server_request(&shared, "vscode/content", &params).ok().unwrap();
            assert!(body.as_str().is_some_and(|b| b.contains("\"jobs\"")), "{params}");
        }
        let refused = server_request(&shared, "vscode/content", &json!("https://example.com/s.json"))
            .err()
            .unwrap();
        assert_eq!(refused.code, REQUEST_FAILED);
        assert!(refused.message.contains("example.com"), "{}", refused.message);
    }

    #[test]
    fn the_schema_requests_are_unsupported_unless_the_setup_asks_for_them() {
        let shared = shared_with(Replies::default());
        for method in ["vscode/content", "custom/schema/request"] {
            let error = server_request(&shared, method, &json!("file:///p/a.yml"))
                .err()
                .unwrap();
            assert_eq!(error.code, METHOD_NOT_FOUND, "{method}");
        }
    }

    #[test]
    fn a_yaml_file_is_associated_with_the_schemas_its_name_has() {
        let shared = shared_with(Replies {
            associate_schemas: true,
            ..Replies::default()
        });
        let answer = server_request(
            &shared,
            "custom/schema/request",
            &json!(["file:///p/.github/workflows/ci.yml"]),
        )
        .ok()
        .unwrap();
        assert_eq!(answer.as_array().map(Vec::len), Some(1), "{answer}");
        let none = server_request(&shared, "custom/schema/request", &json!(["file:///p/notes.yml"]))
            .ok()
            .unwrap();
        assert_eq!(none, json!([]));
    }
}
