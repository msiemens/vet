//! The in-memory answers in front of the file: the same questions asked with caching
//! off, on, and in a launch that starts over the file the previous one left.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use Question::*;
use vet_core::content::{BlobId, Hash, SnapshotId};
use vet_infra::semantic_cache::SqliteSemanticCache;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{At, BackendId, Pos, ServerIdentity, Span, Sym, SymbolInfo};
use vet_semantic::reuse::{Answers, Basis, Computed, KEPT, Key, ProjectQuery, Restore, SemanticCache};

const BUDGET: u64 = 256 << 20;

fn server(version: u8) -> ServerIdentity {
    ServerIdentity {
        backend: BackendId("rust".into()),
        fingerprint: Hash([version; 32]),
        label: format!("fake {version}"),
    }
}

fn symbols(text: &str, version: u8) -> Vec<SymbolInfo> {
    vec![SymbolInfo {
        sym: Sym {
            file: "a.rs".into(),
            name: vec![format!("{text} by {version}")],
        },
        span: Span {
            start: Pos { line: 0, col: 0 },
            end: Pos { line: 1, col: 0 },
        },
        at: Pos { line: 0, col: 3 },
        detail: None,
        container: false,
    }]
}

/// What the session asks: the symbols of texts, and a hover on two trees.
enum Question {
    Symbols { text: &'static str, version: u8 },
    Hover { snapshot: u8, line: u32, version: u8 },
}

fn key(question: &Question) -> Key {
    match question {
        Question::Symbols { text, version } => Key::Text {
            server: server(*version),
            extension: b"rs".to_vec(),
            blob: BlobId::of(text.as_bytes()),
        },
        Question::Hover {
            snapshot,
            line,
            version,
        } => Key::Project {
            server: server(*version),
            basis: Basis {
                snapshot: SnapshotId(Hash([*snapshot; 32])),
                policy: AnalysisPolicy { allow_executing: true },
                restore: Restore::Recorded(Hash([5; 32])),
            },
            query: ProjectQuery::Hover(At {
                file: "a.rs".into(),
                pos: Pos { line: *line, col: 0 },
            }),
        },
    }
}

/// The session: the questions of one launch, in order, and what each answered.
fn session() -> Vec<Question> {
    vec![
        Symbols {
            text: "one",
            version: 1,
        },
        Hover {
            snapshot: 1,
            line: 3,
            version: 1,
        },
        Symbols {
            text: "two",
            version: 1,
        },
        // Edited back.
        Symbols {
            text: "one",
            version: 1,
        },
        Hover {
            snapshot: 1,
            line: 3,
            version: 1,
        },
        // Rebase.
        Hover {
            snapshot: 2,
            line: 3,
            version: 1,
        },
        // The server was updated.
        Symbols {
            text: "one",
            version: 2,
        },
        Hover {
            snapshot: 1,
            line: 3,
            version: 2,
        },
    ]
}

/// Runs the session against `answers`; what was answered and how often a server was asked.
async fn run(answers: &Answers) -> (Vec<String>, usize) {
    let asked = Arc::new(AtomicUsize::new(0));
    let mut results = Vec::new();
    for question in session() {
        let key = key(&question);
        let (said, asked) = (question.answer(), asked.clone());
        match &question {
            Question::Symbols { .. } => {
                let (symbols, _) = answers
                    .get(Some(&key), move || async move {
                        asked.fetch_add(1, Ordering::SeqCst);
                        Ok(Computed {
                            value: said.symbols(),
                            keep: true,
                        })
                    })
                    .await
                    .unwrap();
                results.push(format!("{symbols:?}"));
            }
            Question::Hover { .. } => {
                let (hover, _) = answers
                    .get(Some(&key), move || async move {
                        asked.fetch_add(1, Ordering::SeqCst);
                        Ok(Computed {
                            value: said.hover(),
                            keep: true,
                        })
                    })
                    .await
                    .unwrap();
                results.push(format!("{hover:?}"));
            }
        }
    }
    (results, asked.load(Ordering::SeqCst))
}

/// What a server of that version says to a question.
enum Said {
    Symbols(Vec<SymbolInfo>),
    Hover(Option<String>),
}

impl Question {
    fn answer(&self) -> Said {
        match self {
            Question::Symbols { text, version } => Said::Symbols(symbols(text, *version)),
            Question::Hover {
                snapshot,
                line,
                version,
            } => Said::Hover(Some(format!("tree {snapshot} line {line} by {version}"))),
        }
    }
}

impl Said {
    fn symbols(self) -> Vec<SymbolInfo> {
        match self {
            Said::Symbols(symbols) => symbols,
            Said::Hover(_) => unreachable!("a question about symbols"),
        }
    }

    fn hover(self) -> Option<String> {
        match self {
            Said::Hover(hover) => hover,
            Said::Symbols(_) => unreachable!("a question about a hover"),
        }
    }
}

#[tokio::test]
async fn a_session_gets_the_same_answers_with_every_cache_off_on_and_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();

    let (off, asked_off) = run(&Answers::new(0)).await;
    assert_eq!(asked_off, session().len());

    let cache = Arc::new(SqliteSemanticCache::open(dir.path(), BUDGET).await.unwrap());
    let (on, asked_on) = run(&Answers::new(KEPT).persisting(Some(cache.clone() as Arc<dyn SemanticCache>))).await;
    assert_eq!(on, off);
    // The edit that was undone, and the hover asked twice of one tree.
    assert_eq!(asked_on, session().len() - 2);
    cache.flush().await;
    drop(cache);

    // Another launch: nothing in memory, the file the first one left.
    let cache = Arc::new(SqliteSemanticCache::open(dir.path(), BUDGET).await.unwrap());
    let (restarted, asked_restarted) = run(&Answers::new(KEPT).persisting(Some(cache as Arc<dyn SemanticCache>))).await;
    assert_eq!(restarted, off);
    assert_eq!(asked_restarted, 0);
}
