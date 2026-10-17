//! The list of changesets: which branches it walks, and what it keeps from one listing to the next.

use super::*;

const DAY: i64 = 24 * 60 * 60;

fn walks(fx: &Fixture) -> usize {
    fx.git.behind_walks.load(Ordering::Relaxed)
}

fn info_reads(fx: &Fixture) -> usize {
    fx.git.commit_info_reads.load(Ordering::Relaxed)
}

/// The remote branch `name` at `tip`, committed to `age_days` ago.
fn remote(fx: &Fixture, name: &str, tip: &str, history: &[&str], age_days: i64) {
    fx.git.add_branch(name, false, tip, history);
    for b in fx.git.branches.lock().unwrap().iter_mut().filter(|b| b.name == name) {
        b.remote = true;
    }
    fx.git
        .times
        .lock()
        .unwrap()
        .insert(CommitId(tip.into()), -age_days * DAY);
}

fn row<'a>(list: &'a ChangesetList, title: &str) -> &'a ChangesetRow {
    list.rows.iter().find(|r| r.title == title).unwrap()
}

#[tokio::test]
async fn a_stale_remote_branch_is_neither_listed_nor_walked() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "one\n")]);
    fx.git.branches.lock().unwrap().retain(|b| b.name != "feature");
    remote(&fx, "origin/stale", "r1", &["m1"], 40);
    let opened = walks(&fx);

    let list = fx.session.changesets().await.unwrap();

    assert!(list.rows.iter().all(|r| r.title != "origin/stale"));
    assert_eq!(walks(&fx), opened);

    remote(&fx, "origin/fresh", "r2", &["m1"], 1);
    let list = fx.session.changesets().await.unwrap();

    assert!(list.rows.iter().any(|r| r.title == "origin/fresh"));
    assert_eq!(
        walks(&fx) - opened,
        2,
        "the fresh branch is walked, the stale one still is not"
    );
}

#[tokio::test]
async fn a_second_listing_with_unchanged_tips_walks_nothing() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);

    let first = fx.session.changesets().await.unwrap();
    let (walked, read) = (walks(&fx), info_reads(&fx));
    assert!(walked > 0 && read > 0);

    let second = fx.session.changesets().await.unwrap();

    assert_eq!((walks(&fx), info_reads(&fx)), (walked, read));
    assert_eq!(first, second);
}

#[tokio::test]
async fn a_moved_branch_is_walked_again_alone() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    fx.git.add_branch("other", false, "p1", &["m1"]);
    fx.session.changesets().await.unwrap();
    let (walked, read) = (walks(&fx), info_reads(&fx));

    fx.git.add_branch("feature", false, "tip2", &["tip1", "m1"]);
    let list = fx.session.changesets().await.unwrap();

    assert_eq!((walks(&fx) - walked, info_reads(&fx) - read), (2, 1));
    assert_eq!(
        row(&list, "feature").divergence,
        Some(Divergence { ahead: 2, behind: 1 })
    );
}

#[tokio::test]
async fn a_moved_default_branch_walks_every_branch_again() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    fx.git.add_branch("other", false, "p1", &["m1"]);
    fx.session.changesets().await.unwrap();
    let walked = walks(&fx);

    fx.git.add_commit("main", "m3", &[("a.txt", "one\n")]);
    fx.git.add_branch("main", true, "m3", &["m2", "m1"]);
    let list = fx.session.changesets().await.unwrap();

    assert_eq!(walks(&fx) - walked, 4);
    assert_eq!(
        row(&list, "feature").divergence,
        Some(Divergence { ahead: 1, behind: 2 })
    );
}

#[tokio::test]
async fn a_moved_upstream_recounts_only_unpushed() {
    let fx = Fixture::open(&[("a.txt", "one\n")], &[("a.txt", "one\n")]).await;
    fx.git.feature(&[("a.txt", "one\n")], &[("a.txt", "two\n")]);
    fx.git.add_commit("refs/heads/main", "m2", &[("a.txt", "one\n")]);
    fx.session
        .open_changeset(Source::Branch { name: "main".into() })
        .await
        .unwrap();
    remote(&fx, "origin/main", "o1", &["m1"], 40);
    let before = fx.session.changesets().await.unwrap();
    let walked = walks(&fx);

    remote(&fx, "origin/main", "o2", &[], 40);
    let after = fx.session.changesets().await.unwrap();

    assert_eq!(
        (row(&before, "main").unpushed, row(&after, "main").unpushed),
        (Some(1), Some(2))
    );
    assert_eq!(walks(&fx) - walked, 1);
}
