//! Binds the loopback WebSocket and prints the per-launch token to stdout for
//! the shell to read (§4.1), then serves protocol v1 from the real components.

use vet_backend::{Config, Orchestrator, open_default_store};
use vet_transport::Server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--exit-on-stdin-close") {
        vet_transport::exit_when_stdin_closes();
    }
    vet_infra::materialize::sweep_materialized();
    let store = open_default_store().await.map_err(std::io::Error::other)?;
    let server = Server::bind().await?;
    server.announce()?;
    server.serve(Orchestrator::new(store, Config::default())).await
}
