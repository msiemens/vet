//! The backend process's side of the contract with the shell that launches it
//! (§4.1): the `Ready` line on stdout, and exiting when the shell goes away.

use std::io;

use crate::Server;

/// Prints the `Ready` line to stdout for the shell, and to stderr the address a
/// plain browser needs during development.
pub fn announce(server: &Server) -> io::Result<()> {
    let ready = server.ready()?;
    println!("{}", serde_json::to_string(&ready).expect("ready serializes"));
    eprintln!(
        "dev UI: http://localhost:1420/#port={}&token={}",
        ready.port, ready.token
    );
    Ok(())
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
