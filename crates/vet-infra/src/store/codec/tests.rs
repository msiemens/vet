//! What the codec makes of rows: the ones it wrote come back as they were, and the ones no code of
//! Vet wrote are refused as corrupt rather than read as something else.

use sea_orm::{ActiveEnum, Iterable, TryIntoModel};
use vet_app::changeset::{
    BaseChoice, ChangedFile, ChangesetId, CommitId, Source, Version, VersionChange, VersionOrigin,
};
use vet_app::platform::RemoteKey;
use vet_app::review::{ReviewId, SpanId, SpanOrigin};
use vet_core::content::{
    BlobId, EntryKind, FileEntry, Hash, LineDelta, LineRange, NonTextKind, RepoPath, SnapshotId, SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::review::{
    Anchor, CommentTarget, EditConflict, Message, MessageId, Outgoing, PendingVote, PlatformThread, RemoteComment,
    Side, SpanState, ThreadStatus, Verdict,
};

use super::super::entity::{
    ack, anchor, changeset, kinds, message, project_filter_item, review, snapshot_path, span, thread, tree_child,
    version, version_file,
};
use super::*;

fn h(n: u8) -> Hash {
    Hash([n; 32])
}

/// A way to damage a row, named by what is wrong with it.
type Damage<T> = fn(&mut T);

fn is_corrupt<T>(result: Result<T, DbErr>) -> bool {
    matches!(&result, Err(DbErr::Custom(m)) if m.starts_with("corrupt row"))
}

fn anchor_value(context: usize) -> Anchor {
    Anchor {
        side: Side::Target,
        snapshot: SnapshotId(h(1)),
        file: RepoPath(b"d\xffir/f.rs".to_vec()),
        range: LineRange { start: 3, end: 9 },
        content_hash_raw: h(2),
        content_hash_norm: h(3),
        context_hashes: (0..context).map(|i| h(10 + i as u8)).collect(),
    }
}

fn model<M: sea_orm::ModelTrait, A: TryIntoModel<M>>(row: A) -> M {
    row.try_into_model().unwrap()
}

fn anchor_model(a: &Anchor) -> anchor::Model {
    let mut row = anchor_row(7, a);
    row.id = Set(1);
    model(row)
}

#[test]
fn an_anchor_comes_back_with_its_context_hashes_in_order() {
    for context in [0, 1, 4] {
        let a = anchor_value(context);

        assert_eq!(anchor_from(&anchor_model(&a)).unwrap(), a, "{context} context hashes");
    }
}

#[test]
fn an_anchor_row_that_is_not_what_the_codec_writes_is_corrupt() {
    let bad: [(&str, Damage<anchor::Model>); 8] = [
        ("snapshot too short", |m| m.snapshot.pop().map(drop).unwrap_or_default()),
        ("raw hash too long", |m| m.raw.push(0)),
        ("norm hash missing", |m| m.norm.clear()),
        ("context ends mid-hash", |m| m.context.push(9)),
        ("context is a partial hash", |m| m.context = vec![1; 31]),
        ("negative start", |m| m.start = -1),
        ("negative end", |m| m.end = -1),
        ("end beyond u32", |m| m.end = i64::from(u32::MAX) + 1),
    ];
    for (what, damage) in bad {
        let mut m = anchor_model(&anchor_value(2));
        damage(&mut m);
        assert!(is_corrupt(anchor_from(&m)), "{what}");
    }
}

fn version_value() -> Version {
    Version {
        n: 3,
        snapshots: Some(Snapshots {
            base: SnapshotId(h(1)),
            target: SnapshotId(h(2)),
        }),
        base_commit: CommitId("aa".into()),
        tip: CommitId("bb".into()),
        at: 1_700_000_000,
        origin: VersionOrigin::Observed,
        change: VersionChange {
            files: vec![
                ChangedFile {
                    path: RepoPath(b"z.rs".to_vec()),
                    delta: LineDelta { added: 4, removed: 1 },
                },
                ChangedFile {
                    path: RepoPath(b"a\xff.rs".to_vec()),
                    delta: LineDelta::default(),
                },
            ],
        },
    }
}

fn version_models(v: &Version) -> (version::Model, Vec<version_file::Model>) {
    let mut row = version_row(ChangesetId(5), v);
    row.id = Set(11);
    let files = version_file_rows(11, &v.change)
        .into_iter()
        .enumerate()
        .map(|(i, mut f)| {
            f.id = Set(i as i64 + 1);
            model(f)
        })
        .collect();
    (model(row), files)
}

#[test]
fn a_version_comes_back_with_its_changed_files_in_the_order_they_were_written() {
    let v = version_value();
    let (row, files) = version_models(&v);

    assert_eq!(version_from(row, files).unwrap(), v);
}

#[test]
fn a_version_without_snapshots_comes_back_without_them() {
    let v = Version {
        snapshots: None,
        ..version_value()
    };
    let (row, files) = version_models(&v);

    assert_eq!(version_from(row, files).unwrap(), v);
}

#[test]
fn a_version_row_that_is_not_what_the_codec_writes_is_corrupt() {
    type VersionDamage = fn(&mut version::Model, &mut Vec<version_file::Model>);
    let bad: [(&str, VersionDamage); 6] = [
        ("negative number", |m, _| m.n = -1),
        ("number beyond u32", |m, _| m.n = i64::from(u32::MAX) + 1),
        ("base hash of 31 bytes", |m, _| {
            m.base.pop().map(drop).unwrap_or_default()
        }),
        ("target hash empty", |m, _| m.target.clear()),
        ("negative added lines", |_, f| f[0].added = -1),
        ("negative removed lines", |_, f| f[1].removed = -3),
    ];
    for (what, damage) in bad {
        let (mut row, mut files) = version_models(&version_value());
        damage(&mut row, &mut files);
        assert!(is_corrupt(version_from(row, files)), "{what}");
    }
}

fn tree_child_model(kind: kinds::ChildKind) -> tree_child::Model {
    tree_child::Model {
        node: vec![0; 32],
        name: b"name".to_vec(),
        kind,
        dir: None,
        blob: None,
        mode: None,
        entry_kind: None,
        unfiltered: None,
    }
}

fn file_child() -> tree_child::Model {
    tree_child::Model {
        blob: Some(h(4).0.to_vec()),
        mode: Some(0o100755),
        entry_kind: Some(kinds::EntryKind::Symlink),
        unfiltered: Some(true),
        ..tree_child_model(kinds::ChildKind::File)
    }
}

#[test]
fn a_node_reads_its_files_with_their_entries_and_its_directories_with_their_hashes() {
    let dir = tree_child::Model {
        name: b"sub".to_vec(),
        dir: Some(h(9).0.to_vec()),
        ..tree_child_model(kinds::ChildKind::Dir)
    };

    let node = node_from(vec![file_child(), dir]).unwrap();

    assert_eq!(node.dirs.get(b"sub".as_slice()), Some(&h(9)));
    assert_eq!(
        node.files.get(b"name".as_slice()),
        Some(&FileEntry {
            blob: BlobId(h(4)),
            mode: 0o100755,
            kind: EntryKind::Symlink,
            unfiltered: true,
        })
    );
}

#[test]
fn a_node_row_that_lacks_what_its_kind_needs_is_corrupt() {
    let bad: [(&str, tree_child::Model); 8] = [
        ("directory without hash", tree_child_model(kinds::ChildKind::Dir)),
        (
            "directory hash of 31 bytes",
            tree_child::Model {
                dir: Some(vec![1; 31]),
                ..tree_child_model(kinds::ChildKind::Dir)
            },
        ),
        ("file without anything", tree_child_model(kinds::ChildKind::File)),
        (
            "file without blob",
            tree_child::Model {
                blob: None,
                ..file_child()
            },
        ),
        (
            "file without mode",
            tree_child::Model {
                mode: None,
                ..file_child()
            },
        ),
        (
            "file without kind",
            tree_child::Model {
                entry_kind: None,
                ..file_child()
            },
        ),
        (
            "file without the unfiltered flag",
            tree_child::Model {
                unfiltered: None,
                ..file_child()
            },
        ),
        (
            "file with a negative mode",
            tree_child::Model {
                mode: Some(-1),
                ..file_child()
            },
        ),
    ];
    for (what, row) in bad {
        assert!(is_corrupt(node_from(vec![row])), "{what}");
    }
}

fn entry(kind: EntryKind, mode: u32) -> FileEntry {
    FileEntry {
        blob: BlobId(h(6)),
        mode,
        kind,
        unfiltered: false,
    }
}

fn ack_model(a: &NonTextAck) -> ack::Model {
    let mut row = ack_row(2, a);
    row.id = Set(1);
    model(row)
}

#[test]
fn an_acknowledgement_keeps_which_side_was_absent() {
    let both = NonTextAck {
        kind: NonTextKind::Mode,
        before: Some((RepoPath(b"f".to_vec()), entry(EntryKind::File, 0o100644))),
        after: Some((RepoPath(b"f".to_vec()), entry(EntryKind::File, 0o100755))),
    };
    let added = NonTextAck {
        kind: NonTextKind::Empty,
        before: None,
        after: Some((RepoPath(b"g\xff".to_vec()), entry(EntryKind::File, 0o100644))),
    };
    let deleted = NonTextAck {
        kind: NonTextKind::Submodule,
        before: Some((RepoPath(b"m".to_vec()), entry(EntryKind::Submodule, 0o160000))),
        after: None,
    };

    for a in [both, added, deleted] {
        assert_eq!(ack_from(ack_model(&a)).unwrap(), a);
    }
}

#[test]
fn an_acknowledgement_row_with_an_unreadable_side_is_corrupt() {
    let a = NonTextAck {
        kind: NonTextKind::Mode,
        before: Some((RepoPath(b"f".to_vec()), entry(EntryKind::File, 0o100644))),
        after: Some((RepoPath(b"f".to_vec()), entry(EntryKind::File, 0o100755))),
    };
    let bad: [(&str, Damage<ack::Model>); 6] = [
        ("before blob too short", |m| m.before_blob = Some(vec![1; 5])),
        ("after blob too long", |m| m.after_blob = Some(vec![1; 33])),
        ("negative mode", |m| m.after_mode = Some(-5)),
        ("before side without its path", |m| m.before_path = None),
        ("after side without its kind", |m| m.after_kind = None),
        ("after side with only its flag", |m| {
            m.after_path = None;
            m.after_blob = None;
            m.after_mode = None;
            m.after_kind = None;
        }),
    ];
    for (what, damage) in bad {
        let mut m = ack_model(&a);
        damage(&mut m);
        assert!(is_corrupt(ack_from(m)), "{what}");
    }
}

fn filter_models(repo: &str, filter: &ReviewFilter) -> Vec<project_filter_item::Model> {
    filter_rows(repo, filter)
        .into_iter()
        .enumerate()
        .map(|(i, mut row)| {
            row.id = Set(i as i64 + 1);
            model(row)
        })
        .collect()
}

#[test]
fn a_filter_comes_back_whole_with_roots_that_are_not_text() {
    let filter = ReviewFilter {
        roots: vec![RepoPath(b"src".to_vec()), RepoPath(b"d\xffir".to_vec())],
        include: vec!["*.rs".into(), "ü/**".into()],
        exclude: vec!["target/**".into()],
        hide_binary: true,
        review_generated: true,
    };

    let models = filter_models("/r", &filter);

    assert_eq!(filter_from(true, models).unwrap(), filter);
}

#[test]
fn a_filter_that_reviews_no_generated_files_has_no_row_for_the_flag() {
    let filter = ReviewFilter {
        include: vec!["a".into()],
        ..Default::default()
    };

    let models = filter_models("/r", &filter);

    assert_eq!(models.len(), 1);
    assert!(!filter_from(false, models).unwrap().review_generated);
}

#[test]
fn a_filter_pattern_that_is_not_text_is_corrupt() {
    for kind in [kinds::FilterKind::Include, kinds::FilterKind::Exclude] {
        let row = project_filter_item::Model {
            id: 1,
            repo: "/r".into(),
            kind,
            value: vec![b'a', 0xff],
        };
        assert!(is_corrupt(filter_from(false, vec![row])), "{kind:?}");
    }
}

#[test]
fn a_sources_key_is_its_branch_or_the_pull_request_in_one_string() {
    let pull = Source::PullRequest {
        remote: RemoteKey {
            host: "github.com".into(),
            path: "o/r".into(),
        },
        number: 42,
    };

    assert_eq!(
        source_columns(&Source::WorkingTree),
        (kinds::SourceKind::WorkingTree, String::new())
    );
    assert_eq!(
        source_columns(&Source::Branch { name: "feat/x".into() }),
        (kinds::SourceKind::Branch, "feat/x".to_owned())
    );
    assert_eq!(
        source_columns(&pull),
        (kinds::SourceKind::PullRequest, "github.com/o/r#42".to_owned())
    );
}

#[test]
fn a_base_choice_comes_back_as_it_was_chosen() {
    for choice in [
        BaseChoice::MergeBase {
            reference: "origin/main".into(),
        },
        BaseChoice::Commit {
            id: CommitId("abc123".into()),
        },
    ] {
        let (kind, value) = base_columns(choice.clone());

        assert_eq!(base_from(kind, value), choice);
    }
}

#[test]
fn the_flags_of_a_snapshots_paths_keep_apart() {
    let status = SnapshotStatus {
        path_collisions: vec![RepoPath(b"Case".to_vec()), RepoPath(b"case".to_vec())],
        content_unavailable: vec![RepoPath(b"big\xff.bin".to_vec())],
    };
    let rows: Vec<snapshot_path::Model> = status_rows(&[1; 32], &status)
        .into_iter()
        .enumerate()
        .map(|(i, mut r)| {
            r.id = Set(i as i64 + 1);
            model(r)
        })
        .collect();

    assert_eq!(status_from(rows), status);
}

fn message_value(remote: Option<RemoteComment>, outgoing: Outgoing) -> Message {
    Message {
        id: MessageId(4),
        author: "Tess".into(),
        body: "text".into(),
        at: 100,
        mine: true,
        avatar: Some("https://a/b.png".into()),
        edited_at: Some(150),
        remote,
        outgoing,
    }
}

fn message_model(m: &Message) -> message::Model {
    let mut row = message_row(9, m);
    row.id = Set(m.id.0 as i64);
    model(row)
}

#[test]
fn a_message_keeps_the_state_of_its_trip_to_the_platform() {
    let conflicted = RemoteComment {
        id: "c1".into(),
        synced: "old".into(),
        conflict: Some(EditConflict {
            remote_body: "theirs".into(),
            remote_at: 120,
        }),
    };
    let synced = RemoteComment {
        id: "c2".into(),
        synced: "same".into(),
        conflict: None,
    };
    for m in [
        message_value(None, Outgoing::Draft),
        message_value(None, Outgoing::Posting { since: 77 }),
        message_value(None, Outgoing::Failed { error: "403".into() }),
        message_value(Some(conflicted), Outgoing::Draft),
        message_value(Some(synced), Outgoing::Draft),
    ] {
        assert_eq!(message_from(message_model(&m)).unwrap(), m);
    }
}

#[test]
fn a_message_row_whose_columns_contradict_each_other_is_corrupt() {
    let bad: [(&str, Damage<message::Model>); 3] = [
        ("a remote id without the synced text", |m| {
            m.remote_id = Some("c".into());
            m.synced = None;
        }),
        ("synced text without a remote id", |m| {
            m.remote_id = None;
            m.synced = Some("s".into());
        }),
        ("posting and failed at once", |m| {
            m.posting_since = Some(1);
            m.post_error = Some("e".into());
        }),
    ];
    for (what, damage) in bad {
        let mut m = message_model(&message_value(None, Outgoing::Draft));
        damage(&mut m);
        assert!(is_corrupt(message_from(m)), "{what}");
    }
}

#[test]
fn half_a_conflict_is_no_conflict() {
    let remote = RemoteComment {
        id: "c".into(),
        synced: "s".into(),
        conflict: Some(EditConflict {
            remote_body: "b".into(),
            remote_at: 5,
        }),
    };
    let clearing: [fn(&mut message::Model); 2] = [|m| m.conflict_at = None, |m| m.conflict_body = None];
    for clear in clearing {
        let mut m = message_model(&message_value(Some(remote.clone()), Outgoing::Draft));
        clear(&mut m);

        assert_eq!(message_from(m).unwrap().remote.unwrap().conflict, None);
    }
}

fn changeset_model(vote: Option<&PendingVote>) -> changeset::Model {
    let mut row = <changeset::ActiveModel as Default>::default();
    vote_columns(&mut row, vote);
    changeset::Model {
        id: 1,
        repo: "/r".into(),
        kind: kinds::SourceKind::WorkingTree,
        key: String::new(),
        lens: None,
        title: None,
        draft: false,
        fork: false,
        trusted_head: None,
        target_branch: None,
        pinned_base: None,
        vote: row.vote.unwrap(),
        vote_posting_since: row.vote_posting_since.unwrap(),
        vote_error: row.vote_error.unwrap(),
    }
}

#[test]
fn a_pending_vote_keeps_its_verdict_and_state_and_no_vote_leaves_no_state() {
    assert_eq!(vote_from(changeset_model(None)).unwrap(), None);
    for outgoing in [
        Outgoing::Draft,
        Outgoing::Posting { since: 9 },
        Outgoing::Failed { error: "no".into() },
    ] {
        let vote = PendingVote {
            verdict: Verdict::Approve,
            outgoing,
        };

        assert_eq!(vote_from(changeset_model(Some(&vote))).unwrap(), Some(vote.clone()));
        assert_eq!(changeset_model(Some(&vote)).vote.as_deref(), Some("approve"));
    }
}

#[test]
fn a_vote_is_kept_spelled_how_vet_spells_it_and_one_spelled_else_stays_so() {
    assert_eq!(
        changeset_model(Some(&PendingVote {
            verdict: Verdict::ApproveWithSuggestions,
            outgoing: Outgoing::Draft,
        }))
        .vote
        .as_deref(),
        Some("approve-with-suggestions")
    );
    // A spelling that is no verdict of Vet's keeps spelled, and no verdict pretends to read it.
    assert_eq!(spelled_verdict("approved").clone(), Verdict::Other("approved".into()));
    let strange = PendingVote {
        verdict: Verdict::Other("hidden rule".into()),
        outgoing: Outgoing::Draft,
    };
    assert_eq!(changeset_model(Some(&strange)).vote.as_deref(), Some("hidden rule"));
    assert_eq!(vote_from(changeset_model(Some(&strange))).unwrap(), Some(strange));
}

#[test]
fn a_votes_state_without_a_vote_and_a_vote_in_two_states_are_corrupt() {
    let vote = PendingVote {
        verdict: Verdict::Approve,
        outgoing: Outgoing::Draft,
    };
    let stray_posting = changeset::Model {
        vote_posting_since: Some(1),
        ..changeset_model(None)
    };
    let stray_error = changeset::Model {
        vote_error: Some("e".into()),
        ..changeset_model(None)
    };
    let both = changeset::Model {
        vote_posting_since: Some(1),
        vote_error: Some("e".into()),
        ..changeset_model(Some(&vote))
    };

    for (what, row) in [("posting", stray_posting), ("error", stray_error), ("both", both)] {
        assert!(is_corrupt(vote_from(row)), "{what}");
    }
}

fn thread_model(id: i64) -> thread::Model {
    thread::Model {
        id,
        changeset: 1,
        review: None,
        kind: kinds::ThreadKind::Changeset,
        file_side: None,
        file_path: None,
        anchor: None,
        status: kinds::ThreadStatus::Open,
        on_platform: false,
        remote_id: None,
        resolvable: false,
        resolved_by: None,
        status_pending: false,
    }
}

fn message_row_model(id: i64, thread: i64, at: i64, remote: bool) -> message::Model {
    message::Model {
        id,
        thread,
        author: "a".into(),
        body: format!("m{id}"),
        at,
        mine: false,
        avatar: None,
        edited_at: None,
        remote_id: remote.then(|| format!("r{id}")),
        synced: remote.then(|| "s".to_owned()),
        conflict_body: None,
        conflict_at: None,
        posting_since: None,
        post_error: None,
    }
}

fn bodies(t: &ChangesetThread) -> Vec<&str> {
    t.thread.messages.iter().map(|m| m.body.as_str()).collect()
}

#[test]
fn messages_go_to_their_thread_in_the_order_written_and_a_platforms_own_come_first_by_time() {
    let local = thread_model(1);
    let platform = thread::Model {
        on_platform: true,
        remote_id: Some("t".into()),
        resolvable: true,
        resolved_by: Some("Ann".into()),
        status_pending: true,
        review: Some(8),
        ..thread_model(2)
    };
    let messages = vec![
        message_row_model(1, 1, 50, false),
        message_row_model(2, 2, 5, false),
        message_row_model(3, 2, 20, true),
        message_row_model(4, 1, 10, false),
        message_row_model(5, 2, 20, true),
        message_row_model(6, 2, 10, true),
    ];

    let threads = threads_from(vec![local, platform], vec![], messages).unwrap();

    assert_eq!(bodies(&threads[0]), ["m1", "m4"], "written order, not by time");
    assert_eq!(
        bodies(&threads[1]),
        ["m6", "m3", "m5", "m2"],
        "the platform's by time and id, then the draft"
    );
    assert_eq!(threads[0].thread.platform, None);
    assert_eq!(
        threads[1].thread.platform,
        Some(PlatformThread {
            remote_id: Some("t".into()),
            resolvable: true,
            resolved_by: Some("Ann".into()),
            status_pending: true,
        })
    );
    assert_eq!(threads[1].review, Some(ReviewId(8)));
    assert_eq!(threads[0].thread.status, ThreadStatus::Open);
}

#[test]
fn a_thread_reads_its_target_from_the_columns_its_kind_uses() {
    let a = anchor_value(1);
    let mut anchor_row = anchor_model(&a);
    anchor_row.id = 30;
    let range = thread::Model {
        kind: kinds::ThreadKind::Range,
        anchor: Some(30),
        ..thread_model(1)
    };
    let file = thread::Model {
        kind: kinds::ThreadKind::File,
        file_side: Some(kinds::Side::Base),
        file_path: Some(b"f\xff".to_vec()),
        ..thread_model(2)
    };

    let threads = threads_from(vec![range, file, thread_model(3)], vec![anchor_row], vec![]).unwrap();

    let targets: Vec<_> = threads.into_iter().map(|t| t.thread.target).collect();
    assert_eq!(
        targets,
        [
            CommentTarget::Range(a),
            CommentTarget::File {
                side: Side::Base,
                path: RepoPath(b"f\xff".to_vec())
            },
            CommentTarget::Changeset
        ]
    );
}

#[test]
fn a_thread_row_that_lacks_what_its_kind_needs_is_corrupt() {
    let range_without_anchor = thread::Model {
        kind: kinds::ThreadKind::Range,
        anchor: Some(99),
        ..thread_model(1)
    };
    let file_without_side = thread::Model {
        kind: kinds::ThreadKind::File,
        file_path: Some(b"f".to_vec()),
        ..thread_model(2)
    };
    let file_without_path = thread::Model {
        kind: kinds::ThreadKind::File,
        file_side: Some(kinds::Side::Target),
        ..thread_model(3)
    };

    for (what, row) in [
        ("range without its anchor", range_without_anchor),
        ("file without side", file_without_side),
        ("file without path", file_without_path),
    ] {
        assert!(is_corrupt(threads_from(vec![row], vec![], vec![])), "{what}");
    }
}

#[test]
fn a_thread_target_written_by_set_target_is_read_back() {
    let mut row = <thread::ActiveModel as Default>::default();
    set_target(
        &mut row,
        &CommentTarget::File {
            side: Side::Target,
            path: RepoPath(b"p".to_vec()),
        },
        None,
    );

    let m = thread::Model {
        kind: row.kind.unwrap(),
        file_side: row.file_side.unwrap(),
        file_path: row.file_path.unwrap(),
        anchor: row.anchor.unwrap(),
        ..thread_model(1)
    };

    assert_eq!(
        target_from(&m, None).unwrap(),
        CommentTarget::File {
            side: Side::Target,
            path: RepoPath(b"p".to_vec())
        }
    );
}

fn anchor_for_span(id: i64, a: &Anchor) -> anchor::Model {
    anchor::Model { id, ..anchor_model(a) }
}

fn review_model() -> review::Model {
    review::Model {
        id: 5,
        changeset: 2,
        base_kind: kinds::BaseKind::Commit,
        base_value: "abc".into(),
        last_reviewed: Some(3),
        opened_at: 10,
        closed_at: None,
        complete: false,
        summary_total: None,
        summary_seen: None,
        summary_old_base: None,
    }
}

fn rows(review: review::Model) -> ReviewRows {
    ReviewRows {
        review,
        anchors: vec![],
        spans: vec![],
        acks: vec![],
    }
}

#[test]
fn a_review_is_complete_or_not_only_once_it_is_closed() {
    let open = review::Model {
        complete: true,
        ..review_model()
    };
    let closed = review::Model {
        closed_at: Some(20),
        complete: true,
        ..review_model()
    };
    let closed_incomplete = review::Model {
        closed_at: Some(20),
        ..review_model()
    };

    assert_eq!(review_from(rows(open)).unwrap().complete, None);
    assert_eq!(review_from(rows(closed)).unwrap().complete, Some(true));
    assert_eq!(review_from(rows(closed_incomplete)).unwrap().complete, Some(false));
}

#[test]
fn a_summary_is_there_only_when_all_three_counts_are() {
    let counts = |total, seen, old_base| review::Model {
        summary_total: total,
        summary_seen: seen,
        summary_old_base: old_base,
        ..review_model()
    };

    let full = review_from(rows(counts(Some(10), Some(6), Some(2)))).unwrap();
    assert_eq!(
        full.summary,
        Some(ReviewSummary {
            total: 10,
            seen: 6,
            old_base: 2
        })
    );
    for partial in [
        counts(Some(10), Some(6), None),
        counts(None, Some(6), Some(2)),
        counts(Some(1), None, None),
    ] {
        assert_eq!(review_from(rows(partial)).unwrap().summary, None);
    }
    assert!(is_corrupt(review_from(rows(counts(Some(10), Some(-6), Some(2))))));
}

#[test]
fn a_review_row_with_a_negative_version_number_is_corrupt() {
    let row = review::Model {
        last_reviewed: Some(-1),
        ..review_model()
    };

    assert!(is_corrupt(review_from(rows(row))));
}

#[test]
fn a_span_is_found_with_its_anchors_and_the_commit_whose_diff_it_was_marked_in() {
    let (base, target) = (anchor_value(0), anchor_value(2));
    let span_row = |id, state, base_anchor, target_anchor, origin_commit: Option<&str>| span::Model {
        id,
        review: 5,
        state,
        base_anchor,
        target_anchor,
        origin_commit: origin_commit.map(str::to_owned),
    };
    let mut review_rows = rows(review_model());
    review_rows.anchors = vec![anchor_for_span(1, &base), anchor_for_span(2, &target)];
    review_rows.spans = vec![
        span_row(7, kinds::SpanState::Seen, Some(1), Some(2), None),
        span_row(8, kinds::SpanState::Flagged, None, Some(2), Some("c0ffee")),
        span_row(9, kinds::SpanState::SeenAgainstOldBase, Some(1), None, None),
    ];

    let spans = review_from(review_rows).unwrap().spans;

    let got: Vec<_> = spans
        .iter()
        .map(|s| {
            (
                s.id,
                s.span.base.is_some(),
                s.span.target.is_some(),
                s.span.state,
                s.origin.clone(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (SpanId(7), true, true, SpanState::Seen, SpanOrigin::Squash),
            (
                SpanId(8),
                false,
                true,
                SpanState::Flagged,
                SpanOrigin::Commit(CommitId("c0ffee".into()))
            ),
            (
                SpanId(9),
                true,
                false,
                SpanState::SeenAgainstOldBase,
                SpanOrigin::Squash
            ),
        ]
    );
    assert_eq!(spans[0].span.target.as_ref(), Some(&target));
}

#[test]
fn a_span_whose_anchor_row_is_missing_is_corrupt() {
    let mut review_rows = rows(review_model());
    review_rows.anchors = vec![anchor_for_span(1, &anchor_value(0))];
    review_rows.spans = vec![span::Model {
        id: 7,
        review: 5,
        state: kinds::SpanState::Seen,
        base_anchor: Some(1),
        target_anchor: Some(2),
        origin_commit: None,
    }];

    assert!(is_corrupt(review_from(review_rows)));
}

#[test]
fn the_numbers_the_database_holds_for_each_kind_are_the_ones_rows_already_written_hold() {
    // Renumbering a kind changes what every stored row means: it needs a revision of the stored
    // values in `migration`, not just a changed test.
    fn numbers<E: ActiveEnum<Value = i32>>(kinds: impl IntoIterator<Item = E>) -> Vec<i32> {
        kinds.into_iter().map(|k| k.to_value()).collect()
    }

    assert_eq!(numbers(kinds::Side::iter()), [0, 1]);
    assert_eq!(numbers(kinds::EntryKind::iter()), [0, 1, 2]);
    assert_eq!(numbers(kinds::SpanState::iter()), [0, 1, 2]);
    assert_eq!(numbers(kinds::ThreadKind::iter()), [0, 1, 2]);
    assert_eq!(numbers(kinds::ThreadStatus::iter()), [0, 1]);
    assert_eq!(numbers(kinds::FilterKind::iter()), [0, 1, 2, 3]);
    assert_eq!(numbers(kinds::BaseKind::iter()), [0, 1]);
    assert_eq!(numbers(kinds::ChildKind::iter()), [0, 1]);
    assert_eq!(numbers(kinds::PathFlag::iter()), [0, 1]);
    assert_eq!(numbers(kinds::ChangeKind::iter()), [0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(numbers(kinds::SourceKind::iter()), [0, 1, 2]);
    assert_eq!(numbers(kinds::VersionOrigin::iter()), [0, 1]);
    assert_eq!(numbers(kinds::Lens::iter()), [0, 1]);
}
