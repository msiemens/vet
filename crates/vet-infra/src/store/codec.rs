//! Mapping between `vet-core` review types and entity rows. Isolated so a
//! change to anchors, spans or entries touches only this file.

use sea_orm::sea_query::{SimpleExpr, Value};
use sea_orm::{ColumnTrait, DbErr, Set};
use vet_app::review::NonTextAck;
use vet_core::content::{
    BlobId, EntryKind, FileEntry, Hash, LineRange, NonTextKind, RepoPath, SnapshotId, SnapshotStatus,
};
use vet_core::merkle::TreeNode;
use vet_core::review::{Anchor, CommentTarget, Side, SpanState, ThreadStatus};

use super::entity::{ack, anchor, kinds, snapshot_path, thread, tree_child};

fn corrupt(what: &str) -> DbErr {
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

pub fn anchor_row(session: i64, a: &Anchor) -> anchor::ActiveModel {
    anchor::ActiveModel {
        session: Set(session),
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

pub fn ack_row(session: i64, a: &NonTextAck) -> ack::ActiveModel {
    let (b, f) = (AckSide::new(a.before.as_ref()), AckSide::new(a.after.as_ref()));
    ack::ActiveModel {
        session: Set(session),
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
