//! Binds the loopback WebSocket and prints the per-launch token to stdout for
//! the shell to read (§4.1). It answers only `Ping`; the use cases in vet-app and
//! the adapters in vet-infra are not wired in yet.

use vet_protocol::{Request, Response};
use vet_transport::{Handler, Server};

struct Orchestrator;

impl Handler for Orchestrator {
    async fn handle(&self, request: Request) -> Result<Response, String> {
        match request {
            Request::Ping { payload } => Ok(Response::Pong { payload }),
            _ => Err("not implemented; the shell spike runs against vet-mock".into()),
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--exit-on-stdin-close") {
        vet_transport::exit_when_stdin_closes();
    }
    let server = Server::bind().await?;
    println!("{}", serde_json::to_string(&server.ready()?).expect("ready serializes"));
    server.serve(Orchestrator).await
}
