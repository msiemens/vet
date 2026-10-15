//! The automatic merge of a merge commit's parents against a real temporary repository: what
//! `git show --remerge-diff` compares the commit with. `git` is used only to build the fixture.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use vet_app::changeset::CommitId;
use vet_app::ports::History;
use vet_core::content::RepoPath;
use vet_infra::git::GixRepo;

fn git_in(dir: &Path, args: &[&str], check: bool) -> String {
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
    assert!(
        !check || out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn git(dir: &Path, args: &[&str]) -> String {
    git_in(dir, args, true)
}

fn commit_file(dir: &Path, file: &str, text: &str) -> CommitId {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", file]);
    head(dir)
}

fn head(dir: &Path) -> CommitId {
    CommitId(git(dir, &["rev-parse", "HEAD"]))
}

fn lines(n: usize) -> String {
    (0..n).map(|i| format!("l{i}\n")).collect()
}

/// `main` and `feature` both start from `a.txt` with ten lines. `main` changes `l2`, `feature`
/// changes `l7`, so merging them is clean; `conflicting` has `feature` change `l2` too.
struct Fixture {
    dir: TempDir,
}

fn fixture(conflicting: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    commit_file(p, "a.txt", &lines(10));
    git(p, &["checkout", "-q", "-b", "feature"]);
    let feature = if conflicting {
        lines(10).replace("l2\n", "feature\n")
    } else {
        lines(10).replace("l7\n", "feature\n")
    };
    commit_file(p, "a.txt", &feature);
    git(p, &["checkout", "-q", "main"]);
    commit_file(p, "a.txt", &lines(10).replace("l2\n", "main\n"));
    git(p, &["checkout", "-q", "feature"]);
    Fixture { dir }
}

fn merge(p: &Path) -> CommitId {
    git_in(p, &["merge", "-q", "--no-ff", "-m", "merge", "main"], false);
    git_in(p, &["rev-parse", "HEAD"], true);
    head(p)
}

fn text_of(snapshot: &vet_core::content::Snapshot, path: &str, contents: &[Vec<u8>]) -> Option<String> {
    let blob = snapshot.entries.get(&RepoPath::from(path))?.blob;
    contents
        .iter()
        .find(|c| vet_core::content::BlobId::of(c) == blob)
        .map(|c| String::from_utf8(c.clone()).unwrap())
}

#[tokio::test]
async fn a_clean_merge_remerges_to_its_own_tree() {
    let fx = fixture(false);
    let p = fx.dir.path();
    let merged = merge(p);
    let repo = GixRepo::for_test(p);

    let remerged = repo.remerge(&merged).await.unwrap().expect("a two-parent merge");

    assert_eq!(remerged.snapshot, repo.commit_snapshot(&merged).await.unwrap());
}

#[tokio::test]
async fn a_conflict_stays_in_the_remerge_as_markers_and_the_resolution_is_the_difference() {
    let fx = fixture(true);
    let p = fx.dir.path();
    git_in(p, &["merge", "-q", "--no-ff", "-m", "merge", "main"], false);
    std::fs::write(p.join("a.txt"), lines(10).replace("l2\n", "resolved\n")).unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "--no-edit"]);
    let merged = head(p);
    let repo = GixRepo::for_test(p);

    let remerged = repo.remerge(&merged).await.unwrap().unwrap();

    let text = text_of(&remerged.snapshot, "a.txt", &remerged.contents).expect("the merged file is carried");
    assert!(
        text.contains("<<<<<<<") && text.contains("main") && text.contains("feature"),
        "{text}"
    );
    assert_ne!(remerged.snapshot, repo.commit_snapshot(&merged).await.unwrap());
}

#[tokio::test]
async fn a_change_made_in_the_merge_commit_is_not_in_the_remerge() {
    let fx = fixture(false);
    let p = fx.dir.path();
    git_in(p, &["merge", "-q", "--no-ff", "--no-commit", "main"], false);
    std::fs::write(p.join("extra.txt"), "added by the merge\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "merge with an extra file"]);
    let merged = head(p);
    let repo = GixRepo::for_test(p);

    let remerged = repo.remerge(&merged).await.unwrap().unwrap();
    let actual = repo.commit_snapshot(&merged).await.unwrap();

    assert!(!remerged.snapshot.entries.contains_key(&RepoPath::from("extra.txt")));
    assert!(actual.entries.contains_key(&RepoPath::from("extra.txt")));
    assert_eq!(remerged.snapshot.entries.len() + 1, actual.entries.len());
}

#[tokio::test]
async fn remerging_leaves_the_repository_as_it_was_and_only_merges_have_a_remerge() {
    let fx = fixture(true);
    let p = fx.dir.path();
    let before = git(p, &["count-objects", "-v"]);
    let tip = head(p);
    let repo = GixRepo::for_test(p);
    git_in(p, &["merge", "-q", "--no-ff", "-m", "merge", "main"], false);
    git(p, &["checkout", "--theirs", "a.txt"]);
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "--no-edit"]);
    let merged = head(p);
    let objects = git(p, &["count-objects", "-v"]);

    repo.remerge(&merged).await.unwrap().unwrap();

    assert_eq!(
        git(p, &["count-objects", "-v"]),
        objects,
        "the merged files are kept in memory"
    );
    assert_ne!(before, objects, "the merge itself wrote objects");
    assert!(repo.remerge(&tip).await.unwrap().is_none());
    let detail = repo.commit_detail(&merged).await.unwrap();
    assert_eq!(detail.parents.len(), 2);
    assert_eq!(detail.parents[0], tip);
}

#[tokio::test]
async fn a_commit_has_its_message_split_into_subject_and_body() {
    let fx = fixture(false);
    let p = fx.dir.path();
    std::fs::write(p.join("b.txt"), "b\n").unwrap();
    git(p, &["add", "-A"]);
    git(
        p,
        &[
            "commit",
            "-q",
            "-m",
            "Attach the key",
            "-m",
            "It is carried from the event.\nRetries reuse it.",
        ],
    );
    let repo = GixRepo::for_test(p);

    let detail = repo.commit_detail(&head(p)).await.unwrap();

    assert_eq!(detail.info.subject, "Attach the key");
    assert_eq!(detail.body.trim(), "It is carried from the event.\nRetries reuse it.");
    assert_eq!(detail.parents.len(), 1);
}
