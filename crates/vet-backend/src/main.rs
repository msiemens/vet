//! Binds the loopback WebSocket and prints the per-launch token to stdout for
//! the shell to read (§4.1), then serves protocol v1 from the real components.

use std::io;

use vet_backend::{Config, Orchestrator, open_default_store};
use vet_transport::Server;
use vet_transport::shell::{announce, exit_when_stdin_closes};

#[tokio::main]
async fn main() -> io::Result<()> {
    if std::env::args().any(|a| a == "--exit-on-stdin-close") {
        exit_when_stdin_closes();
    }
    vet_infra::materialize::sweep_materialized();
    let store = open_default_store().await.map_err(io::Error::other)?;
    let server = Server::bind().await?;
    announce(&server)?;
    server.serve(Orchestrator::new(store, Config::default())).await
}
