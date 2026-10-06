//! Base resolution and candidate listing against a real temporary repository.
//! `git` is used only to build the fixture.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use vet_app::pin::CommitId;
use vet_app::ports::GitRepo;
use vet_infra::git::GixRepo;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
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

fn commit(dir: &Path, file: &str, message: &str) -> CommitId {
    std::fs::write(dir.join(file), message).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    CommitId(git(dir, &["rev-parse", "HEAD"]))
}

/// `main` has `m1` and `m2`; `feature` branched at `m1` and has `f1` and `f2`
/// and is checked out.
struct Fixture {
    dir: TempDir,
    m1: CommitId,
    m2: CommitId,
    f2: CommitId,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    let m1 = commit(p, "a.txt", "m1");
    git(p, &["checkout", "-q", "-b", "feature"]);
    commit(p, "b.txt", "f1");
    let f2 = commit(p, "b.txt", "f2");
    git(p, &["checkout", "-q", "main"]);
    let m2 = commit(p, "c.txt", "m2");
    git(p, &["checkout", "-q", "feature"]);
    Fixture { dir, m1, m2, f2 }
}

#[tokio::test]
async fn the_merge_base_is_where_the_branches_diverged() {
    let fx = fixture();
    let repo = GixRepo::new(fx.dir.path());
    assert_eq!(repo.merge_base("main").await.unwrap(), fx.m1);
    assert_eq!(repo.merge_base("feature").await.unwrap(), fx.f2);
    assert_eq!(
        repo.merge_base("HEAD~1").await.unwrap().0,
        git(fx.dir.path(), &["rev-parse", "HEAD~1"])
    );
}

#[tokio::test]
async fn unresolvable_bases_have_user_facing_errors() {
    let fx = fixture();
    let p = fx.dir.path();
    let repo = GixRepo::new(p);
    assert_eq!(
        repo.merge_base("nope").await.unwrap_err().0,
        "unknown branch or revision nope"
    );
    assert_eq!(
        repo.resolve_commit("0123abc").await.unwrap_err().0,
        "unknown commit 0123abc"
    );
    assert_eq!(
        repo.resolve_commit("HEAD^{tree}").await.unwrap_err().0,
        "unknown commit HEAD^{tree}"
    );

    git(p, &["checkout", "-q", "--orphan", "unrelated"]);
    commit(p, "z.txt", "z");
    assert_eq!(
        repo.merge_base("main").await.unwrap_err().0,
        "no common ancestor with main"
    );
}

#[tokio::test]
async fn commits_resolve_by_full_id_abbreviation_and_revision() {
    let fx = fixture();
    let repo = GixRepo::new(fx.dir.path());
    assert_eq!(repo.resolve_commit(&fx.m2.0).await.unwrap(), fx.m2);
    assert_eq!(repo.resolve_commit(&fx.m2.0[..8]).await.unwrap(), fx.m2);
    assert_eq!(repo.resolve_commit("main").await.unwrap(), fx.m2);
    assert_eq!(repo.resolve_commit("HEAD").await.unwrap(), fx.f2);
}

#[tokio::test]
async fn candidates_list_branches_and_recent_commits_newest_first() {
    let fx = fixture();
    let repo = GixRepo::new(fx.dir.path());

    let c = repo.base_candidates(200).await.unwrap();

    let branches: Vec<_> = c
        .branches
        .iter()
        .map(|b| (b.name.as_str(), &b.commit, b.current))
        .collect();
    assert_eq!(
        branches,
        [("feature", &fx.f2, true), ("main", &fx.m2, false)],
        "sorted by name, HEAD's branch marked"
    );
    assert!(c.branches.iter().all(|b| !b.default), "there is no remote");
    let subjects: Vec<_> = c.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["f2", "f1", "m1"], "only what HEAD reaches");
    assert_eq!(c.commits[0].id, fx.f2);
    assert_eq!(c.commits[0].author, "Tess");
    assert!(c.commits[0].time > 1_600_000_000);

    assert_eq!(repo.base_candidates(2).await.unwrap().commits.len(), 2);
}

#[tokio::test]
async fn the_default_branch_is_the_one_origin_head_names() {
    let fx = fixture();
    let p = fx.dir.path();
    git(p, &["update-ref", "refs/remotes/origin/main", "main"]);
    git(
        p,
        &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"],
    );
    let repo = GixRepo::new(p);

    let c = repo.base_candidates(10).await.unwrap();

    let default: Vec<_> = c
        .branches
        .iter()
        .filter(|b| b.default)
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(default, ["main"]);
}
