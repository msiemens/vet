//! Authenticated loopback WebSocket server (see `ARCHITECTURE.md`).
//!
//! Binds `127.0.0.1` on an OS-chosen port with a per-launch random token. The
//! first frame on every connection must be a `Hello` carrying that token and the
//! current protocol version; anything else closes the connection. Requests on an
//! authenticated connection are handled concurrently, so a slow query never
//! blocks a hover.

mod log;
pub mod shell;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use subtle::ConstantTimeEq;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc};
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::Message;
use tracing::Instrument;
use vet_protocol::{
    ClientMessage, ErrorKind, Event, Failure, PROTOCOL_VERSION, Ready, Request, Response, ServerMessage,
};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How many messages one connection's outbox holds. A window that stops reading makes its
/// outbox fill; rather than grow without bound, the connection is closed and the window
/// connects again and resynchronises.
const OUTBOX_CAPACITY: usize = 10_000;

/// The guard of `mutex`, whether or not a thread panicked while holding it: what these locks guard
/// is the table of in-flight handlers, never left half-written.
#[allow(clippy::disallowed_methods, reason = "the one place that locks a std mutex")]
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Pushes messages to the connection a request arrived on, requests' replies and events alike
/// through one ordered outbox. A handler that opens a project keeps a clone and sends that
/// project's events through it; sends after the connection closed are dropped. A send that finds
/// the outbox full does not wait: it marks the connection to be closed, and the window connects
/// again and resynchronises.
#[derive(Clone)]
pub struct Events {
    messages: mpsc::Sender<ServerMessage>,
    overflowed: Arc<Notify>,
}

impl Events {
    /// An `Events` that is not tied to a connection, and what it receives. Nothing closes a
    /// channel that is not a connection's, so a send that finds it full is dropped.
    pub fn channel() -> (Self, mpsc::Receiver<ServerMessage>) {
        Self::connected(OUTBOX_CAPACITY)
    }

    /// The outbox of one connection, holding at most `capacity` messages, and the signal that
    /// raises when it overflows.
    fn connected(capacity: usize) -> (Self, mpsc::Receiver<ServerMessage>) {
        let (messages, receiver) = mpsc::channel(capacity);
        let events = Self {
            messages,
            overflowed: Arc::new(Notify::new()),
        };
        (events, receiver)
    }

    /// Queues a message for the writer. A full outbox is not buffered for: it marks the
    /// connection to close, without the sender ever waiting.
    pub(crate) fn message(&self, message: ServerMessage) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.messages.try_send(message) {
            self.overflowed.notify_one();
        }
    }

    pub fn send(&self, event: Event) {
        self.message(ServerMessage::Event { event });
    }

    /// Whether the connection is gone.
    pub fn is_closed(&self) -> bool {
        self.messages.is_closed()
    }

    /// Whether both belong to the same connection.
    pub fn same_connection(&self, other: &Events) -> bool {
        self.messages.same_channel(&other.messages)
    }
}

/// What sits behind the transport: the backend's request handling.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, request: Request, events: Events) -> impl Future<Output = Result<Response, Failure>> + Send;
}

pub struct Server {
    listener: TcpListener,
    token: String,
    outbox_capacity: usize,
}

impl Server {
    /// Binds the server; every connection's outbox holds `OUTBOX_CAPACITY` messages.
    pub async fn bind() -> io::Result<Self> {
        Self::bind_with_outbox(OUTBOX_CAPACITY).await
    }

    /// Binds the server like [`Server::bind`], with an outbox of `capacity` messages per
    /// connection. A smaller one closes a window that stopped reading sooner.
    pub async fn bind_with_outbox(capacity: usize) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        Ok(Self {
            listener,
            token: random_token()?,
            outbox_capacity: capacity,
        })
    }

    /// What a client needs to connect: the port and the token.
    pub fn ready(&self) -> io::Result<Ready> {
        Ok(Ready {
            port: self.listener.local_addr()?.port(),
            token: self.token.clone(),
        })
    }

    pub async fn serve<H: Handler>(self, handler: H) -> io::Result<()> {
        let handler = Arc::new(handler);
        let token: Arc<str> = self.token.into();
        let outbox_capacity = self.outbox_capacity;
        loop {
            let (stream, _) = self.listener.accept().await?;
            let (handler, token) = (handler.clone(), token.clone());
            tokio::spawn(async move {
                if let Err(error) = connection(stream, &token, handler, outbox_capacity).await {
                    tracing::warn!(%error, "connection closed");
                }
            });
        }
    }
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(hex::encode(bytes))
}

/// Compares without short-circuiting on the first differing byte.
fn token_matches(presented: &str, expected: &str) -> bool {
    presented.as_bytes().ct_eq(expected.as_bytes()).into()
}

fn frame(message: &ServerMessage) -> Message {
    Message::text(serde_json::to_string(message).expect("server messages serialize"))
}

async fn connection<H: Handler>(
    stream: TcpStream,
    token: &str,
    handler: Arc<H>,
    outbox_capacity: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut sink, mut source) = tokio_tungstenite::accept_async(stream).await?.split();

    let first = tokio::time::timeout(HELLO_TIMEOUT, source.next())
        .await
        .map_err(|_| "no hello")?
        .ok_or("closed before hello")??;
    let Message::Text(text) = first else {
        return Err("first frame must be a text Hello".into());
    };
    match serde_json::from_str::<ClientMessage>(text.as_str()) {
        Ok(ClientMessage::Hello {
            token: presented,
            protocol_version,
        }) if token_matches(&presented, token) && protocol_version == PROTOCOL_VERSION => {}
        _ => return Err("bad hello".into()),
    }
    sink.send(frame(&ServerMessage::HelloAck {
        protocol_version: PROTOCOL_VERSION,
    }))
    .await?;

    // One writer owns the sink; request tasks send their replies and events through it. Events and
    // replies share this one queue, so an answer cannot overtake the events that came before it —
    // `WaitForQuiet` relies on that — and `try_send` keeps the order without a sender ever waiting.
    let (events, mut outbox) = Events::connected(outbox_capacity);
    let overflowed = events.overflowed.clone();
    let writer = tokio::spawn(async move {
        while let Some(message) = outbox.recv().await {
            if sink.send(frame(&message)).await.is_err() {
                break;
            }
        }
    });

    let running = Running::default();
    let mut ui_budget = log::UiBudget::new(Instant::now());
    loop {
        tokio::select! {
            // The outbox filled: the window is not reading, so it is closed instead of buffered
            // for, and connecting again is left to it.
            _ = overflowed.notified() => {
                tracing::warn!(bound = outbox_capacity, "a window is not reading; closing its connection");
                break;
            }
            incoming = source.next() => {
                let Some(message) = incoming else { break };
                let Message::Text(text) = message? else { continue };
                match serde_json::from_str::<ClientMessage>(text.as_str()) {
                    Ok(ClientMessage::Request { id, request }) => {
                        let (handler, events) = (handler.clone(), events.clone());
                        // The span carries what every line logged while the request runs is about;
                        // the handler adds the project once it knows it.
                        let span =
                            tracing::info_span!("request", id, kind = %log::kind(&request), project = tracing::field::Empty);
                        running.start(id, events.clone(), |finish| {
                            async move {
                                // Dropped when the task ends, however it ends.
                                let _finish = finish;
                                let started = Instant::now();
                                let reply = match handler.handle(request, events.clone()).await {
                                    Ok(response) => {
                                        tracing::debug!(target: "timing", took_ms = started.elapsed().as_millis() as u64, "answered");
                                        ServerMessage::Response { id, response }
                                    }
                                    Err(Failure { kind, message }) => {
                                        tracing::debug!(target: "timing", took_ms = started.elapsed().as_millis() as u64, ?kind, error = %message, "failed");
                                        ServerMessage::Error {
                                            id: Some(id),
                                            kind,
                                            message,
                                        }
                                    }
                                };
                                events.message(reply);
                            }
                            .instrument(span)
                        });
                    }
                    Ok(ClientMessage::Cancel { id }) => {
                        if running.cancel(id) {
                            tracing::debug!(id, "request cancelled");
                            events.message(ServerMessage::Cancelled { id });
                        }
                    }
                    Ok(ClientMessage::Log {
                        level,
                        message,
                        context,
                    }) => {
                        if ui_budget.admit(Instant::now()) {
                            log::from_ui(level, &message, &context);
                        }
                    }
                    Ok(ClientMessage::Hello { .. }) => {
                        events.message(ServerMessage::Error {
                            id: None,
                            kind: ErrorKind::Invalid,
                            message: "already authenticated".into(),
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "malformed message");
                        events.message(ServerMessage::Error {
                            id: None,
                            kind: ErrorKind::Invalid,
                            message: format!("malformed message: {error}"),
                        });
                    }
                }
            }
        }
    }
    // A handler may still hold an `Events` clone, so the outbox never closes by itself.
    writer.abort();
    Ok(())
}

type Table = Arc<Mutex<HashMap<u32, AbortHandle>>>;

/// The handlers of one connection that have not answered yet, by request id.
/// Dropping it aborts them all, so a closed connection leaves none running; an
/// aborted handler drops its futures at the next await, which is how a query
/// down in the language server layer learns it is no longer wanted.
#[derive(Default)]
struct Running(Table);

impl Running {
    /// Runs `handler`, which gets what removes its entry once it ends, as the task of request
    /// `id`. A handler that panics is answered with an error on `replies`.
    fn start<F: Future<Output = ()> + Send + 'static>(
        &self,
        id: u32,
        replies: Events,
        handler: impl FnOnce(Finish) -> F,
    ) {
        // Held across the spawn, so a task that ends at once finds its entry.
        let mut table = lock(&self.0);
        let task = tokio::spawn(handler(Finish {
            table: self.0.clone(),
            id,
            replies,
        }));
        table.insert(id, task.abort_handle());
    }

    /// Aborts the handler of `id`; whether it was still running.
    fn cancel(&self, id: u32) -> bool {
        // The lock is released before the abort: a handler's drop takes it too.
        let handle = lock(&self.0).remove(&id);
        match handle {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let handles: Vec<_> = lock(&self.0).drain().map(|(_, h)| h).collect();
        for handle in handles {
            handle.abort();
        }
    }
}

/// Takes a handler that ended out of its connection's table when it is dropped, with the handler's
/// future: after an answer, a cancel, or a panic, which the viewer waiting for the answer is told of.
struct Finish {
    table: Table,
    id: u32,
    replies: Events,
}

impl Drop for Finish {
    fn drop(&mut self) {
        if std::thread::panicking() {
            tracing::error!(id = self.id, "a request panicked");
            self.replies.message(ServerMessage::Error {
                id: Some(self.id),
                kind: ErrorKind::Internal,
                message: "the request failed unexpectedly".into(),
            });
        }
        // A panic must not turn into an abort here.
        let mut table = lock(&self.table);
        table.remove(&self.id);
    }
}
