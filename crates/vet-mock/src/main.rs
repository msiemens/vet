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
use vet_protocol::{Completion, FileChange, FileReview, FileStatus, Hunk, HunkState, Manifest, Request, Response, ReviewUpdate, SessionId};
use vet_transport::{Events, Handler, Server};

struct Mock {
    corpus: Corpus,
    index: HashMap<String, usize>,
    /// Review state: one seen flag per hunk per file. It lives here, not in the shell (I5).
    seen: Mutex<Vec<Vec<bool>>>,
}

const SESSION: SessionId = 0;
const REVISION: u32 = 0;

fn review(seen: &[bool]) -> FileReview {
    FileReview { hunks: seen.iter().map(|&s| if s { HunkState::Seen } else { HunkState::Unseen }).collect(), acknowledged: vec![] }
}

/// Changed lines are counted on both sides; the corpus has no non-text items or old-base state.
fn completion(hunks: &[Hunk], seen: &[bool]) -> Completion {
    let mut c = Completion::default();
    for (h, &seen) in hunks.iter().zip(seen) {
        let lines = h.base_len + h.target_len;
        c.lines += lines;
        if seen {
            c.seen_lines += lines;
        }
    }
    c
}

fn total(parts: impl Iterator<Item = Completion>) -> Completion {
    parts.fold(Completion::default(), |a, b| Completion {
        lines: a.lines + b.lines,
        seen_lines: a.seen_lines + b.seen_lines,
        old_base_lines: a.old_base_lines + b.old_base_lines,
        items: a.items + b.items,
        acknowledged_items: a.acknowledged_items + b.acknowledged_items,
    })
}

impl Handler for Mock {
    async fn handle(&self, request: Request, _events: Events) -> Result<Response, String> {
        let position = |path: &str| self.index.get(path).copied().ok_or_else(|| format!("no such file: {path}"));
        let seen = || self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let current = |session: SessionId, revision: u32| match (session == SESSION, revision == REVISION) {
            (true, true) => Ok(()),
            (true, false) => Err("stale revision".to_string()),
            _ => Err("no such session".to_string()),
        };
        match request {
            Request::Ping { payload } => Ok(Response::Pong { payload }),
            // The mock has one session over its corpus; `repo` is ignored.
            Request::OpenSession { .. } => Ok(Response::Session { session: SESSION }),
            Request::CloseSession { .. } => Ok(Response::Done),
            Request::Manifest { .. } => {
                let seen = seen();
                let files: Vec<FileChange> = self
                    .corpus
                    .files
                    .iter()
                    .zip(seen.iter())
                    .map(|(f, seen)| FileChange {
                        path: f.path.clone(),
                        old_path: None,
                        status: FileStatus::Modified,
                        base_lines: corpus::line_count(&f.base) as u32,
                        target_lines: corpus::line_count(&f.target) as u32,
                        hunks: f.hunks.clone(),
                        non_text: vec![],
                        review: review(seen),
                        completion: completion(&f.hunks, seen),
                    })
                    .collect();
                Ok(Response::Manifest(Manifest {
                    session: SESSION,
                    revision: REVISION,
                    name: self.corpus.name.clone(),
                    completion: total(files.iter().map(|f| f.completion)),
                    files,
                    excluded_lines: 0,
                    excluded_items: 0,
                }))
            }
            Request::FileContents { path, .. } => {
                let f = &self.corpus.files[position(&path)?];
                Ok(Response::FileContents { revision: REVISION, base: Some(f.base.clone()), target: Some(f.target.clone()), marks: f.marks.clone() })
            }
            Request::SetSeen { session, revision, path, hunks, seen: value } => {
                current(session, revision)?;
                let i = position(&path)?;
                let mut all = seen();
                for hunk in hunks {
                    *all[i].get_mut(hunk as usize).ok_or("no such hunk")? = value;
                }
                let file = completion(&self.corpus.files[i].hunks, &all[i]);
                let changeset = total(self.corpus.files.iter().zip(all.iter()).map(|(f, s)| completion(&f.hunks, s)));
                Ok(Response::Review(ReviewUpdate { path, review: review(&all[i]), completion: file, changeset }))
            }
            Request::Acknowledge { .. } => Err("the corpus has no non-text items".into()),
            Request::Hover { path, line, col, .. } => {
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
    server.announce()?;
    server.serve(Mock { corpus, index, seen }).await
}
