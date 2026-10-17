//! The contract of the store ports ([`crate::ports`]: `Store`, `ChangesetStore`,
//! `ReviewStore`, `ThreadStore`, `PreferenceStore`, written through `ReviewWrites`), checked
//! once and generic over the store, so that the in-memory fake of `session/tests.rs` and
//! `vet-infra`'s `SqliteStore` cannot drift apart (audit T4): what one satisfies, the other
//! must satisfy too. The fake runs it from this module's tests; the real store runs it from
//! `vet-infra/tests/it`, which enables the `contract` feature to reach this module, without
//! `vet-app` depending on `vet-infra` (`ci/check-deps.sh` forbids that).
//!
//! The `SettingsStore` and `GcStore` ports have no fake, so [`settings_contract`] and
//! [`gc_contract`] run against the real store alone, from `vet-infra/tests/it`.
//!
//! Every case opens its own store empty: the fake models one repository's changesets at a time,
//! and the checks that tell the two apart are those the ports phrase as "per repository",
//! "per changeset", "per review".

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use vet_core::content::{
    BlobId, EntryKind, FileEntry, Hash, LineDelta, LineRange, NonTextKind, RepoPath, Snapshot, SnapshotId,
    SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::merkle;
use vet_core::review::{
    Anchor, CommentTarget, EditConflict, Message, Outgoing, PendingVote, PlatformThread, RemoteComment, ReviewedSpan,
    Side, SpanState, ThreadStatus, Verdict,
};

use crate::changeset::{
    BaseChoice, ChangedFile, ChangesetId, CommitId, Lens, RemoteKey, Snapshots, Source, Version, VersionChange,
    VersionOrigin,
};
use crate::platform::{Discussion, PullRequestHead, PullRequestState, ReviewDecision};
use crate::ports::{
    GcFacts, GcPlan, GcStore, LiveStore, ProjectStore, RecentProject, RecentStore, ReviewRow, ReviewWrites, Settings,
    SettingsStore, Store, StoredBlob,
};
use crate::review::{NonTextAck, ReviewId, ReviewSummary, SpanOrigin};
use crate::trust::PullRequestTrust;

/// The pull request rows the store keeps; the copy read longest ago goes first (the finding's
/// "eviction bounds like the kept pull requests").
const KEPT_PULL_REQUESTS: usize = 256;
const T0: i64 = 1_700_000_000;
const REPO: &str = "/contract/repo";
const OTHER_REPO: &str = "/contract/other";
const PLATFORM_HEAD: &str = "platform-head-1";

/// Runs every case against a store each case opens empty.
pub async fn store_contract<S: LiveStore>(mut open: impl AsyncFnMut() -> S) {
    {
        let store = open().await;
        keeps_its_blobs_and_snapshots(&store).await;
    }
    {
        let store = open().await;
        finds_changesets_by_their_repository_and_source(&store).await;
    }
    {
        let store = open().await;
        replaces_the_versions_at_a_new_review(&store).await;
    }
    {
        let store = open().await;
        a_review_owns_its_marks_and_reads_end_to_end(&store).await;
    }
    {
        let store = open().await;
        threads_have_their_messages_and_belong_to_their_changeset(&store).await;
    }
    {
        let store = open().await;
        keeps_a_newest_copy_of_pull_request_details_up_to_the_bound(&store).await;
    }
    {
        let store = open().await;
        holds_a_repositorys_settings_apart(&store).await;
    }
    {
        let store = open().await;
        keeps_preferences_per_scope_by_key(&store).await;
    }
    {
        let store = open().await;
        what_the_store_was_never_told_of_reads_empty(&store).await;
    }
}

// The values of the cases.

fn tree(n: u8) -> SnapshotId {
    SnapshotId(Hash([n; 32]))
}

fn commit(id: &str) -> CommitId {
    CommitId(id.into())
}

fn entry_of(text: &[u8], mode: u32) -> FileEntry {
    FileEntry {
        blob: BlobId::of(text),
        mode,
        kind: EntryKind::File,
        unfiltered: false,
    }
}

fn entry(blob: u8) -> FileEntry {
    entry_of(&[blob], 0o100644)
}

fn snapshot(files: &[(&str, u8)]) -> Snapshot {
    let entries: BTreeMap<RepoPath, FileEntry> = files.iter().map(|(p, b)| (RepoPath::from(*p), entry(*b))).collect();
    Snapshot {
        id: merkle::snapshot_id(&entries),
        entries,
    }
}

/// A snapshot whose id claims another's content: `put_snapshot` must not keep it.
fn mismarked_snapshot() -> Snapshot {
    let entries: BTreeMap<RepoPath, FileEntry> = [(RepoPath::from("a.txt"), entry(4))].into_iter().collect();
    Snapshot { id: tree(5), entries }
}

fn version_of(n: u32) -> Version {
    Version {
        n,
        snapshots: Some(Snapshots {
            base: tree(1),
            target: tree(11),
        }),
        base_commit: commit("abc123"),
        tip: commit("def456"),
        at: T0 + i64::from(n) * 10,
        origin: VersionOrigin::Settled,
        change: VersionChange {
            files: vec![ChangedFile {
                path: RepoPath::from("src/lib.rs"),
                delta: LineDelta { added: n, removed: 1 },
            }],
        },
    }
}

/// A version whose commits Vet could not fetch: listed, but with nothing to show.
fn version_without_snapshots(n: u32) -> Version {
    Version {
        snapshots: None,
        ..version_of(n)
    }
}

fn message(body: &str) -> Message {
    Message::written("me".into(), body.into(), T0)
}

fn posted(id: &str, body: &str) -> Message {
    Message {
        mine: false,
        avatar: Some("https://avatars.example/lena".into()),
        remote: Some(RemoteComment {
            id: id.into(),
            synced: body.into(),
            conflict: None,
        }),
        ..message(body)
    }
}

fn anchor(side: Side, snapshot: u8) -> Anchor {
    Anchor {
        side,
        snapshot: tree(snapshot),
        file: RepoPath::from("src/lib.rs"),
        range: LineRange { start: 3, end: 9 },
        content_hash_raw: Hash([10u8; 32]),
        content_hash_norm: Hash([11u8; 32]),
        context_hashes: vec![Hash([12u8; 32]), Hash([13u8; 32])],
    }
}

fn seen_both() -> ReviewedSpan {
    ReviewedSpan {
        base: Some(anchor(Side::Base, 21)),
        target: Some(anchor(Side::Target, 22)),
        state: SpanState::Seen,
    }
}

fn seen_against_the_old_base() -> ReviewedSpan {
    ReviewedSpan {
        base: None,
        target: Some(anchor(Side::Target, 23)),
        state: SpanState::SeenAgainstOldBase,
    }
}

fn flagged() -> ReviewedSpan {
    ReviewedSpan {
        base: Some(anchor(Side::Base, 24)),
        target: None,
        state: SpanState::Flagged,
    }
}

fn summary() -> ReviewSummary {
    ReviewSummary {
        total: 9,
        seen: 3,
        old_base: 1,
    }
}

fn mode_change() -> NonTextAck {
    NonTextAck {
        kind: NonTextKind::Mode,
        before: Some((RepoPath::from("run.sh"), entry_of(b"echo", 0o100644))),
        after: Some((RepoPath::from("run.sh"), entry_of(b"echo", 0o100755))),
    }
}

fn binary_change() -> NonTextAck {
    NonTextAck {
        kind: NonTextKind::Binary,
        before: Some((RepoPath::from("x"), entry(5))),
        after: None,
    }
}

fn filter() -> ReviewFilter {
    ReviewFilter {
        roots: vec![RepoPath::from("crates")],
        include: vec!["*.rs".into()],
        exclude: vec!["gen/**".into()],
        hide_binary: true,
        review_generated: true,
    }
}

fn other_filter() -> ReviewFilter {
    ReviewFilter {
        roots: vec![RepoPath::from("tools")],
        hide_binary: true,
        ..ReviewFilter::default()
    }
}

fn discussion(name: &str) -> Discussion {
    Discussion {
        title: format!("the change of {name}"),
        url: format!("https://host.example/{name}/1"),
        description: String::new(),
        draft: false,
        source_branch: "feature".into(),
        target_branch: "main".into(),
        state: PullRequestState::Open,
        author: "ann".into(),
        author_name: "Ann".into(),
        author_avatar: None,
        reviewers: Vec::new(),
        viewer_verdict: Some(Verdict::Approve),
        labels: vec!["ui".into()],
        ci: None,
        decision: ReviewDecision::Approved,
        blocked: true,
        conversation: Vec::new(),
        threads: Vec::new(),
        complete: true,
    }
}

fn head(title: &str) -> PullRequestHead {
    PullRequestHead {
        title: title.into(),
        draft: true,
        fork: true,
        target_branch: "main".into(),
    }
}

// The sources: a changeset is named by a repository and its source.

fn tree_of_repo() -> Source {
    Source::WorkingTree
}

fn branch_of(name: &str) -> Source {
    Source::Branch { name: name.to_owned() }
}

fn pull(number: u64) -> Source {
    Source::PullRequest {
        remote: RemoteKey {
            host: "host.example".into(),
            path: "maker/base".into(),
        },
        number,
    }
}

fn row_of(host: &str, number: u64) -> (RemoteKey, u64) {
    (
        RemoteKey {
            host: host.into(),
            path: "contract/repo".into(),
        },
        number,
    )
}

fn held(content: &[u8]) -> StoredBlob {
    StoredBlob::Bytes(content.to_vec())
}

// The cases.

/// `Store`: blobs keep their identity, what the store lacks reads as absent, and a snapshot
/// keeps what it was put with; its stored id must be the one its entries build.
async fn keeps_its_blobs_and_snapshots<S: Store + LiveStore>(store: &S) {
    let ids = store.put_blobs(vec![b"one".to_vec(), b"two".to_vec()]).await.unwrap();
    let mut read = Vec::with_capacity(ids.len());
    for id in &ids {
        read.push(store.blob(*id).await.unwrap());
    }
    assert_eq!(read, [Some(held(b"one")), Some(held(b"two"))]);
    assert_eq!(store.missing_blobs(ids.clone()).await.unwrap(), []);
    let again = store
        .put_blobs(vec![b"two".to_vec(), b"one".to_vec(), b"three".to_vec()])
        .await
        .unwrap();
    assert_eq!(
        (again[0], again[1]),
        (ids[1], ids[0]),
        "blobs the store holds are not written again and answer the same ids"
    );
    assert_eq!(store.blob(again[2]).await.unwrap(), Some(held(b"three")));
    assert_eq!(store.missing_blobs(ids.clone()).await.unwrap(), []);

    assert_eq!(
        store.blobs(vec![ids[0], BlobId::of(b"gone")]).await.unwrap(),
        BTreeMap::from([(ids[0], held(b"one"))]),
        "the reads of a batch answer only what is held"
    );
    assert_eq!(store.blob(BlobId::of(b"gone")).await.unwrap(), None);

    // Text and images keep their bytes; any other binary file is held by hash and size.
    let archive = b"!<arch>\n\0\0\0".to_vec();
    let image = b"\x89PNG\r\n\x1a\n\0\0".to_vec();
    let text = b"fn main() {}\n".to_vec();
    let ids = store
        .put_blobs(vec![archive.clone(), image.clone(), text.clone()])
        .await
        .unwrap();
    assert_eq!(ids, [&archive, &image, &text].map(|b| BlobId::of(b)));
    assert_eq!(store.missing_blobs(ids.clone()).await.unwrap(), []);
    assert_eq!(
        store.blob(ids[0]).await.unwrap(),
        Some(StoredBlob::Binary {
            size: archive.len() as u64
        })
    );
    assert_eq!(store.blob(ids[1]).await.unwrap(), Some(StoredBlob::Bytes(image)));
    assert_eq!(store.blob(ids[2]).await.unwrap(), Some(StoredBlob::Bytes(text)));
    assert_eq!(store.snapshot(tree(255)).await.unwrap(), None);

    let kept = snapshot(&[("a.txt", 21)]);
    store
        .writes()
        .put_snapshot(kept.clone(), SnapshotStatus::default(), T0)
        .await
        .unwrap();
    assert_eq!(
        store.snapshot(kept.id).await.unwrap(),
        Some((kept, SnapshotStatus::default()))
    );

    let mistook = mismarked_snapshot();
    assert!(
        store
            .writes()
            .put_snapshot(mistook, SnapshotStatus::default(), T0)
            .await
            .is_err(),
        "its id is not what its entries build"
    );
}

/// Writes about a changeset store the changeset itself, so that what was last written is read
/// back by the repository and source it belongs to, and nothing of another one reads as ours.
async fn finds_changesets_by_their_repository_and_source<S: LiveStore>(store: &S) {
    let writes = store.writes();
    writes
        .set_lens(REPO.into(), tree_of_repo(), Lens::PerCommit)
        .await
        .unwrap();
    assert_ne!(store.changeset(REPO.into(), tree_of_repo()).await.unwrap(), None);
    assert_eq!(
        store.lens(REPO.into(), tree_of_repo()).await.unwrap(),
        Some(Lens::PerCommit)
    );
    assert_eq!(store.lens(REPO.into(), branch_of("feature")).await.unwrap(), None);
    assert_eq!(store.lens(OTHER_REPO.into(), tree_of_repo()).await.unwrap(), None);

    writes
        .set_pull_request_head(REPO.into(), branch_of("pr"), head("the change"))
        .await
        .unwrap();
    assert_ne!(store.changeset(REPO.into(), branch_of("pr")).await.unwrap(), None);
    assert_eq!(
        store.pull_request_head(REPO.into(), branch_of("pr")).await.unwrap(),
        Some(head("the change"))
    );
    assert_eq!(
        store.pull_request_head(REPO.into(), branch_of("other")).await.unwrap(),
        None
    );
    assert_eq!(
        store
            .pull_request_head(OTHER_REPO.into(), branch_of("pr"))
            .await
            .unwrap(),
        None,
        "another repository is not this pull request's place"
    );

    writes
        .set_pull_request_trusted_head(REPO.into(), branch_of("pr"), Some(commit("trusted")))
        .await
        .unwrap();
    assert_eq!(
        store.pull_request_trust(REPO.into(), branch_of("pr")).await.unwrap(),
        Some(PullRequestTrust {
            fork: true,
            trusted_head: Some(commit("trusted")),
        })
    );
    writes
        .set_pull_request_trusted_head(REPO.into(), branch_of("pr"), None)
        .await
        .unwrap();
    assert_eq!(
        store.pull_request_trust(REPO.into(), branch_of("pr")).await.unwrap(),
        Some(PullRequestTrust {
            fork: true,
            trusted_head: None,
        })
    );

    writes
        .set_pinned_base(REPO.into(), branch_of("pinned"), Some(commit("pinned-base")))
        .await
        .unwrap();
    assert_ne!(store.changeset(REPO.into(), branch_of("pinned")).await.unwrap(), None);
    assert_eq!(
        store.pinned_base(REPO.into(), branch_of("pinned")).await.unwrap(),
        Some(commit("pinned-base"))
    );
    assert_eq!(
        store.pinned_base(REPO.into(), branch_of("feature")).await.unwrap(),
        None
    );
    writes
        .set_pinned_base(REPO.into(), branch_of("pinned"), None)
        .await
        .unwrap();
    assert_eq!(store.pinned_base(REPO.into(), branch_of("pinned")).await.unwrap(), None);

    let vote = PendingVote {
        verdict: Verdict::Approve,
        outgoing: Outgoing::Posting { since: T0 },
    };
    writes
        .set_pending_vote(REPO.into(), pull(9), Some(vote.clone()))
        .await
        .unwrap();
    assert_ne!(store.changeset(REPO.into(), pull(9)).await.unwrap(), None);
    assert_eq!(store.pending_vote(REPO.into(), pull(9)).await.unwrap(), Some(vote));
    assert_eq!(store.pending_vote(REPO.into(), pull(8)).await.unwrap(), None);
    writes.set_pending_vote(REPO.into(), pull(9), None).await.unwrap();
    assert_eq!(store.pending_vote(REPO.into(), pull(9)).await.unwrap(), None);
}

/// A review starts with the versions it names, which replace the changeset's; the versions are
/// read by their numbers, and a version whose content could not be had is listed without it.
async fn replaces_the_versions_at_a_new_review<S: LiveStore>(store: &S) {
    let writes = store.writes();
    let (cid, rid) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            BaseChoice::Commit { id: commit("base0") },
            vec![version_of(1), version_without_snapshots(2)],
            T0,
        )
        .await
        .unwrap();
    assert_eq!(store.open_review(cid).await.unwrap(), Some(rid));
    let listed = store
        .versions(cid)
        .await
        .unwrap()
        .iter()
        .map(|v| (v.n, v.snapshots))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        vec![
            (
                1,
                Some(Snapshots {
                    base: tree(1),
                    target: tree(11)
                })
            ),
            (2, None),
        ],
        "the versions in order of their numbers, the unavailable one without content"
    );
    writes.append_version(cid, version_of(3)).await.unwrap();
    assert_eq!(store.versions(cid).await.unwrap().len(), 3);

    writes.close_review(rid, T0 + 10, false).await.unwrap();
    assert_eq!(store.open_review(cid).await.unwrap(), None);
    assert_ne!(
        store.latest_review(cid).await.unwrap(),
        None,
        "the newest review is still read once the open one is closed"
    );

    let (again, _rid) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_of(1), version_of(2)],
            T0 + 20,
        )
        .await
        .unwrap();
    assert_eq!(again, cid);
    let listed = store
        .versions(cid)
        .await
        .unwrap()
        .iter()
        .map(|v| (v.n, v.at))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        vec![(1, T0 + 10), (2, T0 + 20)],
        "a new review starts over instead of keeping the last versions"
    );
    writes.append_version(cid, version_of(3)).await.unwrap();
    assert_eq!(store.versions(cid).await.unwrap().len(), 3);
}

/// The review of a changeset carries the marking of exactly one review, reads back with what was
/// set on it, and the listing row of each source's newest review sums its review up.
async fn a_review_owns_its_marks_and_reads_end_to_end<S: LiveStore>(store: &S) {
    let writes = store.writes();
    let (cid, rid) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_of(1)],
            T0,
        )
        .await
        .unwrap();
    let added = vec![
        (seen_both(), SpanOrigin::Squash),
        (seen_against_the_old_base(), SpanOrigin::Commit(commit("c1"))),
        (flagged(), SpanOrigin::Squash),
    ];
    let ids = writes.edit_spans(rid, vec![], added.clone()).await.unwrap();
    assert_eq!(ids.len(), 3);
    writes.set_last_reviewed(rid, 1).await.unwrap();
    writes.set_review_summary(rid, summary()).await.unwrap();
    writes.acknowledge(rid, mode_change()).await.unwrap();
    writes.acknowledge(rid, mode_change()).await.unwrap();
    writes.acknowledge(rid, binary_change()).await.unwrap();

    let loaded = store.review(rid).await.unwrap().expect("the open review reads");
    assert_eq!(loaded.id, rid);
    assert_eq!(loaded.changeset, cid);
    assert_eq!(
        loaded.base,
        BaseChoice::MergeBase {
            reference: "main".into()
        }
    );
    assert_eq!(loaded.spans.iter().map(|r| r.id).collect::<Vec<_>>(), ids);
    assert_ne!(loaded.spans[0].id, loaded.spans[2].id);
    assert_eq!(
        loaded.spans.iter().map(|r| r.span.clone()).collect::<Vec<_>>(),
        added.iter().map(|(span, _)| span.clone()).collect::<Vec<_>>(),
        "the spans as they were added"
    );
    assert_eq!(
        loaded.spans.iter().map(|r| r.origin.clone()).collect::<Vec<_>>(),
        [SpanOrigin::Squash, SpanOrigin::Commit(commit("c1")), SpanOrigin::Squash]
    );
    assert_eq!(
        loaded.acks,
        vec![mode_change(), binary_change()],
        "an equal acknowledgement is not stored again"
    );
    assert_eq!(loaded.last_reviewed, Some(1));
    assert_eq!(loaded.summary, Some(summary()));
    assert_eq!(loaded.opened_at, T0);
    assert_eq!(loaded.closed_at, None);
    assert_eq!(loaded.complete, None);

    // A second review of another source holds none of the first review's marks.
    let (next_cid, next_rid) = writes
        .start_review(
            REPO.into(),
            branch_of("other"),
            BaseChoice::MergeBase {
                reference: "other".into(),
            },
            vec![version_of(1)],
            T0,
        )
        .await
        .unwrap();
    let other = store.review(next_rid).await.unwrap().unwrap();
    assert_ne!(other.id, rid);
    assert_eq!(other.changeset, next_cid);
    assert_ne!(next_cid, cid, "another source is not the first changeset");
    assert_ne!(
        other.base,
        BaseChoice::MergeBase {
            reference: "main".into()
        }
    );
    assert_eq!(
        (other.spans.is_empty(), other.acks.is_empty()),
        (true, true),
        "another review starts without the marks of the one before"
    );
    assert_eq!(other.last_reviewed, None, "it has marked nothing yet");
    assert_eq!(other.summary, None, "it has not been measured");
    assert_eq!(other.opened_at, T0);

    writes.close_review(rid, T0 + 60, true).await.unwrap();
    let ended = store.review(rid).await.unwrap().unwrap();
    assert_eq!(ended.closed_at, Some(T0 + 60));
    assert_eq!(ended.complete, Some(true));
    assert_ne!(ended.spans.len(), 0, "the marks are read to the end of a review");
    assert_eq!(ended.last_reviewed, Some(1));
    assert_eq!(ended.summary, Some(summary()));

    assert_eq!(
        store.open_review(cid).await.unwrap(),
        None,
        "of the first changeset no review is open"
    );
    writes.close_review(next_rid, T0 + 70, false).await.unwrap();
    let elsewhere = store.review(next_rid).await.unwrap().unwrap();
    assert_eq!(elsewhere.closed_at, Some(T0 + 70));
    assert_eq!(elsewhere.complete, Some(false));
    assert_eq!(store.open_changesets().await.unwrap(), [], "no review is open any more");
    let rows = store
        .source_reviews(REPO.into(), &[tree_of_repo(), branch_of("other")])
        .await
        .unwrap();
    let listed = rows
        .iter()
        .map(|r| {
            (
                r.source.clone(),
                r.changeset,
                r.open_threads,
                r.latest.map(ending_of),
                r.latest_version,
            )
        })
        .collect::<Vec<_>>();
    let (source_of, on_of, threads_of, ending_of_one, version_of_one) = &listed[0];
    assert_eq!(source_of, &tree_of_repo(), "the sources in the order asked for");
    assert_eq!(on_of, &Some(cid));
    assert_eq!(threads_of, &0);
    assert_eq!(ending_of_one, &Some((Some(T0 + 60), true, Some(1), Some(summary()))));
    assert_eq!(version_of_one, &1);
    let (_, on_of_2, _, ending_of_2, version_of_2) = &listed[1];
    assert_eq!(on_of_2, &Some(next_cid));
    assert_eq!(
        ending_of_2,
        &Some((Some(T0 + 70), false, None, None)),
        "the newest review of each source"
    );
    assert_eq!(version_of_2, &1);
}

/// How a listing row spells its newest review.
fn ending_of(row: ReviewRow) -> (Option<i64>, bool, Option<u32>, Option<ReviewSummary>) {
    (row.closed_at, row.complete, row.last_reviewed, row.summary)
}

/// Threads belong to the changeset they are opened on; a thread keeps its messages' updates, its
/// anchor and its platform's state, and its messages lie in the platform's text first, the
/// viewer's after it, a thread outside a platform in the order it was written in. A deleted
/// thread's messages go with it.
async fn threads_have_their_messages_and_belong_to_their_changeset<S: LiveStore>(store: &S) {
    let writes = store.writes();
    let (cid, rid) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_of(1)],
            T0,
        )
        .await
        .unwrap();
    let platform = PlatformThread {
        remote_id: Some("T1".into()),
        resolvable: true,
        resolved_by: Some("lena".into()),
        status_pending: false,
    };
    let (range, _) = writes
        .add_thread(
            cid,
            Some(rid),
            CommentTarget::Range(anchor(Side::Target, 2)),
            posted("C1", "why?"),
            Some(platform.clone()),
            ThreadStatus::Open,
        )
        .await
        .unwrap();
    let (file, _) = writes
        .add_thread(
            cid,
            Some(rid),
            CommentTarget::File {
                side: Side::Base,
                path: RepoPath::from("a/b"),
            },
            message("file body"),
            None,
            ThreadStatus::Open,
        )
        .await
        .unwrap();
    let (whole, _) = writes
        .add_thread(
            cid,
            None,
            CommentTarget::Changeset,
            message("over everything"),
            None,
            ThreadStatus::Open,
        )
        .await
        .unwrap();
    writes.add_message(range, posted("C2", "no")).await.unwrap();
    writes
        .add_message(
            range,
            Message {
                at: T0 - 100,
                ..message("mine, although an earlier at than the platform's")
            },
        )
        .await
        .unwrap();
    writes
        .add_message(
            file,
            Message {
                at: T0 - 5,
                ..message("written later, though a smaller at")
            },
        )
        .await
        .unwrap();

    let read = store.threads(cid).await.unwrap();
    assert_eq!(
        read.iter().map(|t| t.thread.id).collect::<Vec<_>>(),
        [range, file, whole],
        "the threads in the order they were opened in"
    );
    assert_eq!(
        read.iter().map(|t| t.review).collect::<Vec<_>>(),
        [Some(rid), Some(rid), None],
        "a thread keeps the review it came from"
    );
    assert_eq!(read[0].thread.platform, Some(platform));
    assert_eq!(read[1].thread.platform, None);
    assert_eq!(read[2].thread.platform, None);
    assert_eq!(
        read[0]
            .thread
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        ["why?", "no", "mine, although an earlier at than the platform's"],
        "the platform's text first (an earlier `at` of the drafts keeps it), then the drafts by their record"
    );
    assert_eq!(
        read[1].thread.messages.iter().map(|m| m.at).collect::<Vec<_>>(),
        [T0, T0 - 5],
        "outside a platform, the messages are in the order they were written in"
    );
    let opener = read[0].thread.messages[0].id;
    let mine = read[0]
        .thread
        .messages
        .iter()
        .find(|m| m.mine)
        .expect("the viewer's draft on the thread")
        .id;

    writes
        .replace_message(Message {
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
            ..posted("C1", "why?")
        })
        .await
        .unwrap();
    writes
        .replace_message(Message {
            id: mine,
            outgoing: Outgoing::Posting { since: T0 + 20 },
            ..message("mine, although an earlier at than the platform's")
        })
        .await
        .unwrap();
    writes.set_thread_status(range, ThreadStatus::Resolved).await.unwrap();
    writes
        .set_thread_platform(
            file,
            PlatformThread {
                remote_id: None,
                resolvable: true,
                resolved_by: None,
                status_pending: true,
            },
        )
        .await
        .unwrap();
    let moved = CommentTarget::Range(Anchor {
        range: LineRange { start: 20, end: 25 },
        ..anchor(Side::Target, 2)
    });
    writes.set_thread_target(range, moved.clone()).await.unwrap();

    let read = store.threads(cid).await.unwrap();
    assert_eq!(read[0].thread.status, ThreadStatus::Resolved);
    assert_eq!(read[1].thread.status, ThreadStatus::Open);
    assert_eq!(
        read[0].thread.target, moved,
        "the thread was re-anchored to what was set"
    );
    assert_eq!(
        read[1].thread.platform,
        Some(PlatformThread {
            remote_id: None,
            resolvable: true,
            resolved_by: None,
            status_pending: true,
        }),
        "the platform's state as it was replaced"
    );
    assert_eq!(read[2].thread.platform, None);
    let updated = read[0].thread.messages[0].clone();
    assert_eq!(updated.id, opener, "the message keeps its place in the thread");
    assert_eq!(updated.body, "why not?");
    assert_eq!(
        updated.remote,
        Some(RemoteComment {
            id: "C1".into(),
            synced: "why?".into(),
            conflict: Some(EditConflict {
                remote_body: "why so?".into(),
                remote_at: T0 + 9,
            }),
        })
    );
    let of_mine = read[0].thread.messages.iter().find(|m| m.id == mine).unwrap();
    assert_eq!(
        of_mine.outgoing,
        Outgoing::Posting { since: T0 + 20 },
        "its way to the platform, kept"
    );
    assert_eq!(
        read[2]
            .thread
            .messages
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        ["over everything"]
    );

    // The threads of another changeset are none of these; deleting a thread deletes its
    // messages.
    let elsewhere = writes
        .ensure_changeset(REPO.into(), branch_of("somewhere-else"))
        .await
        .unwrap();
    assert_eq!(
        store.threads(elsewhere).await.unwrap(),
        [],
        "the later changeset holds none of these"
    );
    writes.delete_thread(range).await.unwrap();
    let read = store.threads(cid).await.unwrap();
    assert_eq!(read.iter().map(|t| t.thread.id).collect::<Vec<_>>(), [file, whole]);
    let left = read
        .iter()
        .flat_map(|t| t.thread.messages.iter().map(|m| m.body.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        left,
        [
            "written later, though a smaller at".to_string(),
            "file body".to_string(),
            "over everything".to_string()
        ],
        "the deleted thread's messages went with it; the file thread's own are in the store's order \
         (a platform thread now, what the viewer wrote first reads first)"
    );
}

/// A pull request's details are kept as one copy per request: a newer copy replaces it, an older
/// one replaces nothing newer, and over the kept bound the ones read longest ago go.
async fn keeps_a_newest_copy_of_pull_request_details_up_to_the_bound<S: LiveStore>(store: &S) {
    let writes = store.writes();
    let (remote, number) = row_of("revisited.example", 1);
    assert_eq!(
        store.stored_discussion(remote.clone(), number).await.unwrap(),
        None,
        "nothing was stored of the pull request yet"
    );
    writes
        .set_discussion(
            remote.clone(),
            number,
            PLATFORM_HEAD.into(),
            T0 + 30,
            T0 - 1000,
            &discussion("first"),
        )
        .await
        .unwrap();
    let kept = store.stored_discussion(remote.clone(), number).await.unwrap().unwrap();
    assert_eq!(
        (kept.updated_at, kept.synced_at, kept.discussion,),
        (T0 + 30, T0 - 1000, discussion("first"))
    );

    // A copy newer than what is kept replaces it; an older one replaces nothing newer.
    writes
        .set_discussion(
            remote.clone(),
            number,
            PLATFORM_HEAD.into(),
            T0 + 20,
            T0 + 50,
            &discussion("older"),
        )
        .await
        .unwrap();
    let kept = store.stored_discussion(remote.clone(), number).await.unwrap().unwrap();
    assert_eq!(kept.updated_at, T0 + 30, "the older copy wrote nothing");
    assert_eq!(kept.synced_at, T0 - 1000);
    assert_eq!(kept.discussion, discussion("first"));
    writes
        .set_discussion(
            remote.clone(),
            number,
            PLATFORM_HEAD.into(),
            T0 + 80,
            T0 + 40,
            &discussion("newer"),
        )
        .await
        .unwrap();
    let kept = store.stored_discussion(remote.clone(), number).await.unwrap().unwrap();
    assert_eq!(
        (kept.updated_at, kept.synced_at, kept.discussion),
        (T0 + 80, T0 + 40, discussion("newer"))
    );

    // Over the kept bound, the copies read longest ago go.
    for n in 1..=KEPT_PULL_REQUESTS as u64 + 1 {
        let (host, number) = row_of("bound.invalid", n);
        writes
            .set_discussion(
                host,
                number,
                PLATFORM_HEAD.into(),
                T0 + 100_000 + i64::from(n as u32),
                T0 + 100_000 + i64::from(n as u32),
                &discussion(&format!("pr{n}")),
            )
            .await
            .unwrap();
    }
    let record_of = |host: String, number: u64| async move {
        let (remote, number) = row_of(&host, number);
        store
            .stored_discussion(remote, number)
            .await
            .unwrap()
            .map(|kept| kept.updated_at)
    };
    assert_eq!(
        (
            record_of("revisited.example".into(), 1).await,
            record_of("bound.invalid".into(), 1).await
        ),
        (None, None),
        "the two longest-ago reads were evicted"
    );
    assert_eq!(
        record_of("bound.invalid".into(), 2).await,
        Some(T0 + 100_002),
        "the newest 256 of 258 are kept"
    );
    assert_eq!(record_of("bound.invalid".into(), 257).await, Some(T0 + 100_257));
}

/// A repository's filter and trust are its own; every other repository reads its default.
async fn holds_a_repositorys_settings_apart<S: LiveStore>(store: &S) {
    let writes = store.writes();
    assert_eq!(store.filter(REPO.into()).await.unwrap(), ReviewFilter::default());
    assert!(!store.project_trusted(REPO.into()).await.unwrap());
    let kept = filter();
    writes.set_filter(REPO.into(), kept.clone()).await.unwrap();
    assert_eq!(
        store.filter(REPO.into()).await.unwrap(),
        kept,
        "the filter as it was set"
    );
    assert_eq!(
        store.filter(OTHER_REPO.into()).await.unwrap(),
        ReviewFilter::default(),
        "another repository reads its own, not the one set here"
    );
    writes.set_project_trusted(REPO.into(), true).await.unwrap();
    assert!(store.project_trusted(REPO.into()).await.unwrap());
    assert!(!store.project_trusted(OTHER_REPO.into()).await.unwrap());

    let other = other_filter();
    writes.set_filter(OTHER_REPO.into(), other.clone()).await.unwrap();
    assert_eq!(store.filter(OTHER_REPO.into()).await.unwrap(), other);
    assert_eq!(
        store.filter(REPO.into()).await.unwrap(),
        kept,
        "a filter set elsewhere does not replace this one"
    );
    writes.set_filter(REPO.into(), ReviewFilter::default()).await.unwrap();
    assert_eq!(
        store.filter(REPO.into()).await.unwrap(),
        ReviewFilter::default(),
        "a whole replacement, without what the filter had before"
    );
    writes.set_project_trusted(REPO.into(), false).await.unwrap();
    assert!(
        !store.project_trusted(REPO.into()).await.unwrap(),
        "the trust `false` clears"
    );
    assert!(!store.project_trusted(OTHER_REPO.into()).await.unwrap());
}

/// What the store was never told of reads as empty, even with a store that has things.
async fn what_the_store_was_never_told_of_reads_empty<S: LiveStore>(store: &S) {
    let writes = store.writes();
    let (cid, rid) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_of(1)],
            T0,
        )
        .await
        .unwrap();
    assert!(
        store.review(rid).await.unwrap().is_some(),
        "what was written is read back"
    );
    let other_changeset = ChangesetId(cid.0 + 1_000_000);
    assert_eq!(
        store
            .changeset(OTHER_REPO.into(), branch_of("never-stored"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(store.open_review(other_changeset).await.unwrap(), None);
    assert_eq!(store.latest_review(other_changeset).await.unwrap(), None);
    assert_eq!(store.versions(other_changeset).await.unwrap(), [], "no versions");
    assert_eq!(store.threads(other_changeset).await.unwrap(), [], "no threads");
    assert_eq!(
        store.review(ReviewId(rid.0 + 1_000_000)).await.unwrap().map(|r| r.id),
        None,
        "no review the store never started"
    );
    let rows = store
        .source_reviews(REPO.into(), &[tree_of_repo(), branch_of("never-stored")])
        .await
        .unwrap();
    let listed = rows
        .iter()
        .map(|r| (r.source.clone(), r.changeset, r.open_threads, r.latest_version))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        [
            (tree_of_repo(), Some(cid), 0, 1),
            (branch_of("never-stored"), None, 0, 0),
        ],
        "each source as its own, the one answered the way one written is, the other empty"
    );
    assert_eq!(store.open_changesets().await.unwrap(), [cid]);
}

/// `PreferenceStore`: a scope's values are its own, a key's write replaces it, `None` forgets it,
/// and a scope's values are read by key.
async fn keeps_preferences_per_scope_by_key<S: LiveStore>(store: &S) {
    assert_eq!(store.preferences(None).await.unwrap(), [], "nothing kept reads empty");
    assert_eq!(store.preferences(Some(REPO.into())).await.unwrap(), []);

    // Written out of order, so that a store reading them by key is told apart from one reading
    // them in the order they were written in.
    for (key, value) in [
        ("theme", "dark"),
        ("wrap", "true"),
        ("font", "mono"),
        ("zoom", "1.0"),
        ("sidebar", "wide"),
        ("tabs", "2"),
    ] {
        store
            .set_preference(None, key.into(), Some(value.into()))
            .await
            .unwrap();
    }
    store
        .set_preference(Some(REPO.into()), "theme".into(), Some("project".into()))
        .await
        .unwrap();

    let pairs = |list: Vec<(String, String)>| list.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>();
    assert_eq!(
        pairs(store.preferences(None).await.unwrap()),
        [
            "font=mono",
            "sidebar=wide",
            "tabs=2",
            "theme=dark",
            "wrap=true",
            "zoom=1.0"
        ],
        "the app scope's values, by key"
    );
    assert_eq!(
        pairs(store.preferences(Some(REPO.into())).await.unwrap()),
        ["theme=project"],
        "a project's values are its own, not the app's"
    );
    assert_eq!(
        store.preferences(Some(OTHER_REPO.into())).await.unwrap(),
        [],
        "a scope never written reads empty"
    );

    store
        .set_preference(None, "theme".into(), Some("light".into()))
        .await
        .unwrap();
    assert_eq!(
        pairs(store.preferences(None).await.unwrap()),
        [
            "font=mono",
            "sidebar=wide",
            "tabs=2",
            "theme=light",
            "wrap=true",
            "zoom=1.0"
        ],
        "a key's value is replaced"
    );
    store.set_preference(None, "tabs".into(), None).await.unwrap();
    store.set_preference(None, "never kept".into(), None).await.unwrap();
    assert_eq!(
        pairs(store.preferences(None).await.unwrap()),
        ["font=mono", "sidebar=wide", "theme=light", "wrap=true", "zoom=1.0"],
        "`None` forgets only that scope's key"
    );
    assert_eq!(
        pairs(store.preferences(Some(REPO.into())).await.unwrap()),
        ["theme=project"],
        "another scope's value of the same key stays"
    );
}

/// `SettingsStore`: the defaults before a write, what was set read back, and a later write
/// replacing it. No fake implements this port, so it runs against the real store alone.
pub async fn settings_contract<S: SettingsStore>(mut open: impl AsyncFnMut() -> S) {
    let store = open().await;
    assert_eq!(
        store.backend_settings().await.unwrap(),
        Settings::default(),
        "a setting that was never kept has its default"
    );
    let one = Settings {
        semantic_cache_mb: 64,
        finished_review_days: 7,
    };
    store.set_backend_settings(one).await.unwrap();
    assert_eq!(
        store.backend_settings().await.unwrap(),
        one,
        "the settings as they were set"
    );

    let two = Settings {
        semantic_cache_mb: 1024,
        finished_review_days: 1,
    };
    store.set_backend_settings(two).await.unwrap();
    assert_eq!(
        store.backend_settings().await.unwrap(),
        two,
        "a later write replaces the settings of the earlier one"
    );
}

/// `GcStore`: the facts the store hands `decide`, the rows the plan deletes, and the tree nodes
/// and content no kept snapshot reaches. No fake implements this port, so it runs against the
/// real store alone.
pub async fn gc_contract<S: LiveStore + GcStore>(mut open: impl AsyncFnMut() -> S) {
    {
        let store = open().await;
        reads_the_facts_of_what_it_holds(&store).await;
    }
    {
        let store = open().await;
        deletes_the_planned_rows_and_sweeps_what_none_reaches(&store).await;
    }
}

/// A version whose base and target are the same snapshot, which is what GC refers to.
fn version_at(snapshot: SnapshotId, n: u32) -> Version {
    Version {
        snapshots: Some(Snapshots {
            base: snapshot,
            target: snapshot,
        }),
        ..version_of(n)
    }
}

fn waiting_vote() -> PendingVote {
    PendingVote {
        verdict: Verdict::Approve,
        outgoing: Outgoing::Posting { since: T0 },
    }
}

/// The facts the store hands `decide`, recorded with a plan that deletes nothing.
async fn recorded_facts<S: GcStore>(store: &S, now: i64) -> GcFacts {
    let seen = OnceLock::new();
    store
        .gc(now, &|facts| {
            let _ = seen.set(facts.clone());
            GcPlan::default()
        })
        .await
        .unwrap();
    seen.into_inner().expect("gc handed the facts to decide")
}

/// `GcStore`: the facts say of each changeset what refers to it, and of each snapshot when it was
/// stored.
async fn reads_the_facts_of_what_it_holds<S: LiveStore + GcStore>(store: &S) {
    let writes = store.writes();
    let kept = snapshot(&[("kept.txt", 31)]);
    let dead = snapshot(&[("dead.txt", 32)]);
    let anchored = snapshot(&[("anchored.txt", 33)]);
    for (s, at) in [(&kept, T0), (&dead, T0 + 1), (&anchored, T0 + 2)] {
        writes
            .put_snapshot(s.clone(), SnapshotStatus::default(), at)
            .await
            .unwrap();
    }
    let (kept_cid, kept_rid) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_at(kept.id, 1)],
            T0,
        )
        .await
        .unwrap();
    writes
        .set_pending_vote(REPO.into(), tree_of_repo(), Some(waiting_vote()))
        .await
        .unwrap();
    writes
        .add_thread(
            kept_cid,
            Some(kept_rid),
            CommentTarget::Range(Anchor {
                snapshot: anchored.id,
                ..anchor(Side::Target, 0)
            }),
            message("open"),
            None,
            ThreadStatus::Open,
        )
        .await
        .unwrap();
    let (dead_cid, dead_rid) = writes
        .start_review(
            REPO.into(),
            branch_of("dead"),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_at(dead.id, 1)],
            T0 + 10,
        )
        .await
        .unwrap();
    writes.close_review(dead_rid, T0 + 20, false).await.unwrap();

    let facts = recorded_facts(store, T0).await;

    let mut changesets = facts.changesets;
    changesets.sort_by_key(|c| c.id.0);
    assert_eq!(changesets.len(), 2, "one entry per stored changeset");
    let kept_facts = &changesets[0];
    assert_eq!(kept_facts.id, kept_cid);
    assert!(kept_facts.has_open_review, "its review is open");
    assert_eq!(kept_facts.last_closed_at, None, "no review of it closed");
    assert!(kept_facts.has_open_thread, "its thread is open");
    assert!(kept_facts.has_pending_vote, "its vote waits to be sent");
    let mut named = kept_facts.snapshots.clone();
    named.sort();
    named.dedup();
    let mut expected = vec![kept.id, anchored.id];
    expected.sort();
    assert_eq!(
        named, expected,
        "the snapshots its versions and its thread's anchor name"
    );

    let dead_facts = &changesets[1];
    assert_eq!(dead_facts.id, dead_cid);
    assert!(!dead_facts.has_open_review, "its review was closed");
    assert_eq!(dead_facts.last_closed_at, Some(T0 + 20));
    assert!(!dead_facts.has_open_thread);
    assert!(!dead_facts.has_pending_vote);
    let mut dead_named = dead_facts.snapshots.clone();
    dead_named.sort();
    dead_named.dedup();
    assert_eq!(dead_named, vec![dead.id]);

    let mut stored = facts.snapshots;
    stored.sort_by_key(|(id, _)| *id);
    let mut expected = vec![(kept.id, T0), (dead.id, T0 + 1), (anchored.id, T0 + 2)];
    expected.sort_by_key(|(id, _)| *id);
    assert_eq!(stored, expected, "every stored snapshot with when it was stored");
}

/// `GcStore`: the store deletes the changesets and snapshots the plan names, with everything that
/// hung off them, and sweeps the tree nodes and content no kept snapshot reaches.
async fn deletes_the_planned_rows_and_sweeps_what_none_reaches<S: LiveStore + GcStore>(store: &S) {
    let writes = store.writes();
    let kept = snapshot(&[("kept.txt", 31)]);
    let dead = snapshot(&[("dead.txt", 32)]);
    let orphan = b"orphan".to_vec();
    store.put_blobs(vec![vec![31], vec![32], orphan.clone()]).await.unwrap();
    writes
        .put_snapshot(kept.clone(), SnapshotStatus::default(), T0)
        .await
        .unwrap();
    writes
        .put_snapshot(dead.clone(), SnapshotStatus::default(), T0 + 1)
        .await
        .unwrap();
    let (kept_cid, _) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_at(kept.id, 1)],
            T0,
        )
        .await
        .unwrap();
    let (dead_cid, dead_rid) = writes
        .start_review(
            REPO.into(),
            branch_of("dead"),
            BaseChoice::MergeBase {
                reference: "main".into(),
            },
            vec![version_at(dead.id, 1)],
            T0 + 10,
        )
        .await
        .unwrap();
    writes.close_review(dead_rid, T0 + 20, false).await.unwrap();

    let plan = GcPlan {
        changesets: vec![dead_cid],
        snapshots: vec![dead.id],
        content_grace: Duration::ZERO,
    };
    let report = store.gc(T0, &|_| plan.clone()).await.unwrap();
    assert_eq!(
        (
            report.changesets_deleted,
            report.snapshots_deleted,
            report.nodes_deleted,
            report.blobs_deleted
        ),
        (1, 1, 1, 2),
        "the planned changeset and snapshot, its tree node, and the content no kept tree names"
    );

    assert_eq!(store.changeset(REPO.into(), branch_of("dead")).await.unwrap(), None);
    assert_eq!(store.versions(dead_cid).await.unwrap(), []);
    assert!(
        store.review(dead_rid).await.unwrap().is_none(),
        "the closed review is gone"
    );
    assert_eq!(store.threads(dead_cid).await.unwrap(), []);
    assert_eq!(store.snapshot(dead.id).await.unwrap(), None);
    assert_eq!(store.blob(BlobId::of(&[32])).await.unwrap(), None);
    assert_eq!(store.blob(BlobId::of(&orphan)).await.unwrap(), None);

    assert_eq!(
        store.changeset(REPO.into(), tree_of_repo()).await.unwrap(),
        Some(kept_cid),
        "the changeset the plan did not name stays"
    );
    assert!(store.open_review(kept_cid).await.unwrap().is_some());
    assert_eq!(store.versions(kept_cid).await.unwrap().len(), 1);
    assert_eq!(store.snapshot(kept.id).await.unwrap().map(|(s, _)| s), Some(kept));
    assert_eq!(store.blob(BlobId::of(&[31])).await.unwrap(), Some(held(&[31])));

    let young = b"young".to_vec();
    store.put_blobs(vec![young.clone()]).await.unwrap();
    let keep = GcPlan {
        content_grace: Duration::from_secs(365 * 24 * 3600),
        ..GcPlan::default()
    };
    let report = store.gc(T0, &|_| keep.clone()).await.unwrap();
    assert_eq!(report.blobs_deleted, 0, "content within the plan's grace is kept");
    assert_eq!(store.blob(BlobId::of(&young)).await.unwrap(), Some(held(&young)));
}

/// `ProjectStore`: what is held for a repository root, and what removing it leaves. No fake
/// implements this port, so it runs against the real store alone.
pub async fn project_contract<S: LiveStore + GcStore + ProjectStore + RecentStore>(mut open: impl AsyncFnMut() -> S) {
    {
        let store = open().await;
        removes_every_kind_of_row_of_a_project_and_leaves_the_others(&store).await;
    }
    {
        let store = open().await;
        sweeps_only_the_content_no_remaining_project_reaches(&store).await;
    }
    {
        let store = open().await;
        reads_the_facts_of_a_project(&store).await;
    }
    {
        let store = open().await;
        measures_the_content_only_one_project_reaches(&store).await;
    }
}

fn pull_of(path: &str, number: u64) -> Source {
    Source::PullRequest {
        remote: RemoteKey {
            host: "host.example".into(),
            path: path.into(),
        },
        number,
    }
}

fn main_base() -> BaseChoice {
    BaseChoice::MergeBase {
        reference: "main".into(),
    }
}

/// `repo` holds a review with a thread, a filter, trust, a preference, a place on the recent list
/// and the stored details of the pull requests `maker/base` #7 and `solo/<repo>` #3.
async fn hold_everything_for<S: LiveStore + RecentStore>(store: &S, repo: &str, recent: &[&str]) {
    let writes = store.writes();
    let (changeset, review) = writes
        .start_review(repo.into(), tree_of_repo(), main_base(), vec![version_of(1)], T0)
        .await
        .unwrap();
    writes
        .add_thread(
            changeset,
            Some(review),
            CommentTarget::Changeset,
            message("why?"),
            None,
            ThreadStatus::Open,
        )
        .await
        .unwrap();
    writes.set_filter(repo.into(), filter()).await.unwrap();
    writes.set_project_trusted(repo.into(), true).await.unwrap();
    store
        .set_preference(Some(repo.into()), "tabs".into(), Some("[]".into()))
        .await
        .unwrap();
    let solo = format!("solo{repo}");
    for source in [pull(7), pull_of(&solo, 3)] {
        writes
            .set_pull_request_head(repo.into(), source.clone(), head("a change"))
            .await
            .unwrap();
        let Source::PullRequest { remote, number } = source else {
            unreachable!("a pull request");
        };
        writes
            .set_discussion(remote, number, PLATFORM_HEAD.into(), T0, T0, &discussion("a"))
            .await
            .unwrap();
    }
    store
        .set_recent_projects(
            recent
                .iter()
                .map(|path| RecentProject {
                    path: (*path).into(),
                    opened_at: T0,
                })
                .collect(),
        )
        .await
        .unwrap();
}

/// `ProjectStore::remove_project`: each kind of row held for the repository goes, in one go, and
/// what another repository holds stays; the details of a pull request go only when no changeset of
/// another repository names its remote.
async fn removes_every_kind_of_row_of_a_project_and_leaves_the_others<S: LiveStore + ProjectStore + RecentStore>(
    store: &S,
) {
    hold_everything_for(store, REPO, &[REPO]).await;
    hold_everything_for(store, OTHER_REPO, &[REPO, OTHER_REPO]).await;
    let mine = store.changeset(REPO.into(), tree_of_repo()).await.unwrap().unwrap();
    let theirs = store
        .changeset(OTHER_REPO.into(), tree_of_repo())
        .await
        .unwrap()
        .unwrap();
    let review = store.open_review(mine).await.unwrap().unwrap();
    assert_eq!(
        store.known_projects().await.unwrap(),
        [OTHER_REPO, REPO],
        "both are known, in root order"
    );

    store.remove_project(REPO.into()).await.unwrap();

    assert_eq!(
        store.changeset(REPO.into(), tree_of_repo()).await.unwrap(),
        None,
        "changeset"
    );
    assert_eq!(
        store.changeset(REPO.into(), pull(7)).await.unwrap(),
        None,
        "pull request changeset"
    );
    assert_eq!(store.versions(mine).await.unwrap(), [], "versions");
    assert!(store.review(review).await.unwrap().is_none(), "review");
    assert_eq!(store.threads(mine).await.unwrap(), [], "threads");
    assert_eq!(
        store.filter(REPO.into()).await.unwrap(),
        ReviewFilter::default(),
        "filter"
    );
    assert!(!store.project_trusted(REPO.into()).await.unwrap(), "trust");
    assert_eq!(store.preferences(Some(REPO.into())).await.unwrap(), [], "preferences");
    let recent: Vec<String> = store
        .recent_projects()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.path)
        .collect();
    assert_eq!(recent, [OTHER_REPO], "the recent list");
    let details = |path: &str, number: u64| {
        let remote = RemoteKey {
            host: "host.example".into(),
            path: path.into(),
        };
        async move { store.stored_discussion(remote, number).await.unwrap().is_some() }
    };
    assert!(
        !details(&format!("solo{REPO}"), 3).await,
        "the details of a pull request only this project named"
    );
    assert!(
        details("maker/base", 7).await,
        "the details of a remote another project's changeset names"
    );

    assert!(store.open_review(theirs).await.unwrap().is_some(), "the other's review");
    assert_eq!(store.versions(theirs).await.unwrap().len(), 1, "the other's versions");
    assert_eq!(store.threads(theirs).await.unwrap().len(), 1, "the other's threads");
    assert_eq!(
        store.filter(OTHER_REPO.into()).await.unwrap(),
        filter(),
        "the other's filter"
    );
    assert!(
        store.project_trusted(OTHER_REPO.into()).await.unwrap(),
        "the other's trust"
    );
    assert_eq!(
        store.preferences(Some(OTHER_REPO.into())).await.unwrap().len(),
        1,
        "the other's preferences"
    );
    assert!(
        details(&format!("solo{OTHER_REPO}"), 3).await,
        "the other's pull request"
    );
    assert_eq!(
        store.known_projects().await.unwrap(),
        [OTHER_REPO],
        "a removed project is known no more"
    );
}

/// Garbage collection after a removal: content and snapshots only the removed project held go once
/// they are older than the grace; what another project's changeset names, and what is younger,
/// stays.
async fn sweeps_only_the_content_no_remaining_project_reaches<S: LiveStore + GcStore + ProjectStore>(store: &S) {
    let writes = store.writes();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    // Blobs are stored at the real time; the collection runs two hours later.
    let collected_at = now + 7200;
    let own = snapshot(&[("own.txt", 41)]);
    let shared = snapshot(&[("shared.txt", 42)]);
    let young = snapshot(&[("young.txt", 43)]);
    let others = snapshot(&[("others.txt", 44)]);
    store
        .put_blobs(vec![vec![41], vec![42], vec![43], vec![44]])
        .await
        .unwrap();
    for (s, at) in [(&own, now), (&shared, now), (&young, collected_at - 60), (&others, now)] {
        writes
            .put_snapshot(s.clone(), SnapshotStatus::default(), at)
            .await
            .unwrap();
    }
    let review = |repo: &'static str, source: Source, snapshot: SnapshotId| {
        writes.start_review(repo.into(), source, main_base(), vec![version_at(snapshot, 1)], T0)
    };
    review(REPO, tree_of_repo(), own.id).await.unwrap();
    review(REPO, branch_of("shares"), shared.id).await.unwrap();
    review(REPO, branch_of("young"), young.id).await.unwrap();
    review(OTHER_REPO, tree_of_repo(), shared.id).await.unwrap();
    review(OTHER_REPO, branch_of("own"), others.id).await.unwrap();
    let all = vec![own.id, shared.id, young.id, others.id];
    let facts = store.project_facts(REPO.into()).await.unwrap();
    assert_eq!(
        store.named_snapshots(all.clone()).await.unwrap().len(),
        4,
        "every snapshot is named by a changeset"
    );

    store.remove_project(REPO.into()).await.unwrap();
    let still = store.named_snapshots(facts.snapshots.clone()).await.unwrap();
    assert_eq!(still, [shared.id], "only what the other project names is named still");
    crate::gc::collect(store, collected_at, &crate::gc::Retention::default())
        .await
        .unwrap();

    let held_by_the_store = async |s: &Snapshot, blob: u8| {
        (
            store.snapshot(s.id).await.unwrap().is_some(),
            store.blob(BlobId::of(&[blob])).await.unwrap().is_some(),
        )
    };
    assert_eq!(
        held_by_the_store(&own, 41).await,
        (false, false),
        "what only the removed project held"
    );
    assert_eq!(
        held_by_the_store(&shared, 42).await,
        (true, true),
        "what the other project names too"
    );
    assert_eq!(
        held_by_the_store(&young, 43).await,
        (true, true),
        "what is younger than the grace"
    );
    assert_eq!(
        held_by_the_store(&others, 44).await,
        (true, true),
        "what only the other project holds"
    );
}

/// `ProjectStore::project_facts`: what removing the project loses, counted from the project's own
/// rows.
async fn reads_the_facts_of_a_project<S: LiveStore + ProjectStore>(store: &S) {
    let writes = store.writes();
    let (open, open_review) = writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            main_base(),
            vec![version_at(tree(1), 1), version_at(tree(2), 2)],
            T0,
        )
        .await
        .unwrap();
    let (closed, closed_review) = writes
        .start_review(
            REPO.into(),
            branch_of("done"),
            main_base(),
            vec![version_at(tree(3), 1)],
            T0,
        )
        .await
        .unwrap();
    writes.close_review(closed_review, T0 + 1, true).await.unwrap();
    writes
        .add_thread(
            open,
            Some(open_review),
            CommentTarget::Range(Anchor {
                snapshot: tree(4),
                ..anchor(Side::Target, 0)
            }),
            message("only here"),
            None,
            ThreadStatus::Open,
        )
        .await
        .unwrap();
    let (thread, _) = writes
        .add_thread(
            open,
            None,
            CommentTarget::Changeset,
            posted("C1", "from the platform"),
            Some(PlatformThread {
                remote_id: Some("T1".into()),
                resolvable: true,
                resolved_by: None,
                status_pending: false,
            }),
            ThreadStatus::Open,
        )
        .await
        .unwrap();
    writes.add_message(thread, message("a draft reply")).await.unwrap();
    writes.add_message(thread, posted("C2", "posted too")).await.unwrap();
    writes
        .add_thread(
            closed,
            None,
            CommentTarget::Changeset,
            message("on the finished one"),
            None,
            ThreadStatus::Resolved,
        )
        .await
        .unwrap();
    writes
        .set_pending_vote(REPO.into(), branch_of("done"), Some(waiting_vote()))
        .await
        .unwrap();
    writes.set_project_trusted(REPO.into(), true).await.unwrap();
    writes
        .start_review(
            OTHER_REPO.into(),
            tree_of_repo(),
            main_base(),
            vec![version_at(tree(9), 1)],
            T0,
        )
        .await
        .unwrap();

    let facts = store.project_facts(REPO.into()).await.unwrap();
    assert_eq!(facts.changesets, [open, closed], "the project's own changesets");
    let mut named = facts.snapshots.clone();
    named.sort();
    assert_eq!(
        named,
        [tree(1), tree(2), tree(3), tree(4)],
        "the snapshots of its versions and anchors, and no other project's"
    );
    assert_eq!(facts.open_reviews, 1, "one review is open");
    assert_eq!(
        facts.unposted_comments, 3,
        "the two messages only Vet has on the open changeset and the one on the finished one, not the posted ones"
    );
    assert_eq!(facts.pending_votes, 1);
    assert!(facts.trusted);
    let other = store.project_facts(OTHER_REPO.into()).await.unwrap();
    assert_eq!(
        (
            other.open_reviews,
            other.unposted_comments,
            other.pending_votes,
            other.trusted
        ),
        (1, 0, 0, false)
    );
}

/// A blob that does not compress and that the store keeps the bytes of.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 255) as u8 + 1
        })
        .collect()
}

/// `ProjectStore::review_data`: the stored bytes of the content only one project's snapshots
/// reach; content two projects reach counts for neither.
async fn measures_the_content_only_one_project_reaches<S: LiveStore + ProjectStore>(store: &S) {
    let writes = store.writes();
    let (own, shared, others) = (noise(1000, 1), noise(3000, 2), noise(2000, 3));
    store
        .put_blobs(vec![own.clone(), shared.clone(), others.clone()])
        .await
        .unwrap();
    let tree_of = |files: &[&[u8]]| {
        let entries: BTreeMap<RepoPath, FileEntry> = files
            .iter()
            .enumerate()
            .map(|(n, text)| (RepoPath::from(format!("f{n}.txt").as_str()), entry_of(text, 0o100644)))
            .collect();
        Snapshot {
            id: merkle::snapshot_id(&entries),
            entries,
        }
    };
    let mine = tree_of(&[&own, &shared]);
    let theirs = tree_of(&[&others, &shared]);
    let unowned = tree_of(&[&[7u8; 10]]);
    for s in [&mine, &theirs, &unowned] {
        writes
            .put_snapshot(s.clone(), SnapshotStatus::default(), T0)
            .await
            .unwrap();
    }
    writes
        .start_review(
            REPO.into(),
            tree_of_repo(),
            main_base(),
            vec![version_at(mine.id, 1)],
            T0,
        )
        .await
        .unwrap();
    writes
        .start_review(
            OTHER_REPO.into(),
            tree_of_repo(),
            main_base(),
            vec![version_at(theirs.id, 1)],
            T0,
        )
        .await
        .unwrap();

    let sizes = store.review_data().await.unwrap();
    assert_eq!(
        sizes,
        BTreeMap::from([(OTHER_REPO.to_owned(), 2000), (REPO.to_owned(), 1000)]),
        "each project's own content, not the shared content, and not what no project names"
    );
}

/// The fake of the session tests satisfies the contract as `SqliteStore` does.
#[cfg(test)]
mod fake {
    use crate::session::tests::MemStore;

    #[tokio::test]
    async fn the_fake_satisfies_the_contract() {
        super::store_contract(|| async { MemStore::default() }).await;
    }
}
