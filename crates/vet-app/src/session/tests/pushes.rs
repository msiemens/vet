//! The versions of a pull request are the pushes its platform lists, from the first one.

use super::*;

fn push(head: &str) -> Push {
    Push {
        head: head.into(),
        at: None,
        forced: false,
    }
}

/// Pull request 7 as it is fetched at its third push, `pr3`, with the earlier pushes `pr1` and
/// `pr2` in the repository, each adding an edit to `a.txt`. The platform lists all three.
async fn pushed_three_times() -> Fixture {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", &base)],
        "pr3",
        &[("a.txt", &numbered(30, &[3, 20, 25]))],
    );
    fx.git.add_commit("push pr1", "pr1", &[("a.txt", &numbered(30, &[3]))]);
    fx.git
        .add_commit("push pr2", "pr2", &[("a.txt", &numbered(30, &[3, 20]))]);
    fx.session
        .remember_pushes(&pull_request_7(), vec![push("pr1"), push("pr2"), push("pr3")]);
    fx.session.open_changeset(pull_request_7()).await.unwrap();
    fx
}

fn shape(versions: &[VersionView]) -> Vec<(u32, String, bool)> {
    versions
        .iter()
        .map(|v| {
            (
                v.n,
                v.tip.as_ref().map_or_else(String::new, |t| t.id.0.clone()),
                v.available,
            )
        })
        .collect()
}

fn pushed(shape: &[(u32, &str, bool)]) -> Vec<(u32, String, bool)> {
    shape.iter().map(|(n, tip, ok)| (*n, (*tip).to_owned(), *ok)).collect()
}

#[tokio::test]
async fn a_review_starts_with_every_push_the_platform_lists_as_a_version() {
    let fx = pushed_three_times().await;
    assert!(fx.session.versions().await.unwrap().is_empty(), "browsing has none");

    fx.begin().await;

    let versions = fx.session.versions().await.unwrap();
    assert_eq!(
        shape(&versions),
        pushed(&[(1, "pr1", true), (2, "pr2", true), (3, "pr3", true)])
    );
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 3, .. }
    ));
    assert_eq!(
        versions.iter().map(|v| v.change.files.len()).collect::<Vec<_>>(),
        [1, 1, 1]
    );
    let second = fx.session.manifest(View::Interdiff { from: 1, to: 2 }).await.unwrap();
    let [file] = &second.files[..] else {
        panic!("{:?}", second.files)
    };
    assert_eq!(file.hunks.len(), 1, "what push 2 added to push 1");
}

#[tokio::test]
async fn several_pushes_between_two_fetches_are_a_version_each() {
    let fx = pushed_three_times().await;
    fx.begin().await;
    let base = numbered(30, &[]);

    fx.git
        .add_commit("push pr4", "pr4", &[("a.txt", &numbered(30, &[3, 20, 25, 27]))]);
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", &base)],
        "pr5",
        &[("a.txt", &numbered(30, &[3, 20, 25, 27, 28]))],
    );
    fx.session.remember_pushes(
        &pull_request_7(),
        vec![push("pr1"), push("pr2"), push("pr3"), push("pr4"), push("pr5")],
    );
    fx.session.observe().await.unwrap().expect("pushed to");

    assert_eq!(
        shape(&fx.session.versions().await.unwrap()),
        pushed(&[
            (1, "pr1", true),
            (2, "pr2", true),
            (3, "pr3", true),
            (4, "pr4", true),
            (5, "pr5", true)
        ])
    );
}

#[tokio::test]
async fn a_push_whose_commits_cannot_be_had_is_listed_and_cannot_be_opened() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", &base)],
        "pr3",
        &[("a.txt", &numbered(30, &[3, 20, 25]))],
    );
    // The platform dropped the commits of pr2 after a force-push.
    fx.git.add_commit("push pr1", "pr1", &[("a.txt", &numbered(30, &[3]))]);
    fx.git.unknown_commits_fail.store(true, Ordering::Relaxed);
    fx.session
        .remember_pushes(&pull_request_7(), vec![push("pr1"), push("pr2"), push("pr3")]);
    fx.session.open_changeset(pull_request_7()).await.unwrap();

    fx.begin().await;

    let versions = fx.session.versions().await.unwrap();
    assert_eq!(
        shape(&versions),
        pushed(&[(1, "pr1", true), (2, "pr2", false), (3, "pr3", true)])
    );
    assert!(versions[1].change.files.is_empty());
    assert_eq!(
        versions[2].change.files.len(),
        1,
        "push 3 is compared with push 1, the one before it that has content"
    );
    for (from, to) in [(1, 2), (2, 3), (0, 2)] {
        let opened = fx.session.manifest(View::Interdiff { from, to }).await;
        assert!(
            matches!(opened, Err(SessionError::Invalid(_))),
            "{from}-{to}: {opened:?}"
        );
    }
    let around = fx.session.manifest(View::Interdiff { from: 1, to: 3 }).await.unwrap();
    assert_eq!(around.files[0].hunks.len(), 2, "pushes 2 and 3 together");
    assert!(matches!(
        fx.session.lifecycle().await,
        Lifecycle::Open { version: 3, .. }
    ));
}

#[tokio::test]
async fn a_review_comes_back_after_a_restart_whatever_its_pushes_were() {
    let base = numbered(30, &[]);
    let fx = Fixture::open(&[("a.txt", &base)], &[("a.txt", &base)]).await;
    fetch_pull_request_7(
        &fx.git,
        &[("a.txt", &base)],
        "pr3",
        &[("a.txt", &numbered(30, &[3, 20, 25]))],
    );
    fx.git.unknown_commits_fail.store(true, Ordering::Relaxed);
    fx.session
        .remember_pushes(&pull_request_7(), vec![push("pr1"), push("pr3")]);
    fx.session.open_changeset(pull_request_7()).await.unwrap();
    fx.begin().await;
    let kept = fx.store.0.lock().unwrap().versions.clone();
    assert_eq!(
        kept.iter().map(|v| v.snapshots.is_some()).collect::<Vec<_>>(),
        [false, true]
    );

    // The working tree's `HEAD`, which the restart looks at, is not the commit in question.
    fx.git.unknown_commits_fail.store(false, Ordering::Relaxed);
    let again = fx.reopen(&[("a.txt", &base)]).await;
    again.open_changeset(pull_request_7()).await.unwrap();

    assert!(matches!(again.lifecycle().await, Lifecycle::Open { version: 2, .. }));
    assert_eq!(
        shape(&again.versions().await.unwrap()),
        pushed(&[(1, "pr1", false), (2, "pr3", true)])
    );
}
