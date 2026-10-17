//! Merge bases of histories whose commits all carry the same timestamp, against `git merge-base`.

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

async fn assert_bases_match_git(dir: &Path, commits: &[String]) {
    let repo = GixRepo::for_test(dir);
    for a in commits {
        git(dir, &["update-ref", "refs/heads/probe", a]);
        for b in commits {
            let all: Vec<String> = git(dir, &["merge-base", "--all", a, b])
                .lines()
                .map(str::to_owned)
                .collect();
            let got = repo
                .merge_base("refs/heads/probe", &CommitId(b.clone()))
                .await
                .unwrap()
                .0;
            assert!(all.contains(&got), "base of {a} and {b}: got {got}, git says {all:?}");

            let descends = all == [a.clone()];
            assert_eq!(
                repo.descends_from(b, a).await.unwrap(),
                descends,
                "{b} descends from {a}"
            );
        }
    }
}

fn init() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    dir
}

/// A pseudo-random history of branches and merges (a fixed seed that gix gets wrong when it
/// trusts commit times).
#[tokio::test]
async fn a_dense_dag_within_one_second_has_a_best_base_for_every_pair() {
    let dir = init();
    let p = dir.path();
    let mut commits = vec![commit(p, "c0", &[])];
    let mut seed = 12345_u64;
    let mut next = |bound: usize| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize % bound
    };
    for i in 1..40 {
        let first = commits[next(i)].clone();
        let mut parents = vec![first];
        if next(3) == 0 {
            let second = commits[next(i)].clone();
            if !parents.contains(&second) {
                parents.push(second);
            }
        }
        let refs: Vec<&str> = parents.iter().map(String::as_str).collect();
        commits.push(commit(p, &format!("c{i}"), &refs));
    }
    assert_bases_match_git(p, &commits).await;
}
