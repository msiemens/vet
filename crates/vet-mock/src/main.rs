//! Mock orchestrator for the shell spike (SPEC §12.0): the real transport and
//! handshake, answering from a real git changeset (see `corpus`). Prints the same
//! `Ready` line as the backend.
//!
//! Flags (each also settable through `VET_CORPUS_{URL,REPO,BASE,HEAD}`):
//! `--url`, `--repo`, `--base`, `--head`; `--stats` prints corpus statistics and
//! exits non-zero if §12.0's thresholds are unmet; `--prepare` only fetches the
//! corpus's refs; `--exit-on-stdin-close` makes it exit with its parent shell.

mod corpus;

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use corpus::{Config, Corpus};
use vet_protocol::{FileEntry, Request, Response};
use vet_transport::{Handler, Server};

struct Mock {
    corpus: Corpus,
    index: HashMap<String, usize>,
    /// Review state: one seen flag per hunk per file. It lives here, not in the shell (I5).
    seen: Mutex<Vec<Vec<bool>>>,
}

impl Handler for Mock {
    async fn handle(&self, request: Request) -> Result<Response, String> {
        let position = |path: &str| self.index.get(path).copied().ok_or_else(|| format!("no such file: {path}"));
        let seen = || self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        match request {
            Request::Ping { payload } => Ok(Response::Pong { payload }),
            Request::OpenChangeset => {
                let seen = seen();
                Ok(Response::Changeset {
                    name: self.corpus.name.clone(),
                    files: self
                        .corpus
                        .files
                        .iter()
                        .zip(seen.iter())
                        .map(|(f, seen)| FileEntry {
                            path: f.path.clone(),
                            base_lines: corpus::line_count(&f.base) as u32,
                            target_lines: corpus::line_count(&f.target) as u32,
                            hunks: f.hunks.iter().map(|h| (h.base_len, h.target_len)).collect(),
                            seen: seen.clone(),
                        })
                        .collect(),
                })
            }
            Request::GetFile { path } => {
                let i = position(&path)?;
                let f = &self.corpus.files[i];
                Ok(Response::File { path, base: f.base.clone(), target: f.target.clone(), hunks: f.hunks.clone(), marks: f.marks.clone() })
            }
            Request::SetSeen { path, hunks, seen: value } => {
                let i = position(&path)?;
                let mut all = seen();
                for hunk in hunks {
                    *all[i].get_mut(hunk as usize).ok_or("no such hunk")? = value;
                }
                Ok(Response::Seen { path, seen: all[i].clone() })
            }
            Request::Hover { path, line, col } => {
                position(&path)?;
                Ok(Response::Hover { text: format!("{path}:{}:{}\n\nmock hover (derived: reachable from 2 entry points)", line + 1, col + 1) })
            }
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    if flag("--exit-on-stdin-close") {
        vet_transport::exit_when_stdin_closes();
    }
    let config = Config::from_args(&args);
    let fail = |error: String| -> ! {
        eprintln!("cannot load the corpus: {error}");
        std::process::exit(2);
    };
    if flag("--prepare") {
        corpus::ensure_repo(&config).unwrap_or_else(|e| fail(e));
        return Ok(());
    }
    let corpus = corpus::load(&config).unwrap_or_else(|e| fail(e));

    let stats = corpus.stats();
    eprintln!(
        "corpus {}: {} files, {} hunks, biggest file {} ({} lines)",
        corpus.name, stats.files, stats.hunks, stats.biggest.0, stats.biggest.1
    );
    eprintln!("hunks are counted as zero-context regions (like git diff -U0)");
    if flag("--stats") {
        let problems = stats.problems();
        for problem in &problems {
            eprintln!("below the spike's corpus thresholds: {problem}");
        }
        std::process::exit(i32::from(!problems.is_empty()));
    }

    let index = corpus.files.iter().enumerate().map(|(i, f)| (f.path.clone(), i)).collect();
    let seen = Mutex::new(corpus.files.iter().map(|f| vec![false; f.hunks.len()]).collect());
    let server = Server::bind().await?;
    println!("{}", serde_json::to_string(&server.ready()?).expect("ready serializes"));
    server.serve(Mock { corpus, index, seen }).await
}
