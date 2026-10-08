//! Binds the loopback WebSocket and prints the per-launch token to stdout for
//! the shell to read (§4.1), then serves protocol v1 from the real components.

use std::io;

use vet_backend::{Config, Orchestrator, open_store};
use vet_infra::store::SqliteStore;
use vet_transport::Server;
use vet_transport::shell::{announce, exit_when_stdin_closes};

#[tokio::main]
async fn main() -> io::Result<()> {
    if std::env::args().any(|a| a == "--exit-on-stdin-close") {
        exit_when_stdin_closes();
    }
    let dir = SqliteStore::default_dir().ok_or_else(|| io::Error::other("no data directory"))?;
    let _log = vet_log::init(&dir.join(vet_log::LOGS));
    let store = open_store(&dir)
        .await
        .inspect_err(|error| tracing::error!(%error, "cannot open the store"))
        .map_err(io::Error::other)?;
    let orchestrator = Orchestrator::new(store, Config::default());
    orchestrator.collect_trees().await;
    let server = Server::bind().await?;
    announce(&server)?;
    server.serve(orchestrator).await
}
