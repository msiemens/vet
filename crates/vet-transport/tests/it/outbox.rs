use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use vet_protocol::{ClientMessage, ErrorKind, Event, Failure, PROTOCOL_VERSION, Request, Response, ServerMessage};
use vet_transport::{Events, Handler, Server};

/// Answers `Ping` with as many events as its payload names, then the pong. An event per
/// revision lets a reader count and order what arrived.
struct Flood;

impl Handler for Flood {
    async fn handle(&self, request: Request, events: Events) -> Result<Response, Failure> {
        let Request::Ping { payload } = request else {
            return Err(Failure::new(ErrorKind::Invalid, "unsupported"));
        };
        let count: u32 = payload.parse().unwrap_or(0);
        for revision in 0..count {
            events.send(Event::SnapshotSettled { project: 1, revision });
        }
        Ok(Response::Pong { payload })
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A server whose connections close once their outbox holds `capacity` messages.
async fn start(capacity: usize) -> (u16, String) {
    let server = Server::bind_with_outbox(capacity).await.unwrap();
    let ready = server.ready().unwrap();
    tokio::spawn(server.serve(Flood));
    (ready.port, ready.token)
}

async fn connect(port: u16, token: &str) -> Socket {
    let mut socket = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .unwrap()
        .0;
    send(
        &mut socket,
        &ClientMessage::Hello {
            token: token.into(),
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await;
    assert_eq!(
        receive(&mut socket).await,
        Some(ServerMessage::HelloAck {
            protocol_version: PROTOCOL_VERSION
        })
    );
    socket
}

async fn send(socket: &mut Socket, message: &ClientMessage) {
    socket
        .send(Message::text(serde_json::to_string(message).unwrap()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket) -> Option<ServerMessage> {
    match socket.next().await {
        Some(Ok(Message::Text(text))) => Some(serde_json::from_str(text.as_str()).unwrap()),
        _ => None,
    }
}

fn ping(id: u32, count: u32) -> ClientMessage {
    ClientMessage::Request {
        id,
        request: Request::Ping {
            payload: count.to_string(),
        },
    }
}

fn event(revision: u32) -> ServerMessage {
    ServerMessage::Event {
        event: Event::SnapshotSettled { project: 1, revision },
    }
}

/// Reads whatever a connection still sends, until it closes; panics if it stays open.
async fn read_until_closed(socket: &mut Socket) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while let Ok(incoming) = tokio::time::timeout_at(deadline, socket.next()).await {
        match incoming {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
            Some(Ok(_)) => {}
        }
    }
    panic!("the connection stayed open");
}

#[tokio::test]
async fn a_window_that_stops_reading_is_closed_while_another_keeps_its_events() {
    let (port, token) = start(4).await;
    let mut silent = connect(port, &token).await;
    let mut reading = connect(port, &token).await;

    // The silent window asks for far more than its outbox holds and never reads it.
    send(&mut silent, &ping(1, 100_000)).await;

    // The other connection on the same server is served throughout.
    send(&mut reading, &ping(1, 1)).await;
    assert_eq!(receive(&mut reading).await, Some(event(0)));
    assert_eq!(
        receive(&mut reading).await,
        Some(ServerMessage::Response {
            id: 1,
            response: Response::Pong { payload: "1".into() }
        })
    );

    read_until_closed(&mut silent).await;

    // And it still is once the silent one is gone.
    send(&mut reading, &ping(2, 1)).await;
    assert_eq!(receive(&mut reading).await, Some(event(0)));
    assert_eq!(
        receive(&mut reading).await,
        Some(ServerMessage::Response {
            id: 2,
            response: Response::Pong { payload: "1".into() }
        })
    );
}

#[tokio::test]
async fn a_window_that_reads_a_burst_below_the_bound_is_not_closed() {
    let (port, token) = start(64).await;
    let mut socket = connect(port, &token).await;

    let burst = 32;
    send(&mut socket, &ping(1, burst)).await;
    for revision in 0..burst {
        assert_eq!(receive(&mut socket).await, Some(event(revision)));
    }
    assert_eq!(
        receive(&mut socket).await,
        Some(ServerMessage::Response {
            id: 1,
            response: Response::Pong {
                payload: burst.to_string()
            }
        })
    );

    // Nothing was closed: the connection still answers.
    send(&mut socket, &ping(2, 1)).await;
    assert_eq!(receive(&mut socket).await, Some(event(0)));
    assert_eq!(
        receive(&mut socket).await,
        Some(ServerMessage::Response {
            id: 2,
            response: Response::Pong { payload: "1".into() }
        })
    );
}
