use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use vet_protocol::{ClientMessage, ErrorKind, Failure, PROTOCOL_VERSION, Request, Response, ServerMessage};
use vet_transport::{Events, Handler, Server};

/// Answers `Ping` at once, except a payload of `"hang"`, which never ends and
/// counts itself while it runs, so a test sees when its future is dropped.
#[derive(Clone, Default)]
struct Hanging {
    running: Arc<AtomicUsize>,
}

struct Running(Arc<AtomicUsize>);

impl Running {
    fn new(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Handler for Hanging {
    async fn handle(&self, request: Request, _: Events) -> Result<Response, Failure> {
        let Request::Ping { payload } = request else {
            return Err(Failure::new(ErrorKind::Invalid, "unsupported"));
        };
        if payload == "panic" {
            panic!("a handler that fails");
        }
        if payload == "hang" {
            let _running = Running::new(&self.running);
            std::future::pending::<()>().await;
        }
        Ok(Response::Pong { payload })
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connected(handler: Hanging) -> Socket {
    let server = Server::bind().await.unwrap();
    let ready = server.ready().unwrap();
    tokio::spawn(server.serve(handler));
    let mut socket = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", ready.port))
        .await
        .unwrap()
        .0;
    send(
        &mut socket,
        &ClientMessage::Hello {
            token: ready.token,
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await;
    receive(&mut socket).await;
    socket
}

async fn send(socket: &mut Socket, message: &ClientMessage) {
    socket
        .send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket) -> ServerMessage {
    let Some(Ok(Message::Text(text))) = socket.next().await else {
        panic!("connection closed");
    };
    serde_json::from_str(text.as_str()).unwrap()
}

fn ping(id: u32, payload: &str) -> ClientMessage {
    ClientMessage::Request {
        id,
        request: Request::Ping {
            payload: payload.into(),
        },
    }
}

async fn until(what: &str, condition: impl Fn() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never happened: {what}");
}

#[tokio::test]
async fn cancel_drops_the_handler_of_that_request_alone_and_is_answered() {
    let handler = Hanging::default();
    let mut socket = connected(handler.clone()).await;
    send(&mut socket, &ping(1, "hang")).await;
    send(&mut socket, &ping(2, "hang")).await;
    until("both handlers running", || handler.running.load(Ordering::SeqCst) == 2).await;

    send(&mut socket, &ClientMessage::Cancel { id: 1 }).await;
    assert_eq!(receive(&mut socket).await, ServerMessage::Cancelled { id: 1 });
    until("the handler dropped", || handler.running.load(Ordering::SeqCst) == 1).await;

    // The other request is unaffected, and the connection still serves.
    send(&mut socket, &ping(3, "hi")).await;
    assert_eq!(
        receive(&mut socket).await,
        ServerMessage::Response {
            id: 3,
            response: Response::Pong { payload: "hi".into() }
        }
    );
    assert_eq!(handler.running.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancelling_an_answered_request_is_ignored() {
    let mut socket = connected(Hanging::default()).await;
    send(&mut socket, &ping(1, "hi")).await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Response { id: 1, .. }
    ));

    send(&mut socket, &ClientMessage::Cancel { id: 1 }).await;
    send(&mut socket, &ping(2, "next")).await;
    // No `Cancelled` for the answered request comes first.
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Response { id: 2, .. }
    ));
}

#[tokio::test]
async fn a_handler_that_panics_is_answered_with_an_error_and_forgotten() {
    let mut socket = connected(Hanging::default()).await;
    send(&mut socket, &ping(1, "panic")).await;
    let answer = tokio::time::timeout(Duration::from_secs(10), receive(&mut socket))
        .await
        .expect("the panicking request was never answered");
    assert!(matches!(
        answer,
        ServerMessage::Error {
            id: Some(1),
            kind: ErrorKind::Internal,
            ..
        }
    ));

    // Nothing of request 1 is left to cancel, and the connection still serves.
    send(&mut socket, &ClientMessage::Cancel { id: 1 }).await;
    send(&mut socket, &ping(2, "next")).await;
    assert!(matches!(
        receive(&mut socket).await,
        ServerMessage::Response { id: 2, .. }
    ));
}

#[tokio::test]
async fn a_disconnect_aborts_every_handler() {
    let handler = Hanging::default();
    let mut socket = connected(handler.clone()).await;
    for id in 0..3 {
        send(&mut socket, &ping(id, "hang")).await;
    }
    until("all handlers running", || handler.running.load(Ordering::SeqCst) == 3).await;

    drop(socket);
    until("every handler dropped", || handler.running.load(Ordering::SeqCst) == 0).await;
}
