use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use vet_log::capture::Capture;
use vet_protocol::{ClientMessage, LogLevel, PROTOCOL_VERSION, Request, Response, ServerMessage};
use vet_transport::{Events, Handler, Server};

struct Chatty;

impl Handler for Chatty {
    async fn handle(&self, request: Request, _: Events) -> Result<Response, String> {
        let Request::Ping { payload } = request else {
            return Err("unsupported".into());
        };
        tracing::info!("handling the ping");
        Ok(Response::Pong { payload })
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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

/// Logs on this thread, which the server's tasks run on too, and connects.
async fn connected() -> (Capture, Socket, tracing::subscriber::DefaultGuard) {
    let capture = Capture::default();
    let guard = capture.install(vet_log::filter(None).0);
    let server = Server::bind().await.unwrap();
    let ready = server.ready().unwrap();
    tokio::spawn(server.serve(Chatty));
    let mut socket = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", ready.port))
        .await
        .unwrap()
        .0;
    let hello = ClientMessage::Hello {
        token: ready.token,
        protocol_version: PROTOCOL_VERSION,
    };
    send(&mut socket, &hello).await;
    receive(&mut socket).await;
    (capture, socket, guard)
}

/// Once the answer to a ping is in, every message sent before it was handled.
async fn settled(socket: &mut Socket, id: u32) {
    let request = Request::Ping { payload: "sync".into() };
    send(socket, &ClientMessage::Request { id, request }).await;
    assert!(matches!(receive(socket).await, ServerMessage::Response { .. }));
}

fn ui_entry(level: LogLevel, message: &str) -> ClientMessage {
    ClientMessage::Log {
        level,
        message: message.into(),
        context: "window.onerror".into(),
    }
}

#[tokio::test]
async fn what_a_handler_logs_carries_the_request_it_answers() {
    let (capture, mut socket, _guard) = connected().await;
    settled(&mut socket, 41).await;
    let text = capture.text();
    assert!(text.contains("request{id=41 kind=Ping}"), "{text}");
    assert!(text.contains("handling the ping"), "{text}");
}

#[tokio::test]
async fn an_entry_of_the_ui_is_logged_under_its_own_target() {
    let (capture, mut socket, _guard) = connected().await;
    send(&mut socket, &ui_entry(LogLevel::Error, "x is undefined")).await;
    settled(&mut socket, 1).await;
    let text = capture.text();
    assert!(text.contains("ERROR ui: x is undefined"), "{text}");
    assert!(text.contains("context=window.onerror"), "{text}");
}

#[tokio::test]
async fn a_ui_that_floods_the_log_gets_only_a_part_of_it_logged() {
    let (capture, mut socket, _guard) = connected().await;
    for n in 0..500 {
        send(&mut socket, &ui_entry(LogLevel::Warn, &format!("entry {n}"))).await;
    }
    settled(&mut socket, 1).await;
    let logged = capture.text().matches("WARN ui:").count();
    assert!((1..500).contains(&logged), "{logged}");
}
