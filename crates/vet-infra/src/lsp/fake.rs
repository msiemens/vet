//! A language server in the test process, spoken to over the real framing:
//! it records what it receives and answers requests as a test says.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader, duplex};

use super::rpc::{Connection, read_message};

/// What the server does with a request.
pub(super) enum Reply {
    Result(Value),
    Error(i64),
    /// No answer, until the test gives one with [`Fake::reply`].
    Hold,
}

#[derive(Default)]
struct Received {
    log: Vec<Value>,
}

type Writer = Arc<tokio::sync::Mutex<tokio::io::DuplexStream>>;

pub(super) struct Fake {
    received: Arc<Mutex<Received>>,
    writer: Writer,
}

/// A connection to a fake server that answers every request by `behave`.
pub(super) fn start(behave: impl Fn(&Value) -> Reply + Send + 'static) -> (Connection, Fake) {
    let (client_out, server_in) = duplex(1 << 16);
    let (server_out, client_in) = duplex(1 << 16);
    let conn = Connection::over(client_in, client_out, None, Value::Null);
    let received = Arc::new(Mutex::new(Received::default()));
    let writer: Writer = Arc::new(tokio::sync::Mutex::new(server_out));
    tokio::spawn(serve(server_in, writer.clone(), received.clone(), behave));
    (conn, Fake { received, writer })
}

async fn serve(
    input: tokio::io::DuplexStream,
    writer: Writer,
    received: Arc<Mutex<Received>>,
    behave: impl Fn(&Value) -> Reply,
) {
    let mut reader = BufReader::new(input);
    while let Ok(Some(message)) = read_message(&mut reader).await {
        received.lock().expect("not poisoned").log.push(message.clone());
        if let (Some(id), Some(_)) = (message.get("id"), message.get("method")) {
            let reply = behave(&message);
            send_reply(&writer, id, reply).await;
        }
    }
}

async fn send_reply(writer: &Writer, id: &Value, reply: Reply) {
    let body = match reply {
        Reply::Result(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Reply::Error(code) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": "failed"}}),
        Reply::Hold => return,
    };
    let body = serde_json::to_vec(&body).expect("serializes");
    let mut writer = writer.lock().await;
    let _ = writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await;
    let _ = writer.write_all(&body).await;
}

impl Fake {
    /// Everything received so far, in order.
    pub fn log(&self) -> Vec<Value> {
        self.received.lock().expect("not poisoned").log.clone()
    }

    /// The `method` messages received so far.
    pub fn messages(&self, method: &str) -> Vec<Value> {
        self.log().into_iter().filter(|m| m["method"] == method).collect()
    }

    /// Waits until `condition` holds for the log. Panics after five seconds.
    pub async fn until(&self, what: &str, condition: impl Fn(&[Value]) -> bool) {
        for _ in 0..1000 {
            if condition(&self.log()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the server never received: {what}\n{:#?}", self.log());
    }

    /// Answers request `id`, late or not.
    pub async fn reply(&self, id: i64, reply: Reply) {
        send_reply(&self.writer, &json!(id), reply).await;
    }
}
