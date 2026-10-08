use std::collections::BTreeMap;
use std::time::Duration;

use sea_orm::PaginatorTrait;
use tempfile::TempDir;
use vet_app::changeset::{ChangedFile, CommitId, Lens, RemoteKey, VersionChange, VersionOrigin};
use vet_app::gc::{Retention, collect, open_bases};
use vet_app::ports::{GcReport, Store};
use vet_app::review::{NonTextAck, SpanOrigin};
use vet_core::content::{
    BlobId, EntryKind, FileEntry, Hash, LineDelta, LineRange, NonTextKind, RepoPath, Snapshot, SnapshotId,
    SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::review::{
    Anchor, CommentTarget, EditConflict, Message, PlatformThread, RemoteComment, ReviewedSpan, Side, SpanState,
    ThreadStatus,
};

use super::*;

const DAY: i64 = 24 * 3600;
/// A fixed "now", so retention and grace are deterministic.
const T0: i64 = 1_700_000_000;

fn h(n: u8) -> Hash {
    Hash([n; 32])
}

async fn open(dir: &TempDir) -> SqliteStore {
    SqliteStore::open(dir.path()).await.unwrap()
}

/// Collects with no grace period, so content is eligible as soon as it is unreferenced.
async fn gc(store: &SqliteStore, now: i64) -> GcReport {
    let retention = Retention {
        grace: Duration::ZERO,
        ..Default::default()
    };
    collect(store, now, &retention).await.unwrap()
}

fn file_entry(blob: u8) -> FileEntry {
    FileEntry {
        blob: BlobId::of(&[blob]),
        mode: 0o100644,
        kind: EntryKind::File,
        unfiltered: false,
    }
}

fn snapshot(files: &[(&str, u8)]) -> Snapshot {
    let entries = files
        .iter()
        .map(|(p, b)| (RepoPath(p.as_bytes().to_vec()), file_entry(*b)))
        .collect::<BTreeMap<_, _>>();
    Snapshot {
        id: merkle::snapshot_id(&entries),
        entries,
    }
}

fn anchor(side: Side, snapshot: u8) -> Anchor {
    Anchor {
        side,
        snapshot: SnapshotId(h(snapshot)),
        file: RepoPath(b"src/lib.rs".to_vec()),
        range: LineRange { start: 3, end: 9 },
        content_hash_raw: h(10),
        content_hash_norm: h(11),
        context_hashes: vec![h(12), h(13)],
    }
}

fn message(body: &str) -> Message {
    Message::written("me".into(), body.into(), 1_700_000_000)
}

async fn add_thread(
    store: &SqliteStore,
    changeset: ChangesetId,
    review: Option<ReviewId>,
    target: CommentTarget,
    opener: Message,
) -> ThreadId {
    store
        .add_thread(changeset, review, target, opener, None, ThreadStatus::Open)
        .await
        .unwrap()
        .0
}

fn commit(id: &str) -> CommitId {
    CommitId(id.into())
}

fn version(n: u32, base: SnapshotId, target: SnapshotId) -> Version {
    Version {
        n,
        base,
        base_commit: commit("abc123"),
        target,
        tip: commit("abc123"),
        at: i64::from(n) * 10,
        origin: VersionOrigin::Settled,
        change: VersionChange::default(),
    }
}

/// Opens a review of the working tree of `/repo`, whose version 1 is `base` and `target`.
async fn review(store: &SqliteStore, base: SnapshotId, target: SnapshotId) -> (ChangesetId, ReviewId) {
    store
        .start_review(
            "/repo".into(),
            Source::WorkingTree,
            BaseChoice::Commit { id: commit("abc123") },
            version(1, base, target),
            T0,
        )
        .await
        .unwrap()
}

fn sample_filter() -> ReviewFilter {
    ReviewFilter {
        roots: vec![RepoPath(b"crates".to_vec())],
        include: vec!["*.rs".into()],
        exclude: vec!["*.lock".into(), "gen/**".into()],
        hide_binary: true,
    }
}

#[tokio::test]
async fn the_filter_is_stored_per_repository_and_replaced_as_a_whole() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    assert_eq!(store.filter("/a".into()).await.unwrap(), ReviewFilter::default());

    store.set_filter("/a".into(), sample_filter()).await.unwrap();
    let other = ReviewFilter {
        exclude: vec!["b".into()],
        ..Default::default()
    };
    store.set_filter("/b".into(), other.clone()).await.unwrap();
    let narrower = ReviewFilter {
        exclude: vec!["x".into()],
        ..Default::default()
    };
    store.set_filter("/a".into(), narrower.clone()).await.unwrap();
    store.set_filter("/b".into(), other.clone()).await.unwrap();

    drop(store);
    let store = open(&dir).await;
    assert_eq!(store.filter("/a".into()).await.unwrap(), narrower);
    assert_eq!(store.filter("/b".into()).await.unwrap(), other);
    assert_eq!(store.filter("/c".into()).await.unwrap(), ReviewFilter::default());

    store.set_filter("/a".into(), sample_filter()).await.unwrap();
    assert_eq!(store.filter("/a".into()).await.unwrap(), sample_filter());
}

#[tokio::test]
async fn a_database_of_another_version_is_recreated_and_one_of_this_version_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let db = connect(dir.path()).await.unwrap();
    // What an earlier version left behind: its migrations and a differently shaped session table.
    for sql in [
        "CREATE TABLE seaql_migrations (version VARCHAR NOT NULL PRIMARY KEY, applied_at BIGINT NOT NULL)",
        "INSERT INTO seaql_migrations VALUES ('schema_0000000000000000', 1)",
        "CREATE TABLE session (id INTEGER PRIMARY KEY, repo TEXT)",
        "INSERT INTO session VALUES (1, '/old')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    db.close().await.unwrap();

    let store = open(&dir).await;
    assert_eq!(store.changeset("/old".into(), Source::WorkingTree).await.unwrap(), None);
    let (changeset, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    drop(store);

    let store = open(&dir).await;
    assert_eq!(
        store.changeset("/repo".into(), Source::WorkingTree).await.unwrap(),
        Some(changeset)
    );
    assert_eq!(store.open_review(changeset).await.unwrap(), Some(id));
}

#[tokio::test]
async fn a_changeset_is_found_by_its_repository_and_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let pull_request = |host: &str, path: &str, number| Source::PullRequest {
        remote: RemoteKey {
            host: host.into(),
            path: path.into(),
        },
        number,
    };
    let sources = [
        ("/repo", Source::WorkingTree),
        ("/other", Source::WorkingTree),
        ("/repo", Source::Branch { name: "main".into() }),
        ("/repo", Source::Branch { name: "".into() }),
        ("/repo", pull_request("github.com", "o/r", 1)),
        ("/repo", pull_request("github.com", "o/r", 2)),
        ("/repo", pull_request("github.com", "o/x", 1)),
        ("/repo", pull_request("example.com", "o/r", 1)),
    ];
    let mut ids = Vec::new();
    for (repo, source) in &sources {
        assert_eq!(store.changeset((*repo).into(), source.clone()).await.unwrap(), None);
        let (id, _) = store
            .start_review(
                (*repo).into(),
                source.clone(),
                BaseChoice::Commit { id: commit("a") },
                version(1, SnapshotId(h(1)), SnapshotId(h(2))),
                T0,
            )
            .await
            .unwrap();
        ids.push(id);
    }
    drop(store);

    let store = open(&dir).await;
    for ((repo, source), id) in sources.iter().zip(&ids) {
        assert_eq!(
            store.changeset((*repo).into(), source.clone()).await.unwrap(),
            Some(*id)
        );
    }
    let mut distinct = ids.clone();
    distinct.sort_by_key(|id| id.0);
    distinct.dedup();
    assert_eq!(distinct.len(), sources.len(), "every source is a changeset of its own");
}

#[tokio::test]
async fn the_lens_is_remembered_per_changeset_before_and_after_a_review_of_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let feature = || Source::Branch { name: "feature".into() };
    assert_eq!(store.lens("/repo".into(), feature()).await.unwrap(), None);

    store
        .set_lens("/repo".into(), feature(), Lens::PerCommit)
        .await
        .unwrap();
    store
        .start_review(
            "/repo".into(),
            feature(),
            BaseChoice::Commit { id: commit("a") },
            version(1, SnapshotId(h(1)), SnapshotId(h(2))),
            T0,
        )
        .await
        .unwrap();
    drop(store);

    let store = open(&dir).await;
    assert_eq!(
        store.lens("/repo".into(), feature()).await.unwrap(),
        Some(Lens::PerCommit)
    );
    assert_eq!(
        store
            .lens("/repo".into(), Source::Branch { name: "other".into() })
            .await
            .unwrap(),
        None
    );
    store.set_lens("/repo".into(), feature(), Lens::Squash).await.unwrap();
    assert_eq!(store.lens("/repo".into(), feature()).await.unwrap(), Some(Lens::Squash));
}

#[tokio::test]
async fn the_base_choice_is_stored_with_the_review_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (_, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let loaded = |store: &SqliteStore| {
        let store = store.clone();
        async move { store.review(id).await.unwrap().unwrap().base }
    };
    assert_eq!(loaded(&store).await, BaseChoice::Commit { id: commit("abc123") });

    let merge_base = BaseChoice::MergeBase {
        reference: "main".into(),
    };
    store.set_base(id, merge_base.clone()).await.unwrap();
    assert_eq!(loaded(&store).await, merge_base);

    let commit_choice = BaseChoice::Commit { id: commit("abc") };
    store.set_base(id, commit_choice.clone()).await.unwrap();
    drop(store);
    let store = open(&dir).await;
    assert_eq!(loaded(&store).await, commit_choice);
}

#[tokio::test]
async fn the_browsing_base_is_stored_per_repository_and_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    assert_eq!(store.browsing_base("/a".into()).await.unwrap(), None);

    let first = BaseChoice::MergeBase {
        reference: "develop".into(),
    };
    let merge_base = BaseChoice::MergeBase {
        reference: "main".into(),
    };
    store.set_browsing_base("/a".into(), first).await.unwrap();
    store.set_browsing_base("/b".into(), merge_base.clone()).await.unwrap();
    let commit_choice = BaseChoice::Commit { id: commit("abc") };
    store
        .set_browsing_base("/a".into(), commit_choice.clone())
        .await
        .unwrap();

    drop(store);
    let store = open(&dir).await;
    assert_eq!(store.browsing_base("/a".into()).await.unwrap(), Some(commit_choice));
    assert_eq!(store.browsing_base("/b".into()).await.unwrap(), Some(merge_base));
    assert_eq!(store.browsing_base("/c".into()).await.unwrap(), None);
}

#[tokio::test]
async fn database_is_in_wal_mode() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    store.put_snapshot(snapshot(&[]), Default::default(), T0).await.unwrap();
    assert!(dir.path().join("vet.sqlite-wal").exists());
}

#[tokio::test]
async fn spans_and_acks_belong_to_the_review_and_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (_, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let (_, other) = store
        .start_review(
            "/other".into(),
            Source::WorkingTree,
            BaseChoice::Commit { id: commit("abc123") },
            version(1, SnapshotId(h(1)), SnapshotId(h(2))),
            T0,
        )
        .await
        .unwrap();

    let spans = vec![
        ReviewedSpan {
            base: Some(anchor(Side::Base, 1)),
            target: Some(anchor(Side::Target, 2)),
            state: SpanState::Seen,
        },
        ReviewedSpan {
            base: None,
            target: Some(anchor(Side::Target, 2)),
            state: SpanState::SeenAgainstOldBase,
        },
        ReviewedSpan {
            base: Some(anchor(Side::Base, 1)),
            target: None,
            state: SpanState::Seen,
        },
    ];
    let origins = [SpanOrigin::Squash, SpanOrigin::Commit(commit("c1")), SpanOrigin::Squash];
    let added = spans.iter().cloned().zip(origins.iter().cloned()).collect();
    let ids = store.edit_spans(id, vec![], added).await.unwrap();
    assert_eq!(ids.len(), 3);

    let ack = NonTextAck {
        kind: NonTextKind::Binary,
        before: Some((RepoPath(b"x".to_vec()), file_entry(5))),
        after: None,
    };
    store.acknowledge(id, ack.clone()).await.unwrap();
    store.acknowledge(id, ack.clone()).await.unwrap();
    // Same entries, another kind: a separate item.
    let mode = NonTextAck {
        kind: NonTextKind::Mode,
        ..ack.clone()
    };
    store.acknowledge(id, mode.clone()).await.unwrap();

    let loaded = store.review(id).await.unwrap().unwrap();
    assert_eq!(loaded.spans.iter().map(|r| r.span.clone()).collect::<Vec<_>>(), spans);
    assert_eq!(
        loaded.spans.iter().map(|r| r.origin.clone()).collect::<Vec<_>>(),
        origins
    );
    assert_eq!(loaded.acks, vec![ack.clone(), mode.clone()]);
    let elsewhere = store.review(other).await.unwrap().unwrap();
    assert!(elsewhere.spans.is_empty() && elsewhere.acks.is_empty());

    store.unacknowledge(id, ack).await.unwrap();
    assert_eq!(store.review(id).await.unwrap().unwrap().acks, vec![mode.clone()]);
    store.unacknowledge(id, mode).await.unwrap();
    store.edit_spans(id, ids, vec![]).await.unwrap();
    let loaded = store.review(id).await.unwrap().unwrap();
    assert!(loaded.spans.is_empty() && loaded.acks.is_empty());
}

#[tokio::test]
async fn threads_belong_to_the_changeset_and_outlive_its_review() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, first) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;

    let range = add_thread(
        &store,
        changeset,
        Some(first),
        CommentTarget::Range(anchor(Side::Target, 2)),
        message("why?"),
    )
    .await;
    let file = add_thread(
        &store,
        changeset,
        Some(first),
        CommentTarget::File {
            side: Side::Base,
            path: RepoPath(b"a/b".to_vec()),
        },
        message("file"),
    )
    .await;
    let overall = add_thread(&store, changeset, None, CommentTarget::Changeset, message("overall")).await;
    store.add_message(range, message("because")).await.unwrap();
    store.set_thread_status(file, ThreadStatus::Resolved).await.unwrap();

    store.close_review(first, T0, true).await.unwrap();
    let (again, second) = review(&store, SnapshotId(h(1)), SnapshotId(h(3))).await;
    assert_eq!(again, changeset);
    assert_ne!(second, first);

    let threads = store.threads(changeset).await.unwrap();
    assert_eq!(
        threads.iter().map(|t| t.thread.id).collect::<Vec<_>>(),
        [range, file, overall]
    );
    assert_eq!(
        threads.iter().map(|t| t.review).collect::<Vec<_>>(),
        [Some(first), Some(first), None],
        "a thread keeps the review it was opened in"
    );
    assert_eq!(threads[0].thread.target, CommentTarget::Range(anchor(Side::Target, 2)));
    let bodies: Vec<_> = threads[0].thread.messages.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, ["why?", "because"]);
    assert_eq!(threads[1].thread.status, ThreadStatus::Resolved);
    assert_eq!(
        threads[1].thread.target,
        CommentTarget::File {
            side: Side::Base,
            path: RepoPath(b"a/b".to_vec())
        }
    );
    assert_eq!(threads[2].thread.target, CommentTarget::Changeset);

    let moved = CommentTarget::Range(Anchor {
        range: LineRange { start: 20, end: 25 },
        ..anchor(Side::Target, 2)
    });
    store.set_thread_target(range, moved.clone()).await.unwrap();
    store.delete_thread(file).await.unwrap();
    let threads = store.threads(changeset).await.unwrap();
    assert_eq!(
        threads.iter().map(|t| &t.thread.target).collect::<Vec<_>>(),
        vec![&moved, &CommentTarget::Changeset]
    );
}

fn posted(id: &str, synced: &str, body: &str) -> Message {
    Message {
        mine: false,
        avatar: Some("https://avatars.example/lena".into()),
        remote: Some(RemoteComment {
            id: id.into(),
            synced: synced.into(),
            conflict: None,
        }),
        ..Message::written("lena".into(), body.into(), T0)
    }
}

#[tokio::test]
async fn what_a_platform_has_of_a_thread_survives_a_restart_and_a_thread_needs_no_review() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let source = Source::PullRequest {
        remote: RemoteKey {
            host: "github.com".into(),
            path: "acme/app".into(),
        },
        number: 7,
    };
    let changeset = store.ensure_changeset("/repo".into(), source.clone()).await.unwrap();
    assert_eq!(
        store.ensure_changeset("/repo".into(), source.clone()).await.unwrap(),
        changeset
    );
    assert_eq!(
        store.open_review(changeset).await.unwrap(),
        None,
        "no review is started by it"
    );

    let platform = PlatformThread {
        remote_id: Some("T1".into()),
        resolvable: true,
        resolved_by: Some("lena".into()),
        status_pending: false,
    };
    let (thread, opener) = store
        .add_thread(
            changeset,
            None,
            CommentTarget::Range(anchor(Side::Target, 2)),
            posted("C1", "why?", "why?"),
            Some(platform.clone()),
            ThreadStatus::Resolved,
        )
        .await
        .unwrap();
    // A draft of the viewer's is written after the platform's reply, which then comes second.
    let draft = store.add_message(thread, message("agreed")).await.unwrap();
    let reply = store.add_message(thread, posted("C2", "no", "no")).await.unwrap();

    let edited = Message {
        id: opener,
        body: "why not?".into(),
        edited_at: Some(T0 + 5),
        remote: Some(RemoteComment {
            id: "C1".into(),
            synced: "why?".into(),
            conflict: Some(EditConflict {
                remote_body: "why so?".into(),
                remote_at: T0 + 9,
            }),
        }),
        ..posted("C1", "why?", "why?")
    };
    store.replace_message(edited.clone()).await.unwrap();
    store.delete_message(reply).await.unwrap();
    store
        .set_thread_platform(
            thread,
            PlatformThread {
                status_pending: true,
                ..platform.clone()
            },
        )
        .await
        .unwrap();

    drop(store);
    let store = open(&dir).await;
    assert_eq!(store.changeset("/repo".into(), source).await.unwrap(), Some(changeset));
    let [found] = <[_; 1]>::try_from(store.threads(changeset).await.unwrap()).expect("one thread");
    let found = found.thread;
    assert_eq!(found.status, ThreadStatus::Resolved);
    assert_eq!(
        found.platform,
        Some(PlatformThread {
            status_pending: true,
            ..platform
        })
    );
    assert!(found.is_posted());
    assert_eq!(found.messages.len(), 2, "the reply was removed");
    assert_eq!(found.messages[0], edited);
    assert_eq!(
        (found.messages[1].id, found.messages[1].remote.clone()),
        (draft, None),
        "a draft has no remote"
    );
    assert!(found.messages[1].mine);
}

#[tokio::test]
async fn a_thread_outside_a_platform_has_none_and_a_review_started_later_finds_the_changeset() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, first) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    add_thread(
        &store,
        changeset,
        Some(first),
        CommentTarget::Changeset,
        message("local"),
    )
    .await;

    let [found] = <[_; 1]>::try_from(store.threads(changeset).await.unwrap()).expect("one thread");

    assert_eq!(found.thread.platform, None);
    assert!(!found.thread.is_posted());
    let again = store
        .ensure_changeset("/repo".into(), Source::WorkingTree)
        .await
        .unwrap();
    assert_eq!(again, changeset);
}

#[tokio::test]
async fn the_open_bases_are_the_latest_versions_of_the_changesets_with_an_open_review() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (open_one, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    store
        .append_version(open_one, version(2, SnapshotId(h(3)), SnapshotId(h(2))))
        .await
        .unwrap();
    let (_, closed) = store
        .start_review(
            "/other".into(),
            Source::WorkingTree,
            BaseChoice::Commit { id: commit("abc123") },
            version(1, SnapshotId(h(4)), SnapshotId(h(5))),
            T0,
        )
        .await
        .unwrap();
    store.close_review(closed, T0, true).await.unwrap();

    let bases = open_bases(&store).await.unwrap();

    assert_eq!(bases, HashSet::from([SnapshotId(h(3))]));
}

#[tokio::test]
async fn versions_are_numbered_per_changeset_and_a_new_review_starts_over() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    let changeset;
    {
        let store = open(&dir).await;
        (changeset, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
        let moved = Version {
            base_commit: commit("def456"),
            origin: VersionOrigin::BaseMoved,
            ..version(3, SnapshotId(h(4)), SnapshotId(h(3)))
        };
        store
            .append_version(changeset, version(2, SnapshotId(h(1)), SnapshotId(h(3))))
            .await
            .unwrap();
        store.append_version(changeset, moved).await.unwrap();
        assert!(
            store
                .append_version(changeset, version(3, SnapshotId(h(1)), SnapshotId(h(5))))
                .await
                .is_err(),
            "a number is used once"
        );
    }
    let store = open(&dir).await;
    let versions = store.versions(changeset).await.unwrap();
    assert_eq!(
        versions
            .iter()
            .map(|v| (v.n, v.base, v.target, v.origin))
            .collect::<Vec<_>>(),
        [
            (1, SnapshotId(h(1)), SnapshotId(h(2)), VersionOrigin::Settled),
            (2, SnapshotId(h(1)), SnapshotId(h(3)), VersionOrigin::Settled),
            (3, SnapshotId(h(4)), SnapshotId(h(3)), VersionOrigin::BaseMoved),
        ]
    );
    assert_eq!(
        (versions[2].base_commit.clone(), versions[2].tip.clone(), versions[2].at),
        (commit("def456"), commit("abc123"), 30)
    );

    store.close_review(id, T0, false).await.unwrap();
    let (_, next) = review(&store, SnapshotId(h(6)), SnapshotId(h(7))).await;
    assert_ne!(next, id);
    let versions = store.versions(changeset).await.unwrap();
    assert_eq!(
        versions.iter().map(|v| (v.n, v.target)).collect::<Vec<_>>(),
        [(1, SnapshotId(h(7)))]
    );
}

#[tokio::test]
async fn what_a_version_changed_is_stored_with_it_in_order_and_goes_with_its_review() {
    let dir = tempfile::tempdir().unwrap();
    let changed = |path: &str, added, removed| ChangedFile {
        path: RepoPath(path.as_bytes().to_vec()),
        delta: LineDelta { added, removed },
    };
    let change = VersionChange {
        files: vec![changed("a.rs", 3, 1), changed("b.rs", 0, 0), changed("c/d.rs", 40, 0)],
    };
    let changeset;
    {
        let store = open(&dir).await;
        (changeset, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
        let second = Version {
            change: change.clone(),
            ..version(2, SnapshotId(h(1)), SnapshotId(h(3)))
        };
        store.append_version(changeset, second).await.unwrap();
    }

    let store = open(&dir).await;
    let versions = store.versions(changeset).await.unwrap();
    assert_eq!(versions[0].change, VersionChange::default());
    assert_eq!(versions[1].change, change);

    review(&store, SnapshotId(h(6)), SnapshotId(h(7))).await;
    let rows = version_file::Entity::find().all(&store.db).await.unwrap();
    assert!(rows.is_empty(), "the files of the replaced versions are gone");
}

#[tokio::test]
async fn a_review_records_when_it_ended_and_what_was_last_reviewed() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (changeset, id) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    let open_review = store.review(id).await.unwrap().unwrap();
    assert_eq!(
        (open_review.changeset, open_review.last_reviewed, open_review.opened_at),
        (changeset, None, T0)
    );
    assert_eq!((open_review.closed_at, open_review.complete), (None, None));
    assert_eq!(store.open_review(changeset).await.unwrap(), Some(id));
    assert_eq!(store.open_review(ChangesetId(changeset.0 + 1)).await.unwrap(), None);

    store.set_last_reviewed(id, 2).await.unwrap();
    store.close_review(id, 1, true).await.unwrap();
    assert_eq!(store.open_review(changeset).await.unwrap(), None);
    let closed = store.review(id).await.unwrap().unwrap();
    assert_eq!(
        (closed.last_reviewed, closed.closed_at, closed.complete),
        (Some(2), Some(1), Some(true))
    );
}

#[tokio::test]
async fn snapshots_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let snap = snapshot(&[("a", 1), ("b/c/d", 2), ("b/e", 3)]);
    let status = SnapshotStatus {
        path_collisions: vec![RepoPath(b"A".to_vec())],
        content_unavailable: vec![RepoPath(b"big".to_vec())],
    };
    {
        let store = open(&dir).await;
        store.put_snapshot(snap.clone(), status.clone(), T0).await.unwrap();
        store.put_snapshot(snap.clone(), status.clone(), T0).await.unwrap();
    }
    let store = open(&dir).await;
    assert_eq!(store.snapshot(snap.id).await.unwrap(), Some((snap, status)));
    assert_eq!(store.snapshot(SnapshotId(h(99))).await.unwrap(), None);
}

#[tokio::test]
async fn gc_keeps_referenced_content_and_drops_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let snaps = [21u8, 22, 23].map(|n| snapshot(&[("a", n)]));
    for s in &snaps {
        store.put_snapshot(s.clone(), Default::default(), T0).await.unwrap();
    }
    for n in [21, 22, 23, 24] {
        store.put_blob(vec![n]).await.unwrap();
    }
    let (changeset, id) = review(&store, snaps[0].id, snaps[1].id).await;
    let now = T0;

    let report = gc(&store, now).await;
    assert_eq!(
        (
            report.changesets_deleted,
            report.snapshots_deleted,
            report.nodes_deleted,
            report.blobs_deleted
        ),
        (0, 1, 1, 2)
    );
    assert!(store.blob(BlobId::of(&[21])).await.unwrap().is_some());
    assert!(store.blob(BlobId::of(&[22])).await.unwrap().is_some());
    assert_eq!(store.blob(BlobId::of(&[23])).await.unwrap(), None);
    assert_eq!(store.blob(BlobId::of(&[24])).await.unwrap(), None);
    assert!(store.snapshot(snaps[2].id).await.unwrap().is_none());

    // A changeset is kept for the retention period after its review closed.
    store.close_review(id, now, false).await.unwrap();
    assert_eq!(gc(&store, now + 29 * DAY).await, Default::default());
    assert!(store.review(id).await.unwrap().is_some());

    // An open thread keeps an expired changeset.
    let thread = add_thread(&store, changeset, Some(id), CommentTarget::Changeset, message("todo")).await;
    assert_eq!(gc(&store, now + 31 * DAY).await, Default::default());
    store.set_thread_status(thread, ThreadStatus::Resolved).await.unwrap();

    // So does an open review, and the retention counts from the last closed one.
    let (_, next) = review(&store, snaps[0].id, snaps[1].id).await;
    assert_eq!(gc(&store, now + 31 * DAY).await, Default::default());
    store.close_review(next, now + 31 * DAY, true).await.unwrap();
    assert_eq!(gc(&store, now + 32 * DAY).await, Default::default());

    let report = gc(&store, now + 62 * DAY).await;
    assert_eq!(
        (
            report.changesets_deleted,
            report.snapshots_deleted,
            report.nodes_deleted,
            report.blobs_deleted
        ),
        (1, 2, 2, 2)
    );
    assert!(store.review(id).await.unwrap().is_none());
    assert!(store.threads(changeset).await.unwrap().is_empty());
    assert!(store.versions(changeset).await.unwrap().is_empty());
    assert_eq!(
        store.changeset("/repo".into(), Source::WorkingTree).await.unwrap(),
        None
    );
    assert_eq!(store.blob(BlobId::of(&[21])).await.unwrap(), None);
}

#[tokio::test]
async fn gc_keeps_what_an_open_thread_of_a_closed_review_is_anchored_in() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let snaps = [31u8, 32, 33].map(|n| snapshot(&[("a", n)]));
    for s in &snaps {
        store.put_snapshot(s.clone(), Default::default(), T0).await.unwrap();
    }
    let (changeset, id) = review(&store, snaps[0].id, snaps[1].id).await;
    let anchored = Anchor {
        snapshot: snaps[2].id,
        ..anchor(Side::Target, 0)
    };
    add_thread(
        &store,
        changeset,
        Some(id),
        CommentTarget::Range(anchored),
        message("open"),
    )
    .await;
    store.close_review(id, T0, false).await.unwrap();

    let report = gc(&store, T0 + 100 * DAY).await;
    assert_eq!((report.changesets_deleted, report.snapshots_deleted), (0, 0));
    for s in &snaps {
        assert!(store.snapshot(s.id).await.unwrap().is_some());
    }
}

#[tokio::test]
async fn gc_grace_protects_fresh_content() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let retention = Retention::default();
    store.put_blob(vec![1]).await.unwrap();
    store
        .put_snapshot(snapshot(&[("a", 1)]), Default::default(), T0)
        .await
        .unwrap();
    assert_eq!(collect(&store, T0, &retention).await.unwrap(), Default::default());
    assert!(store.blob(BlobId::of(&[1])).await.unwrap().is_some());
    assert!(store.snapshot(snapshot(&[("a", 1)]).id).await.unwrap().is_some());

    let later = collect(&store, T0 + 3600, &retention).await.unwrap();
    assert_eq!(later.snapshots_deleted, 1);
}

async fn node_count(store: &SqliteStore) -> u64 {
    tree_node::Entity::find().count(&store.db).await.unwrap()
}

#[tokio::test]
async fn snapshots_that_differ_in_one_file_share_all_other_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let before = snapshot(&[("a/x", 1), ("b/y", 2), ("c/d/z", 3)]);
    let after = snapshot(&[("a/x", 9), ("b/y", 2), ("c/d/z", 3)]);
    store
        .put_snapshot(before.clone(), Default::default(), T0)
        .await
        .unwrap();
    assert_eq!(node_count(&store).await, 5); // root, a, b, c, c/d
    store.put_snapshot(after.clone(), Default::default(), T0).await.unwrap();
    assert_eq!(node_count(&store).await, 7); // plus a new root and a new `a`
    assert_eq!(store.snapshot(before.id).await.unwrap().unwrap().0, before);
    assert_eq!(store.snapshot(after.id).await.unwrap().unwrap().0, after);
}

#[tokio::test]
async fn gc_drops_a_node_only_when_no_retained_root_reaches_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let kept = snapshot(&[("a/x", 1), ("b/y", 2)]);
    let other = snapshot(&[("a/x", 9), ("b/y", 2)]);
    for s in [&kept, &other] {
        store.put_snapshot(s.clone(), Default::default(), T0).await.unwrap();
    }
    assert_eq!(node_count(&store).await, 5);
    review(&store, kept.id, kept.id).await;

    let report = gc(&store, T0).await;
    assert_eq!((report.snapshots_deleted, report.nodes_deleted), (1, 2)); // other's root and `a`; `b` is shared
    assert_eq!(node_count(&store).await, 3);
    assert_eq!(store.snapshot(kept.id).await.unwrap().unwrap().0, kept);
}

#[tokio::test]
async fn a_long_run_of_versions_over_a_large_tree_stays_small() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let mut entries: BTreeMap<RepoPath, FileEntry> = (0..2000)
        .map(|i| (RepoPath(format!("d{:02}/f{i}", i % 100).into_bytes()), file_entry(0)))
        .collect();
    let (changeset, _) = review(&store, SnapshotId(h(1)), SnapshotId(h(2))).await;
    for i in 0..50u32 {
        let path = RepoPath(format!("d{:02}/f{}", (i * 7) % 100, (i * 7) % 100).into_bytes());
        entries.insert(
            path,
            FileEntry {
                blob: BlobId::of(&i.to_le_bytes()),
                ..file_entry(0)
            },
        );
        let snap = Snapshot {
            id: merkle::snapshot_id(&entries),
            entries: entries.clone(),
        };
        store.put_snapshot(snap.clone(), Default::default(), T0).await.unwrap();
        store
            .append_version(changeset, version(i + 2, SnapshotId(h(1)), snap.id))
            .await
            .unwrap();
    }
    // 101 nodes for the first snapshot (which includes edit 0), then a root and one directory per later edit.
    assert_eq!(node_count(&store).await, 101 + 2 * 49);
    let rows = tree_child::Entity::find().count(&store.db).await.unwrap();
    assert!(rows < 2000 + 100 + 50 * 125, "{rows} child rows");
    let last = store.versions(changeset).await.unwrap().last().unwrap().target;
    assert_eq!(store.snapshot(last).await.unwrap().unwrap().0.entries, entries);
}

#[tokio::test]
async fn recorded_spans_survive_remapping_so_a_revert_restores_coverage() {
    use vet_core::content::Changeset;
    use vet_core::prepared::PreparedFile;
    use vet_core::remap::{RemapMode, remap_span};
    use vet_core::review::FileRemapInput;

    let text = |line15: &str| -> String {
        (0..40)
            .map(|i| {
                if i == 15 {
                    format!("{line15}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect()
    };
    let (original, edited) = (text("line 15"), text("changed"));
    let (o, e) = (BlobId::of(original.as_bytes()), BlobId::of(edited.as_bytes()));
    let blobs = BTreeMap::from([(o, original.clone().into_bytes()), (e, edited.into_bytes())]);
    let path = || Some(RepoPath(b"a.rs".to_vec()));
    let input = |new| FileRemapInput {
        old_path: path(),
        new_path: path(),
        old_blob: Some(o),
        new_blob: Some(new),
    };
    let now = Changeset {
        base: SnapshotId(h(2)),
        target: SnapshotId(h(3)),
    };

    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir).await;
    let (_, id) = review(&store, SnapshotId(h(2)), SnapshotId(h(3))).await;
    let recorded = Anchor::from_text(
        Side::Target,
        SnapshotId(h(1)),
        path().unwrap(),
        original.as_bytes(),
        LineRange { start: 10, end: 30 },
    )
    .unwrap();
    let span = ReviewedSpan {
        base: None,
        target: Some(recorded),
        state: SpanState::Seen,
    };
    let ids = store
        .edit_spans(id, vec![], vec![(span.clone(), SpanOrigin::Squash)])
        .await
        .unwrap();

    let remapped = async |new| {
        let review = store.review(id).await.unwrap().unwrap();
        let input = input(new);
        let target = PreparedFile::new(&input, &blobs);
        review
            .spans
            .iter()
            .flat_map(|r| remap_span(&r.span, now, None, Some(&target), RemapMode::SameUnit))
            .count()
    };
    assert_eq!(remapped(e).await, 2, "the edited line splits the span");
    assert_eq!(remapped(o).await, 1, "reverted: full coverage again");
    assert_eq!(
        store.review(id).await.unwrap().unwrap().spans[0].span,
        span,
        "nothing was overwritten"
    );

    store.edit_spans(id, ids, vec![]).await.unwrap();
    assert_eq!(remapped(o).await, 0, "un-marked");
}
