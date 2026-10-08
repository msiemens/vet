//! Mapping between `vet-core` review types and entity rows. Isolated so a
//! change to anchors, spans or entries touches only this file.

use std::collections::HashMap;

use sea_orm::sea_query::{SimpleExpr, Value};
use sea_orm::{ColumnTrait, DbErr, Set};
use vet_app::changeset::{
    BaseChoice, ChangedFile, ChangesetId, CommitId, Lens, Source, Version, VersionChange, VersionOrigin,
};
use vet_app::review::{ChangesetThread, NonTextAck, RecordedSpan, Review, ReviewId, ReviewSummary, SpanId, SpanOrigin};
use vet_core::content::{
    BlobId, EntryKind, FileEntry, Hash, LineDelta, LineRange, NonTextKind, RepoPath, SnapshotId, SnapshotStatus,
};
use vet_core::filter::ReviewFilter;
use vet_core::merkle::TreeNode;
use vet_core::review::{
    Anchor, CommentTarget, EditConflict, Message, MessageId, PlatformThread, RemoteComment, ReviewedSpan, Side,
    SpanState, Thread, ThreadId, ThreadStatus,
};

use super::entity::{
    ack, anchor, kinds, message, project_filter_item, review, snapshot_path, span, thread, tree_child, version,
    version_file,
};

pub fn corrupt(what: &str) -> DbErr {
    DbErr::Custom(format!("corrupt row: {what}"))
}

pub fn hash(bytes: &[u8]) -> Result<Hash, DbErr> {
    <[u8; 32]>::try_from(bytes)
        .map(Hash)
        .map_err(|_| corrupt("hash is not 32 bytes"))
}

/// Infallible conversions between a `vet-core` enum and its stored twin.
macro_rules! twin {
    ($core:ident, $db:ident { $($variant:ident),+ }) => {
        impl From<$core> for kinds::$db {
            fn from(v: $core) -> Self {
                match v { $($core::$variant => Self::$variant),+ }
            }
        }
        impl From<kinds::$db> for $core {
            fn from(v: kinds::$db) -> Self {
                match v { $(kinds::$db::$variant => Self::$variant),+ }
            }
        }
    };
}

twin!(Side, Side { Base, Target });
twin!(
    EntryKind,
    EntryKind {
        File,
        Symlink,
        Submodule
    }
);
twin!(
    SpanState,
    SpanState {
        Seen,
        SeenAgainstOldBase
    }
);
twin!(ThreadStatus, ThreadStatus { Open, Resolved });
twin!(
    VersionOrigin,
    VersionOrigin {
        Settled,
        BaseMoved,
        Observed
    }
);
twin!(Lens, Lens { Squash, PerCommit });
twin!(
    NonTextKind,
    ChangeKind {
        Mode,
        Symlink,
        Submodule,
        Binary,
        ContentUnavailable,
        Rename,
        Empty
    }
);

/// The item rows of `repo`'s filter, roots first, in the order they are read back.
pub fn filter_rows(repo: &str, filter: &ReviewFilter) -> Vec<project_filter_item::ActiveModel> {
    let item = |kind: kinds::FilterKind, value: Vec<u8>| project_filter_item::ActiveModel {
        repo: Set(repo.to_owned()),
        kind: Set(kind),
        value: Set(value),
        ..Default::default()
    };
    let text = |kind, patterns: &[String]| {
        patterns
            .iter()
            .map(|s| item(kind, s.clone().into_bytes()))
            .collect::<Vec<_>>()
    };
    let roots = filter.roots.iter().map(|r| item(kinds::FilterKind::Root, r.0.clone()));
    roots
        .chain(text(kinds::FilterKind::Include, &filter.include))
        .chain(text(kinds::FilterKind::Exclude, &filter.exclude))
        .collect()
}

pub fn filter_from(hide_binary: bool, items: Vec<project_filter_item::Model>) -> Result<ReviewFilter, DbErr> {
    let mut filter = ReviewFilter {
        hide_binary,
        ..Default::default()
    };
    for item in items {
        let text = |v: Vec<u8>| String::from_utf8(v).map_err(|_| corrupt("filter pattern is not UTF-8"));
        match item.kind {
            kinds::FilterKind::Root => filter.roots.push(RepoPath(item.value)),
            kinds::FilterKind::Include => filter.include.push(text(item.value)?),
            kinds::FilterKind::Exclude => filter.exclude.push(text(item.value)?),
        }
    }
    Ok(filter)
}

/// The kind and value a base choice is stored as.
pub fn base_columns(choice: BaseChoice) -> (kinds::BaseKind, String) {
    match choice {
        BaseChoice::MergeBase { reference } => (kinds::BaseKind::MergeBase, reference),
        BaseChoice::Commit { id } => (kinds::BaseKind::Commit, id.0),
    }
}

pub fn base_from(kind: kinds::BaseKind, value: String) -> BaseChoice {
    match kind {
        kinds::BaseKind::MergeBase => BaseChoice::MergeBase { reference: value },
        kinds::BaseKind::Commit => BaseChoice::Commit { id: CommitId(value) },
    }
}

/// The kind and key a source is stored as: the branch name, or `host/path#number`
/// of a pull request, and nothing for the working tree.
pub fn source_columns(source: &Source) -> (kinds::SourceKind, String) {
    match source {
        Source::WorkingTree => (kinds::SourceKind::WorkingTree, String::new()),
        Source::Branch { name } => (kinds::SourceKind::Branch, name.clone()),
        Source::PullRequest { remote, number } => (
            kinds::SourceKind::PullRequest,
            format!("{}/{}#{number}", remote.host, remote.path),
        ),
    }
}

pub fn version_row(changeset: ChangesetId, v: &Version) -> version::ActiveModel {
    version::ActiveModel {
        changeset: Set(changeset.0 as i64),
        n: Set(v.n.into()),
        base: Set(v.base.0.0.to_vec()),
        base_commit: Set(v.base_commit.0.clone()),
        target: Set(v.target.0.0.to_vec()),
        tip: Set(v.tip.0.clone()),
        at: Set(v.at),
        origin: Set(v.origin.into()),
        ..Default::default()
    }
}

/// The rows of what version `id` changed.
pub fn version_file_rows(id: i64, change: &VersionChange) -> Vec<version_file::ActiveModel> {
    change
        .files
        .iter()
        .map(|f| version_file::ActiveModel {
            version: Set(id),
            path: Set(f.path.0.clone()),
            added: Set(f.delta.added.into()),
            removed: Set(f.delta.removed.into()),
            ..Default::default()
        })
        .collect()
}

/// A version from its row and the rows of its files, in the order they were written.
pub fn version_from(m: version::Model, files: Vec<version_file::Model>) -> Result<Version, DbErr> {
    let count = |n: i64| u32::try_from(n).map_err(|_| corrupt("line count"));
    let files = files
        .into_iter()
        .map(|f| {
            Ok(ChangedFile {
                path: RepoPath(f.path),
                delta: LineDelta {
                    added: count(f.added)?,
                    removed: count(f.removed)?,
                },
            })
        })
        .collect::<Result<_, DbErr>>()?;
    Ok(Version {
        change: VersionChange { files },
        n: u32::try_from(m.n).map_err(|_| corrupt("version number"))?,
        base: SnapshotId(hash(&m.base)?),
        base_commit: CommitId(m.base_commit),
        target: SnapshotId(hash(&m.target)?),
        tip: CommitId(m.tip),
        at: m.at,
        origin: m.origin.into(),
    })
}

pub fn anchor_row(changeset: i64, a: &Anchor) -> anchor::ActiveModel {
    anchor::ActiveModel {
        changeset: Set(changeset),
        side: Set(a.side.into()),
        snapshot: Set(a.snapshot.0.0.to_vec()),
        file: Set(a.file.0.clone()),
        start: Set(a.range.start.into()),
        end: Set(a.range.end.into()),
        raw: Set(a.content_hash_raw.0.to_vec()),
        norm: Set(a.content_hash_norm.0.to_vec()),
        context: Set(a.context_hashes.iter().flat_map(|h| h.0).collect()),
        ..Default::default()
    }
}

pub fn anchor_from(m: &anchor::Model) -> Result<Anchor, DbErr> {
    Ok(Anchor {
        side: m.side.into(),
        snapshot: SnapshotId(hash(&m.snapshot)?),
        file: RepoPath(m.file.clone()),
        range: LineRange {
            start: u32::try_from(m.start).map_err(|_| corrupt("range"))?,
            end: u32::try_from(m.end).map_err(|_| corrupt("range"))?,
        },
        content_hash_raw: hash(&m.raw)?,
        content_hash_norm: hash(&m.norm)?,
        context_hashes: m.context.chunks(32).map(hash).collect::<Result<_, _>>()?,
    })
}

pub fn child_rows(node: Hash, n: &TreeNode) -> Vec<tree_child::ActiveModel> {
    let row = |name: &[u8], kind| tree_child::ActiveModel {
        node: Set(node.0.to_vec()),
        name: Set(name.to_vec()),
        kind: Set(kind),
        ..Default::default()
    };
    let files = n.files.iter().map(|(name, e)| tree_child::ActiveModel {
        blob: Set(Some(e.blob.0.0.to_vec())),
        mode: Set(Some(e.mode.into())),
        entry_kind: Set(Some(e.kind.into())),
        unfiltered: Set(Some(e.unfiltered)),
        ..row(name, kinds::ChildKind::File)
    });
    let dirs = n.dirs.iter().map(|(name, h)| tree_child::ActiveModel {
        dir: Set(Some(h.0.to_vec())),
        ..row(name, kinds::ChildKind::Dir)
    });
    files.chain(dirs).collect()
}

pub fn node_from(rows: Vec<tree_child::Model>) -> Result<TreeNode, DbErr> {
    let mut node = TreeNode::default();
    for r in rows {
        match r.kind {
            kinds::ChildKind::Dir => {
                node.dirs
                    .insert(r.name, hash(&r.dir.ok_or_else(|| corrupt("dir child without hash"))?)?);
            }
            kinds::ChildKind::File => {
                let (Some(blob), Some(mode), Some(kind), Some(unfiltered)) =
                    (r.blob, r.mode, r.entry_kind, r.unfiltered)
                else {
                    return Err(corrupt("file child without entry"));
                };
                let entry = FileEntry {
                    blob: BlobId(hash(&blob)?),
                    mode: u32::try_from(mode).map_err(|_| corrupt("mode"))?,
                    kind: kind.into(),
                    unfiltered,
                };
                node.files.insert(r.name, entry);
            }
        }
    }
    Ok(node)
}

pub fn status_rows(snapshot: &[u8], status: &SnapshotStatus) -> Vec<snapshot_path::ActiveModel> {
    let row = |flag, p: &RepoPath| snapshot_path::ActiveModel {
        snapshot: Set(snapshot.to_vec()),
        flag: Set(flag),
        path: Set(p.0.clone()),
        ..Default::default()
    };
    let collisions = status
        .path_collisions
        .iter()
        .map(|p| row(kinds::PathFlag::Collision, p));
    collisions
        .chain(
            status
                .content_unavailable
                .iter()
                .map(|p| row(kinds::PathFlag::ContentUnavailable, p)),
        )
        .collect()
}

pub fn status_from(rows: Vec<snapshot_path::Model>) -> SnapshotStatus {
    let mut status = SnapshotStatus::default();
    for r in rows {
        match r.flag {
            kinds::PathFlag::Collision => status.path_collisions.push(RepoPath(r.path)),
            kinds::PathFlag::ContentUnavailable => status.content_unavailable.push(RepoPath(r.path)),
        }
    }
    status
}

/// The flattened columns of one side of an acknowledgement.
#[derive(Default)]
pub struct AckSide {
    path: Option<Vec<u8>>,
    blob: Option<Vec<u8>>,
    mode: Option<i64>,
    kind: Option<kinds::EntryKind>,
    unfiltered: Option<bool>,
}

impl AckSide {
    fn new(entry: Option<&(RepoPath, FileEntry)>) -> Self {
        let Some((path, e)) = entry else { return Self::default() };
        Self {
            path: Some(path.0.clone()),
            blob: Some(e.blob.0.0.to_vec()),
            mode: Some(e.mode.into()),
            kind: Some(e.kind.into()),
            unfiltered: Some(e.unfiltered),
        }
    }

    fn entry(self) -> Result<Option<(RepoPath, FileEntry)>, DbErr> {
        let (Some(path), Some(blob), Some(mode), Some(kind), Some(unfiltered)) =
            (self.path, self.blob, self.mode, self.kind, self.unfiltered)
        else {
            return Ok(None);
        };
        let entry = FileEntry {
            blob: BlobId(hash(&blob)?),
            mode: u32::try_from(mode).map_err(|_| corrupt("mode"))?,
            kind: kind.into(),
            unfiltered,
        };
        Ok(Some((RepoPath(path), entry)))
    }
}

pub fn ack_row(review: i64, a: &NonTextAck) -> ack::ActiveModel {
    let (b, f) = (AckSide::new(a.before.as_ref()), AckSide::new(a.after.as_ref()));
    ack::ActiveModel {
        review: Set(review),
        change: Set(a.kind.into()),
        before_path: Set(b.path),
        before_blob: Set(b.blob),
        before_mode: Set(b.mode),
        before_kind: Set(b.kind),
        before_unfiltered: Set(b.unfiltered),
        after_path: Set(f.path),
        after_blob: Set(f.blob),
        after_mode: Set(f.mode),
        after_kind: Set(f.kind),
        after_unfiltered: Set(f.unfiltered),
        ..Default::default()
    }
}

fn is<T: Into<Value>>(column: impl ColumnTrait, value: Option<T>) -> SimpleExpr {
    match value {
        Some(v) => column.eq(v),
        None => column.is_null(),
    }
}

/// Matches rows equal to `a`, treating absent entries as NULL.
pub fn ack_filter(a: &NonTextAck) -> sea_orm::Condition {
    use ack::Column as C;
    let (b, f) = (AckSide::new(a.before.as_ref()), AckSide::new(a.after.as_ref()));
    sea_orm::Condition::all()
        .add(C::Change.eq(kinds::ChangeKind::from(a.kind)))
        .add(is(C::BeforePath, b.path))
        .add(is(C::BeforeBlob, b.blob))
        .add(is(C::BeforeMode, b.mode))
        .add(is(C::BeforeKind, b.kind))
        .add(is(C::BeforeUnfiltered, b.unfiltered))
        .add(is(C::AfterPath, f.path))
        .add(is(C::AfterBlob, f.blob))
        .add(is(C::AfterMode, f.mode))
        .add(is(C::AfterKind, f.kind))
        .add(is(C::AfterUnfiltered, f.unfiltered))
}

pub fn ack_from(m: ack::Model) -> Result<NonTextAck, DbErr> {
    let before = AckSide {
        path: m.before_path,
        blob: m.before_blob,
        mode: m.before_mode,
        kind: m.before_kind,
        unfiltered: m.before_unfiltered,
    };
    let after = AckSide {
        path: m.after_path,
        blob: m.after_blob,
        mode: m.after_mode,
        kind: m.after_kind,
        unfiltered: m.after_unfiltered,
    };
    Ok(NonTextAck {
        kind: m.change.into(),
        before: before.entry()?,
        after: after.entry()?,
    })
}

/// The rows of one stored review. Spans and acknowledgements are in the order
/// they were written; `anchors` are those the spans name.
pub struct ReviewRows {
    pub review: review::Model,
    pub anchors: Vec<anchor::Model>,
    pub spans: Vec<span::Model>,
    pub acks: Vec<ack::Model>,
}

pub fn review_from(rows: ReviewRows) -> Result<Review, DbErr> {
    let ReviewRows {
        review,
        anchors,
        spans,
        acks,
    } = rows;
    let anchors: HashMap<i64, anchor::Model> = anchors.into_iter().map(|a| (a.id, a)).collect();
    let anchor_of = |id: Option<i64>| id.and_then(|i| anchors.get(&i)).map(anchor_from).transpose();

    let spans = spans
        .into_iter()
        .map(|s| {
            Ok(RecordedSpan {
                id: SpanId(s.id as u64),
                span: ReviewedSpan {
                    base: anchor_of(s.base_anchor)?,
                    target: anchor_of(s.target_anchor)?,
                    state: s.state.into(),
                },
                origin: s
                    .origin_commit
                    .map_or(SpanOrigin::Squash, |c| SpanOrigin::Commit(CommitId(c))),
            })
        })
        .collect::<Result<_, DbErr>>()?;
    let acks = acks.into_iter().map(ack_from).collect::<Result<_, DbErr>>()?;

    Ok(Review {
        id: ReviewId(review.id as u64),
        changeset: ChangesetId(review.changeset as u64),
        base: base_from(review.base_kind, review.base_value),
        spans,
        acks,
        last_reviewed: review
            .last_reviewed
            .map(|n| u32::try_from(n).map_err(|_| corrupt("version number")))
            .transpose()?,
        opened_at: review.opened_at,
        closed_at: review.closed_at,
        complete: review.closed_at.map(|_| review.complete),
        summary: match (review.summary_total, review.summary_seen, review.summary_old_base) {
            (Some(total), Some(seen), Some(old_base)) => {
                let count = |n: i64| u32::try_from(n).map_err(|_| corrupt("review summary"));
                Some(ReviewSummary {
                    total: count(total)?,
                    seen: count(seen)?,
                    old_base: count(old_base)?,
                })
            }
            _ => None,
        },
    })
}

/// The threads of one changeset in the order given; `anchors` are those the
/// range threads name and `messages` are in the order they were written.
pub fn threads_from(
    threads: Vec<thread::Model>,
    anchors: Vec<anchor::Model>,
    messages: Vec<message::Model>,
) -> Result<Vec<ChangesetThread>, DbErr> {
    let anchors: HashMap<i64, anchor::Model> = anchors.into_iter().map(|a| (a.id, a)).collect();
    let mut by_thread: HashMap<i64, Vec<Message>> = HashMap::new();
    for m in messages {
        by_thread.entry(m.thread).or_default().push(message_from(m)?);
    }
    threads
        .into_iter()
        .map(|t| {
            let mut messages = by_thread.remove(&t.id).unwrap_or_default();
            if t.on_platform {
                // What the platform has comes in its order, the viewer's drafts after it.
                messages.sort_by_key(|m| (m.remote.is_none(), m.at, m.id));
            }
            Ok(ChangesetThread {
                thread: Thread {
                    id: ThreadId(t.id as u64),
                    target: target_from(&t, t.anchor.and_then(|a| anchors.get(&a)))?,
                    messages,
                    status: t.status.into(),
                    platform: t.on_platform.then(|| PlatformThread {
                        remote_id: t.remote_id.clone(),
                        resolvable: t.resolvable,
                        resolved_by: t.resolved_by.clone(),
                        status_pending: t.status_pending,
                    }),
                },
                review: t.review.map(|r| ReviewId(r as u64)),
            })
        })
        .collect()
}

pub fn message_from(m: message::Model) -> Result<Message, DbErr> {
    let remote = match (m.remote_id, m.synced) {
        (Some(id), Some(synced)) => Some(RemoteComment {
            id,
            synced,
            conflict: match (m.conflict_body, m.conflict_at) {
                (Some(remote_body), Some(remote_at)) => Some(EditConflict { remote_body, remote_at }),
                _ => None,
            },
        }),
        (None, None) => None,
        _ => return Err(corrupt("a comment's remote id and synced text go together")),
    };
    Ok(Message {
        id: MessageId(m.id as u64),
        author: m.author,
        body: m.body,
        at: m.at,
        mine: m.mine,
        avatar: m.avatar,
        edited_at: m.edited_at,
        remote,
    })
}

/// The columns of `message` in `thread`; the row id is the caller's.
pub fn message_row(thread: i64, m: &Message) -> message::ActiveModel {
    message::ActiveModel {
        thread: Set(thread),
        author: Set(m.author.clone()),
        body: Set(m.body.clone()),
        at: Set(m.at),
        mine: Set(m.mine),
        avatar: Set(m.avatar.clone()),
        edited_at: Set(m.edited_at),
        remote_id: Set(m.remote.as_ref().map(|r| r.id.clone())),
        synced: Set(m.remote.as_ref().map(|r| r.synced.clone())),
        conflict_body: Set(conflict(m).map(|c| c.remote_body.clone())),
        conflict_at: Set(conflict(m).map(|c| c.remote_at)),
        ..Default::default()
    }
}

fn conflict(m: &Message) -> Option<&EditConflict> {
    m.remote.as_ref().and_then(|r| r.conflict.as_ref())
}

/// Sets the platform columns of a thread row.
pub fn set_platform(row: &mut thread::ActiveModel, platform: Option<&PlatformThread>) {
    row.on_platform = Set(platform.is_some());
    row.remote_id = Set(platform.and_then(|p| p.remote_id.clone()));
    row.resolvable = Set(platform.is_some_and(|p| p.resolvable));
    row.resolved_by = Set(platform.and_then(|p| p.resolved_by.clone()));
    row.status_pending = Set(platform.is_some_and(|p| p.status_pending));
}

/// Sets the target columns; `anchor` is the row id of a `Range` target's anchor.
pub fn set_target(row: &mut thread::ActiveModel, target: &CommentTarget, anchor: Option<i64>) {
    let (kind, side, path) = match target {
        CommentTarget::Range(_) => (kinds::ThreadKind::Range, None, None),
        CommentTarget::File { side, path } => (kinds::ThreadKind::File, Some((*side).into()), Some(path.0.clone())),
        CommentTarget::Changeset => (kinds::ThreadKind::Changeset, None, None),
    };
    row.kind = Set(kind);
    row.file_side = Set(side);
    row.file_path = Set(path);
    row.anchor = Set(anchor);
}

pub fn target_from(m: &thread::Model, anchor: Option<&anchor::Model>) -> Result<CommentTarget, DbErr> {
    match m.kind {
        kinds::ThreadKind::Range => Ok(CommentTarget::Range(anchor_from(
            anchor.ok_or_else(|| corrupt("range thread without anchor"))?,
        )?)),
        kinds::ThreadKind::File => Ok(CommentTarget::File {
            side: m.file_side.ok_or_else(|| corrupt("file thread without side"))?.into(),
            path: RepoPath(m.file_path.clone().ok_or_else(|| corrupt("file thread without path"))?),
        }),
        kinds::ThreadKind::Changeset => Ok(CommentTarget::Changeset),
    }
}
