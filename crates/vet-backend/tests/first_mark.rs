//! How long the first marks take on a large changeset, against SPEC §13's 100 ms. A
//! measurement, not a check: it is ignored, prints its timings and asserts nothing about
//! them. Run it alone, in the build to measure:
//!
//! `cargo test --release -p vet-backend --test first_mark -- --ignored --nocapture`
//!
//! The changeset is `VET_MEASURE_FILES` (300) of this repository's own source files, edited
//! in the working tree. They are named `*.txt`, so that no language server starts.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Client, config, git, init, write};
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{ProjectId, Request, Response, View};

/// This repository's source files over 1 KB, in path order, at most `limit`.
fn sources(limit: usize) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = Vec::new();
    for dir in ["crates", "ui/src"] {
        collect(&root.join(dir), &mut found);
    }
    found.sort();
    found
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            (text.len() > 1024).then(|| {
                let name = path.strip_prefix(&root).unwrap().to_string_lossy().replace('/', "_");
                (format!("src/{name}.txt"), text)
            })
        })
        .take(limit)
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "target" | "generated" | "node_modules") {
                collect(&path, out);
            }
        } else if [".rs", ".ts", ".svelte"].iter().any(|e| name.ends_with(e)) {
            out.push(path);
        }
    }
}

/// `text` edited as a review would find it: every 40th line changed, three lines added a third
/// of the way in, two removed two thirds of the way in.
fn edited(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (insert, remove) = (lines.len() / 3, 2 * lines.len() / 3);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == insert {
            out.push_str("// added for the review\n// and a second line\n// and a third\n");
        }
        if i == remove || i == remove + 1 {
            continue;
        }
        out.push_str(line);
        if i % 40 == 20 {
            out.push_str(" // edited");
        }
        out.push('\n');
    }
    out
}

fn repo(files: &[(String, String)]) -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    init(repo.path());
    for (name, text) in files {
        write(repo.path(), name, text);
    }
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    for (name, text) in files {
        write(repo.path(), name, &edited(text));
    }
    repo
}

async fn timed<T>(work: impl Future<Output = T>) -> (T, Duration) {
    let started = Instant::now();
    let out = work.await;
    (out, started.elapsed())
}

async fn toggle(client: &Client, project: ProjectId, revision: u32, path: &str, hunk: u32) -> Duration {
    let (answer, took) = timed(client.call(Request::ToggleHunk {
        project,
        view: View::Changeset,
        revision,
        path: path.into(),
        hunk,
    }))
    .await;
    assert!(matches!(answer, Ok(Response::Review(_))), "{answer:?}");
    took
}

/// How long until the content store in `data` holds every one of `texts`.
async fn copied(data: &Path, texts: &[String]) -> Duration {
    let started = Instant::now();
    for text in texts {
        let hex = vet_core::content::BlobId::of(text.as_bytes()).0.to_hex();
        let path = data.join("content").join(&hex[..2]).join(&hex[2..]);
        while !path.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(120),
                "the copy ahead never finished"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    started.elapsed()
}

/// Opens the repository on the store in `data`, then marks `marks` hunks, one in each of the
/// files from `first` on; with `every`, then marks every file seen. With `ahead`, the marks
/// wait until the store holds these texts, which the session copies into it in the background.
/// Prints how long each step took.
async fn session(
    label: &str,
    data: &Path,
    repo: &Path,
    names: &[String],
    (first, marks): (usize, usize),
    every: bool,
    ahead: Option<&[String]>,
) {
    let handler = Arc::new(Orchestrator::new(open_store(data).await.unwrap(), config()));
    let client = Client::new(&handler);
    let (project, open) = timed(client.open(repo)).await;
    let (revision, manifest) = timed(client.revision(project)).await;
    let (_, second_manifest) = timed(client.revision(project)).await;
    println!("{label}: open {open:?}, first manifest {manifest:?}, second manifest {second_manifest:?}");
    if let Some(texts) = ahead {
        println!(
            "{label}: the copy ahead finished {:?} after the second manifest",
            copied(data, texts).await
        );
    }
    let mut took = Vec::new();
    for path in names.iter().skip(first).take(marks) {
        took.push(toggle(&client, project, revision, path, 0).await);
    }
    let rest = &took[1..];
    println!(
        "{label}: mark 1 {:?}, mark 2 {:?}, marks 2..{marks} median {:?} max {:?}, last {:?}",
        took[0],
        took[1],
        {
            let mut sorted = rest.to_vec();
            sorted.sort();
            sorted[sorted.len() / 2]
        },
        rest.iter().max().unwrap(),
        took.last().unwrap(),
    );
    if !every {
        return;
    }
    // A mark remaps the spans of its file only, so it should not grow with what is seen.
    let (mut slowest, mut slowest_at) = (Duration::ZERO, 0);
    for (n, path) in names.iter().enumerate() {
        let (answer, took) = timed(client.call(Request::ToggleFileSeen {
            project,
            view: View::Changeset,
            revision,
            path: path.clone(),
        }))
        .await;
        assert!(matches!(answer, Ok(Response::Review(_))), "{answer:?}");
        if took > slowest {
            (slowest, slowest_at) = (took, n + 1);
        }
        if (n + 1) % 50 == 0 {
            println!("{label}: file {} seen in {took:?}", n + 1);
        }
    }
    let Ok(Response::Manifest(manifest)) = client
        .call(Request::Manifest {
            project,
            view: View::Changeset,
        })
        .await
    else {
        panic!("no manifest");
    };
    println!(
        "{label}: every file seen ({} hunks), the slowest (file {slowest_at}) took {slowest:?}",
        manifest.completion.seen_hunks
    );
    let undo = toggle(&client, project, revision, &names[0], 0).await;
    let redo = toggle(&client, project, revision, &names[0], 0).await;
    println!("{label}: a mark with everything seen: unmark {undo:?}, mark {redo:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement: run it alone with --ignored --nocapture"]
async fn the_first_marks_on_a_large_changeset() {
    let count = std::env::var("VET_MEASURE_FILES").map_or(300, |n| n.parse().unwrap());
    let files = sources(count);
    let bytes: usize = files.iter().map(|(_, t)| t.len()).sum();
    println!("{} files, {} KB", files.len(), bytes / 1024);
    let repo = repo(&files);
    let names: Vec<_> = files.iter().map(|(n, _)| n.clone()).collect();
    let data = tempfile::tempdir().unwrap();

    // The first mark starts the review, while the target's content is still being copied.
    session("cold", data.path(), repo.path(), &names, (0, 20), false, None).await;
    // The review is open now, with 20 marks to remap; the store keeps the target's content.
    session("reopened", data.path(), repo.path(), &names, (20, 20), true, None).await;
    // On a store of its own, the first mark comes once the target's content was copied.
    let fresh = tempfile::tempdir().unwrap();
    let texts: Vec<String> = files.iter().map(|(_, text)| edited(text)).collect();
    session(
        "copied ahead",
        fresh.path(),
        repo.path(),
        &names,
        (0, 20),
        false,
        Some(&texts),
    )
    .await;
}
