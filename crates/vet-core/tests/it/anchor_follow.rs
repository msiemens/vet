//! A thread on lines of a hunk keeps following them when a later push changes the hunk.

use std::collections::BTreeMap;

use vet_core::content::{BlobId, Hash, LineRange, RepoPath, SnapshotId};
use vet_core::prepared::PreparedFile;
use vet_core::resolve::{Resolution, resolve_anchor};
use vet_core::review::{Anchor, AnchorState, Confidence, FileRemapInput, Side, StaleReason, Strategy};

const RECORDED: SnapshotId = SnapshotId(Hash([1; 32]));

const OLD: &str = "\
use std::io;

pub fn load(path: &str) -> io::Result<Vec<Job>> {
    let text = std::fs::read_to_string(path)?;
    let mut jobs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let job = Job::parse(line)?;
        jobs.push(job);
    }
    jobs.sort();
    Ok(jobs)
}

pub fn run(jobs: &[Job]) {
    for job in jobs {
        job.run();
    }
}
";

const COMMENTED: &str = "let job = Job::parse(line)?;";

/// Resolves a thread on the lines of `OLD` from the one containing `first` to the one containing
/// `last` into `new`.
fn follow(first: &str, last: &str, new: &str) -> Resolution {
    let lines: Vec<&str> = OLD.lines().collect();
    let at = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap() as u32;
    let range = LineRange {
        start: at(first),
        end: at(last) + 1,
    };
    let anchor = Anchor::from_text(Side::Target, RECORDED, RepoPath::from("a.rs"), OLD.as_bytes(), range).unwrap();
    let mut blobs = BTreeMap::new();
    let (o, n) = (BlobId::of(OLD.as_bytes()), BlobId::of(new.as_bytes()));
    blobs.insert(o, OLD.as_bytes().to_vec());
    blobs.insert(n, new.as_bytes().to_vec());
    let input = FileRemapInput {
        old_path: Some(RepoPath::from("a.rs")),
        new_path: Some(RepoPath::from("a.rs")),
        old_blob: Some(o),
        new_blob: Some(n),
    };
    let file = PreparedFile::new(&input, &blobs);
    resolve_anchor(&anchor, &file, std::slice::from_ref(&file))
}

fn replace(from: &str, to: &str) -> String {
    assert!(OLD.contains(from));
    OLD.replace(from, to)
}

fn lines_of(r: &Resolution) -> (u32, u32) {
    let range = r.location.as_ref().expect("attached").range;
    (range.start, range.end)
}

const MOVED_LOW: AnchorState = AnchorState::Inferred {
    strategy: Strategy::ContextWindow,
    confidence: Confidence::Low,
};

#[test]
fn a_comment_stays_with_a_hunk_that_a_push_rewrote_around_it() {
    let rewritten = replace(
        "        let line = line.trim();\n        if line.is_empty() {\n            continue;\n        }\n        let job = Job::parse(line)?;\n        jobs.push(job);\n",
        "        match Job::parse(line.trim()) {\n            Ok(job) => jobs.push(job),\n            Err(e) if line.trim().is_empty() => continue,\n            Err(e) => return Err(e),\n        }\n",
    );

    let single = follow(COMMENTED, COMMENTED, &rewritten);
    let several = follow("if line.is_empty", COMMENTED, &rewritten);

    assert_eq!((&single.state, lines_of(&single)), (&MOVED_LOW, (6, 12)));
    assert_eq!((&several.state, lines_of(&several)), (&MOVED_LOW, (6, 12)));
}

#[test]
fn a_comment_stays_with_a_line_whose_neighbours_were_edited_too() {
    let edited = replace(
        "let line = line.trim();\n        if line.is_empty() {\n            continue;\n        }\n        let job = Job::parse(line)?;\n        jobs.push(job);\n    }\n    jobs.sort();",
        "let line = line.trim_end();\n        if line.is_empty() {\n            continue;\n        }\n        let job = Job::parse(line).map_err(wrap)?;\n        jobs.push(job.with_source(path));\n    }\n    jobs.sort_unstable();",
    );

    let r = follow(COMMENTED, COMMENTED, &edited);

    assert!(matches!(r.state, AnchorState::Inferred { .. }), "{:?}", r.state);
    assert_eq!(lines_of(&r), (10, 11));
}

#[test]
fn a_comment_on_a_line_that_moved_into_a_helper_follows_it() {
    let moved = replace(
        "        let job = Job::parse(line)?;\n        jobs.push(job);\n",
        "        jobs.push(parse_one(line)?);\n",
    ) + "\nfn parse_one(line: &str) -> io::Result<Job> {\n        let job = Job::parse(line)?;\n        Ok(job)\n}\n";

    let r = follow(COMMENTED, COMMENTED, &moved);

    assert!(matches!(r.state, AnchorState::Inferred { .. }), "{:?}", r.state);
    assert_eq!(lines_of(&r), (23, 24));
}

#[test]
fn a_comment_on_a_deleted_line_is_detached() {
    let r = follow(
        COMMENTED,
        COMMENTED,
        &replace("        let job = Job::parse(line)?;\n", ""),
    );

    assert_eq!(
        r.state,
        AnchorState::Stale {
            reason: StaleReason::ContextChanged
        }
    );
}

#[test]
fn nothing_vouches_for_a_rewritten_file() {
    let r = follow(COMMENTED, COMMENTED, "fn main() {}\n");

    assert!(matches!(r.state, AnchorState::Stale { .. }));
}
