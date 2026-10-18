//! Merge bases of histories whose commits all carry the same timestamp.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use vet_app::changeset::CommitId;
use vet_app::ports::{Fetcher, History};
use vet_infra::git::GixRepo;

const DATE: &str = "1700000000 +0000";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_DATE", DATE)
        .env("GIT_COMMITTER_DATE", DATE)
        .args([
            "-c",
            "user.name=Tess",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// A commit with the given parents on a fresh tree, so merges need no conflict handling.
fn commit(dir: &Path, name: &str, parents: &[&str]) -> String {
    let tree = git(dir, &["write-tree"]);
    let mut args = vec!["commit-tree", tree.as_str(), "-m", name];
    for parent in parents {
        args.extend(["-p", parent]);
    }
    git(dir, &args)
}

/// The smallest history found in which a search that trusts commit times returns an ancestor of
/// the real base: `e` descends from `d`, yet reaches `c` (an ancestor of `d`) a second way.
///
/// ```text
/// a <- b <- c <- d <- e      c = [a, b]    e = [b, d]
/// ```
struct Ties {
    _dir: tempfile::TempDir,
    repo: GixRepo,
    a: String,
    b: String,
    c: String,
    d: String,
    e: String,
}

fn ties() -> Ties {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    let a = commit(p, "a", &[]);
    let b = commit(p, "b", &[&a]);
    let c = commit(p, "c", &[&a, &b]);
    let d = commit(p, "d", &[&c]);
    let e = commit(p, "e", &[&b, &d]);
    Ties {
        repo: GixRepo::for_test(p),
        _dir: dir,
        a,
        b,
        c,
        d,
        e,
    }
}

async fn base(t: &Ties, reference: &str, tip: &str) -> String {
    t.repo.merge_base(reference, &CommitId(tip.to_owned())).await.unwrap().0
}

#[tokio::test]
async fn the_merge_base_is_the_newest_common_commit_when_commit_times_tie() {
    let t = ties();

    assert_eq!(base(&t, &t.d, &t.e).await, t.d);
    assert_eq!(base(&t, &t.e, &t.d).await, t.d);
    assert_eq!(base(&t, &t.e, &t.c).await, t.c);
    assert_eq!(base(&t, &t.b, &t.e).await, t.b);
    assert_eq!(base(&t, &t.a, &t.e).await, t.a);
}

#[tokio::test]
async fn descent_is_decided_by_ancestry_when_commit_times_tie() {
    let t = ties();
    let r = &t.repo;

    assert!(r.descends_from(&t.e, &t.d).await.unwrap());
    assert!(r.descends_from(&t.e, &t.a).await.unwrap());
    assert!(r.descends_from(&t.d, &t.b).await.unwrap());
    assert!(!r.descends_from(&t.d, &t.e).await.unwrap());
    assert!(!r.descends_from(&t.b, &t.c).await.unwrap());
}
