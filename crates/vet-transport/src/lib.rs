//! Authenticated loopback WebSocket server (SPEC §4.1).
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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use subtle::ConstantTimeEq;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::Message;
use tracing::Instrument;
use vet_protocol::{ClientMessage, Event, PROTOCOL_VERSION, Ready, Request, Response, ServerMessage};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Pushes events to the connection a request arrived on. A handler that opens a
/// project keeps a clone and sends that project's events through it; sends after
/// the connection closed are dropped.
#[derive(Clone)]
pub struct Events(mpsc::UnboundedSender<ServerMessage>);

impl Events {
    /// An `Events` that is not tied to a connection, and what it receives.
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<ServerMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self(tx), rx)
    }

    pub fn send(&self, event: Event) {
        let _ = self.0.send(ServerMessage::Event { event });
    }

    /// Whether the connection is gone.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Whether both belong to the same connection.
    pub fn same_connection(&self, other: &Events) -> bool {
        self.0.same_channel(&other.0)
    }
}

/// What sits behind the transport: the backend's request handling.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, request: Request, events: Events) -> impl Future<Output = Result<Response, String>> + Send;
}

pub struct Server {
    listener: TcpListener,
    token: String,
}

impl Server {
    pub async fn bind() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        Ok(Self {
            listener,
            token: random_token()?,
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
        loop {
            let (stream, _) = self.listener.accept().await?;
            let (handler, token) = (handler.clone(), token.clone());
            tokio::spawn(async move {
                if let Err(error) = connection(stream, &token, handler).await {
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

    // One writer owns the sink; request tasks send their replies through it.
    let (replies, mut outbox) = mpsc::unbounded_channel::<ServerMessage>();
    let writer = tokio::spawn(async move {
        while let Some(message) = outbox.recv().await {
            if sink.send(frame(&message)).await.is_err() {
                break;
            }
        }
    });

    let running = Running::default();
    let mut ui_budget = log::UiBudget::new(Instant::now());
    while let Some(message) = source.next().await {
        let Message::Text(text) = message? else { continue };
        match serde_json::from_str::<ClientMessage>(text.as_str()) {
            Ok(ClientMessage::Request { id, request }) => {
                let (handler, replies) = (handler.clone(), replies.clone());
                // The span carries what every line logged while the request runs is about; the
                // handler adds the project once it knows it.
                let span =
                    tracing::info_span!("request", id, kind = %log::kind(&request), project = tracing::field::Empty);
                running.start(id, replies.clone(), |finish| {
                    async move {
                        // Dropped when the task ends, however it ends.
                        let _finish = finish;
                        let started = Instant::now();
                        let reply = match handler.handle(request, Events(replies.clone())).await {
                            Ok(response) => {
                                tracing::debug!(target: "timing", took_ms = started.elapsed().as_millis() as u64, "answered");
                                ServerMessage::Response { id, response }
                            }
                            Err(message) => {
                                tracing::debug!(target: "timing", took_ms = started.elapsed().as_millis() as u64, error = %message, "failed");
                                ServerMessage::Error { id: Some(id), message }
                            }
                        };
                        let _ = replies.send(reply);
                    }
                    .instrument(span)
                });
            }
            Ok(ClientMessage::Cancel { id }) => {
                if running.cancel(id) {
                    tracing::debug!(id, "request cancelled");
                    let _ = replies.send(ServerMessage::Cancelled { id });
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
                let _ = replies.send(ServerMessage::Error {
                    id: None,
                    message: "already authenticated".into(),
                });
            }
            Err(error) => {
                tracing::warn!(%error, "malformed message");
                let _ = replies.send(ServerMessage::Error {
                    id: None,
                    message: format!("malformed message: {error}"),
                });
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
        replies: mpsc::UnboundedSender<ServerMessage>,
        handler: impl FnOnce(Finish) -> F,
    ) {
        // Held across the spawn, so a task that ends at once finds its entry.
        let mut table = self.0.lock().expect("not poisoned");
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
        let handle = self.0.lock().expect("not poisoned").remove(&id);
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
        let handles: Vec<_> = self.0.lock().expect("not poisoned").drain().map(|(_, h)| h).collect();
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
    replies: mpsc::UnboundedSender<ServerMessage>,
}

impl Drop for Finish {
    fn drop(&mut self) {
        if std::thread::panicking() {
            tracing::error!(id = self.id, "a request panicked");
            let _ = self.replies.send(ServerMessage::Error {
                id: Some(self.id),
                message: "the request failed unexpectedly".into(),
            });
        }
        // A panic must not turn into an abort here.
        let mut table = self.table.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        table.remove(&self.id);
    }
}
