//! Base resolution and candidate listing against a real temporary repository.
//! `git` is used only to build the fixture.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use vet_app::pin::CommitId;
use vet_app::ports::History;
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
async fn first_parent_distance_and_commits_behind_follow_the_history() {
    let fx = fixture();
    let repo = GixRepo::new(fx.dir.path());
    let head = CommitId(git(fx.dir.path(), &["rev-parse", "HEAD~1"]));

    assert_eq!(repo.first_parent_distance(&fx.f2, &fx.f2, 10).await.unwrap(), Some(0));
    assert_eq!(repo.first_parent_distance(&fx.f2, &head, 10).await.unwrap(), Some(1));
    assert_eq!(repo.first_parent_distance(&fx.f2, &fx.m1, 10).await.unwrap(), Some(2));
    assert_eq!(
        repo.first_parent_distance(&fx.f2, &fx.m1, 1).await.unwrap(),
        None,
        "too far"
    );
    assert_eq!(
        repo.first_parent_distance(&fx.f2, &fx.m2, 10).await.unwrap(),
        None,
        "other branch"
    );

    assert_eq!(repo.commits_behind(&fx.f2, &fx.m1, 100).await.unwrap(), 2);
    assert_eq!(repo.commits_behind(&fx.f2, &fx.f2, 100).await.unwrap(), 0);
    assert_eq!(
        repo.commits_behind(&fx.m2, &fx.f2, 100).await.unwrap(),
        1,
        "only m2 is not on feature"
    );
    assert_eq!(
        repo.commits_behind(&fx.f2, &fx.m1, 1).await.unwrap(),
        1,
        "counted up to the cap"
    );
}

#[tokio::test]
async fn commits_and_branches_carry_what_a_chip_shows() {
    let fx = fixture();
    let repo = GixRepo::new(fx.dir.path());

    let info = repo.commit_info(&fx.m1).await.unwrap();
    assert_eq!((info.subject.as_str(), info.author.as_str()), ("m1", "Tess"));
    assert!(info.time > 1_600_000_000);

    let names: Vec<_> = repo
        .branches()
        .await
        .unwrap()
        .into_iter()
        .map(|b| (b.name, b.current))
        .collect();
    assert_eq!(names, [("feature".to_string(), true), ("main".to_string(), false)]);
}

#[tokio::test]
async fn a_base_is_named_like_a_person_would() {
    let fx = fixture();
    let repo = GixRepo::new(fx.dir.path());
    let branches = repo.branches().await.unwrap();
    let name = async |commit: &CommitId| vet_app::base::name_of(&repo, &branches, None, commit).await.unwrap();

    assert_eq!(name(&fx.f2).await, "feature");
    assert_eq!(name(&fx.m2).await, "main");
    // On both branches' history: the current branch's name wins.
    assert_eq!(name(&fx.m1).await, "feature~2");
    let hash = CommitId("0123456789abcdef0123456789abcdef01234567".into());
    assert_eq!(name(&hash).await, "0123456");
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
