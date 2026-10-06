//! Authenticated loopback WebSocket server (SPEC §4.1).
//!
//! Binds `127.0.0.1` on an OS-chosen port with a per-launch random token. The
//! first frame on every connection must be a `Hello` carrying that token and the
//! current protocol version; anything else closes the connection. Requests on an
//! authenticated connection are handled concurrently, so a slow query never
//! blocks a hover.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use vet_protocol::{ClientMessage, Event, PROTOCOL_VERSION, Ready, Request, Response, ServerMessage};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Pushes events to the connection a request arrived on. A handler that opens a
/// session keeps a clone and sends that session's events through it; sends after
/// the connection closed are dropped.
#[derive(Clone)]
pub struct Events(mpsc::UnboundedSender<ServerMessage>);

impl Events {
    pub fn send(&self, event: Event) {
        let _ = self.0.send(ServerMessage::Event { event });
    }
}

/// What sits behind the transport: the real orchestrator or the spike's mock.
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

    /// The line the backend prints to stdout for the shell to read.
    pub fn ready(&self) -> io::Result<Ready> {
        Ok(Ready {
            port: self.listener.local_addr()?.port(),
            token: self.token.clone(),
        })
    }

    /// Prints the `Ready` line to stdout for the shell, and to stderr the
    /// address a plain browser needs during development.
    pub fn announce(&self) -> io::Result<()> {
        let ready = self.ready()?;
        println!("{}", serde_json::to_string(&ready).expect("ready serializes"));
        eprintln!(
            "dev UI: http://localhost:1420/#port={}&token={}",
            ready.port, ready.token
        );
        Ok(())
    }

    pub async fn serve<H: Handler>(self, handler: H) -> io::Result<()> {
        let handler = Arc::new(handler);
        let token: Arc<str> = self.token.into();
        loop {
            let (stream, _) = self.listener.accept().await?;
            let (handler, token) = (handler.clone(), token.clone());
            tokio::spawn(async move {
                if let Err(error) = connection(stream, &token, handler).await {
                    eprintln!("connection closed: {error}");
                }
            });
        }
    }
}

/// Exits the process when stdin reaches EOF. The shell keeps the backend's stdin
/// pipe open, so a shell that dies without killing its child (Ctrl-C, crash) does
/// not leave the backend running, which on Windows also locks its .exe.
pub fn exit_when_stdin_closes() {
    std::thread::spawn(|| {
        let _ = io::copy(&mut io::stdin(), &mut io::sink());
        std::process::exit(0);
    });
}

fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Compares without short-circuiting on the first differing byte.
fn token_matches(presented: &str, expected: &str) -> bool {
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
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

    while let Some(message) = source.next().await {
        let Message::Text(text) = message? else { continue };
        match serde_json::from_str::<ClientMessage>(text.as_str()) {
            Ok(ClientMessage::Request { id, request }) => {
                let (handler, replies) = (handler.clone(), replies.clone());
                tokio::spawn(async move {
                    let reply = match handler.handle(request, Events(replies.clone())).await {
                        Ok(response) => ServerMessage::Response { id, response },
                        Err(message) => ServerMessage::Error { id: Some(id), message },
                    };
                    let _ = replies.send(reply);
                });
            }
            Ok(ClientMessage::Hello { .. }) => {
                let _ = replies.send(ServerMessage::Error {
                    id: None,
                    message: "already authenticated".into(),
                });
            }
            Err(error) => {
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
