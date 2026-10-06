use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use vet_protocol::{ClientMessage, Event, PROTOCOL_VERSION, Request, Response, ServerMessage};
use vet_transport::{Events, Handler, Server};

struct Echo;

impl Handler for Echo {
    async fn handle(&self, request: Request, events: Events) -> Result<Response, String> {
        match request {
            Request::Ping { payload } => Ok(Response::Pong { payload }),
            Request::OpenSession { .. } => {
                events.send(Event::SnapshotSettled {
                    session: 3,
                    revision: 1,
                });
                Ok(Response::Session { session: 3 })
            }
            _ => Err("unsupported".into()),
        }
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn start() -> (u16, String) {
    let server = Server::bind().await.unwrap();
    let ready = server.ready().unwrap();
    tokio::spawn(server.serve(Echo));
    (ready.port, ready.token)
}

async fn connect(port: u16) -> Socket {
    tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
        .await
        .unwrap()
        .0
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

fn hello(token: &str) -> ClientMessage {
    ClientMessage::Hello {
        token: token.into(),
        protocol_version: PROTOCOL_VERSION,
    }
}

#[tokio::test]
async fn valid_token_allows_requests() {
    let (port, token) = start().await;
    let mut socket = connect(port).await;
    send(&mut socket, &hello(&token)).await;
    assert_eq!(
        receive(&mut socket).await,
        Some(ServerMessage::HelloAck {
            protocol_version: PROTOCOL_VERSION
        })
    );

    send(
        &mut socket,
        &ClientMessage::Request {
            id: 7,
            request: Request::Ping { payload: "hi".into() },
        },
    )
    .await;
    assert_eq!(
        receive(&mut socket).await,
        Some(ServerMessage::Response {
            id: 7,
            response: Response::Pong { payload: "hi".into() }
        })
    );
}

#[tokio::test]
async fn handlers_push_events_to_the_connection() {
    let (port, token) = start().await;
    let mut socket = connect(port).await;
    send(&mut socket, &hello(&token)).await;
    receive(&mut socket).await;

    send(
        &mut socket,
        &ClientMessage::Request {
            id: 1,
            request: Request::OpenSession { repo: ".".into() },
        },
    )
    .await;
    let mut got = [receive(&mut socket).await.unwrap(), receive(&mut socket).await.unwrap()];
    got.sort_by_key(|m| matches!(m, ServerMessage::Event { .. }));
    assert_eq!(
        got[0],
        ServerMessage::Response {
            id: 1,
            response: Response::Session { session: 3 }
        }
    );
    assert_eq!(
        got[1],
        ServerMessage::Event {
            event: Event::SnapshotSettled {
                session: 3,
                revision: 1
            }
        }
    );
}

#[tokio::test]
async fn wrong_token_is_closed_without_an_ack() {
    let (port, _) = start().await;
    let mut socket = connect(port).await;
    send(&mut socket, &hello("not-the-token")).await;
    assert_eq!(receive(&mut socket).await, None);
}

#[tokio::test]
async fn request_before_hello_is_closed() {
    let (port, _) = start().await;
    let mut socket = connect(port).await;
    send(
        &mut socket,
        &ClientMessage::Request {
            id: 1,
            request: Request::Ping { payload: "x".into() },
        },
    )
    .await;
    assert_eq!(receive(&mut socket).await, None);
}
