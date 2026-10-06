//! Mapping between `vet-core` review types and entity rows. Isolated so a
//! change to anchors, spans or entries touches only this file.

use sea_orm::sea_query::{SimpleExpr, Value};
use sea_orm::{ColumnTrait, DbErr, Set};
use vet_app::review::NonTextAck;
use vet_core::content::{BlobId, EntryKind, FileEntry, Hash, LineRange, RepoPath, SnapshotId};
use vet_core::review::{Anchor, CommentTarget, Side, SpanState, ThreadStatus};

use super::entity::{ack, anchor, snapshot_entry, thread};

fn corrupt(what: &str) -> DbErr {
    DbErr::Custom(format!("corrupt row: {what}"))
}

pub fn hash(bytes: &[u8]) -> Result<Hash, DbErr> {
    <[u8; 32]>::try_from(bytes).map(Hash).map_err(|_| corrupt("hash is not 32 bytes"))
}

pub fn side_to(side: Side) -> i64 {
    match side {
        Side::Base => 0,
        Side::Target => 1,
    }
}

pub fn side_from(v: i64) -> Result<Side, DbErr> {
    match v {
        0 => Ok(Side::Base),
        1 => Ok(Side::Target),
        _ => Err(corrupt("side")),
    }
}

pub fn kind_to(kind: EntryKind) -> i64 {
    match kind {
        EntryKind::File => 0,
        EntryKind::Symlink => 1,
        EntryKind::Submodule => 2,
    }
}

fn kind_from(v: i64) -> Result<EntryKind, DbErr> {
    match v {
        0 => Ok(EntryKind::File),
        1 => Ok(EntryKind::Symlink),
        2 => Ok(EntryKind::Submodule),
        _ => Err(corrupt("entry kind")),
    }
}

pub const OPEN: i64 = 0;

pub fn status_to(status: ThreadStatus) -> i64 {
    match status {
        ThreadStatus::Open => OPEN,
        ThreadStatus::Resolved => 1,
    }
}

pub fn status_from(v: i64) -> Result<ThreadStatus, DbErr> {
    match v {
        OPEN => Ok(ThreadStatus::Open),
        1 => Ok(ThreadStatus::Resolved),
        _ => Err(corrupt("thread status")),
    }
}

pub fn span_state_to(state: SpanState) -> i64 {
    match state {
        SpanState::Seen => 0,
        SpanState::SeenAgainstOldBase => 1,
    }
}

pub fn span_state_from(v: i64) -> Result<SpanState, DbErr> {
    match v {
        0 => Ok(SpanState::Seen),
        1 => Ok(SpanState::SeenAgainstOldBase),
        _ => Err(corrupt("span state")),
    }
}

pub fn anchor_row(session: i64, a: &Anchor) -> anchor::ActiveModel {
    anchor::ActiveModel {
        session: Set(session),
        side: Set(side_to(a.side)),
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
        side: side_from(m.side)?,
        snapshot: SnapshotId(hash(&m.snapshot)?),
        file: RepoPath(m.file.clone()),
        range: LineRange { start: u32::try_from(m.start).map_err(|_| corrupt("range"))?, end: u32::try_from(m.end).map_err(|_| corrupt("range"))? },
        content_hash_raw: hash(&m.raw)?,
        content_hash_norm: hash(&m.norm)?,
        context_hashes: m.context.chunks(32).map(hash).collect::<Result<_, _>>()?,
    })
}

pub fn entry_row(snapshot: &[u8], path: &RepoPath, e: &FileEntry) -> snapshot_entry::ActiveModel {
    snapshot_entry::ActiveModel {
        snapshot: Set(snapshot.to_vec()),
        path: Set(path.0.clone()),
        blob: Set(e.blob.0.0.to_vec()),
        mode: Set(e.mode.into()),
        kind: Set(kind_to(e.kind)),
        unfiltered: Set(e.unfiltered),
    }
}

pub fn entry_from(m: snapshot_entry::Model) -> Result<(RepoPath, FileEntry), DbErr> {
    let entry = FileEntry { blob: BlobId(hash(&m.blob)?), mode: u32::try_from(m.mode).map_err(|_| corrupt("mode"))?, kind: kind_from(m.kind)?, unfiltered: m.unfiltered };
    Ok((RepoPath(m.path), entry))
}

/// The flattened columns of one side of an acknowledgement.
#[derive(Default)]
pub struct AckSide {
    path: Option<Vec<u8>>,
    blob: Option<Vec<u8>>,
    mode: Option<i64>,
    kind: Option<i64>,
    unfiltered: Option<bool>,
}

impl AckSide {
    fn new(entry: Option<&(RepoPath, FileEntry)>) -> Self {
        let Some((path, e)) = entry else { return Self::default() };
        Self { path: Some(path.0.clone()), blob: Some(e.blob.0.0.to_vec()), mode: Some(e.mode.into()), kind: Some(kind_to(e.kind)), unfiltered: Some(e.unfiltered) }
    }

    fn entry(self) -> Result<Option<(RepoPath, FileEntry)>, DbErr> {
        let (Some(path), Some(blob), Some(mode), Some(kind), Some(unfiltered)) = (self.path, self.blob, self.mode, self.kind, self.unfiltered) else {
            return Ok(None);
        };
        let entry = FileEntry { blob: BlobId(hash(&blob)?), mode: u32::try_from(mode).map_err(|_| corrupt("mode"))?, kind: kind_from(kind)?, unfiltered };
        Ok(Some((RepoPath(path), entry)))
    }
}

pub fn ack_row(session: i64, a: &NonTextAck) -> ack::ActiveModel {
    let (b, f) = (AckSide::new(a.before.as_ref()), AckSide::new(a.after.as_ref()));
    ack::ActiveModel {
        session: Set(session),
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
    let before = AckSide { path: m.before_path, blob: m.before_blob, mode: m.before_mode, kind: m.before_kind, unfiltered: m.before_unfiltered };
    let after = AckSide { path: m.after_path, blob: m.after_blob, mode: m.after_mode, kind: m.after_kind, unfiltered: m.after_unfiltered };
    Ok(NonTextAck { before: before.entry()?, after: after.entry()? })
}

const RANGE: i64 = 0;
const FILE: i64 = 1;
const CHANGESET: i64 = 2;

/// Sets the target columns; `anchor` is the row id of a `Range` target's anchor.
pub fn set_target(row: &mut thread::ActiveModel, target: &CommentTarget, anchor: Option<i64>) {
    let (kind, side, path) = match target {
        CommentTarget::Range(_) => (RANGE, None, None),
        CommentTarget::File { side, path } => (FILE, Some(side_to(*side)), Some(path.0.clone())),
        CommentTarget::Changeset => (CHANGESET, None, None),
    };
    row.kind = Set(kind);
    row.file_side = Set(side);
    row.file_path = Set(path);
    row.anchor = Set(anchor);
}

pub fn target_from(m: &thread::Model, anchor: Option<&anchor::Model>) -> Result<CommentTarget, DbErr> {
    match m.kind {
        RANGE => Ok(CommentTarget::Range(anchor_from(anchor.ok_or_else(|| corrupt("range thread without anchor"))?)?)),
        FILE => Ok(CommentTarget::File {
            side: side_from(m.file_side.ok_or_else(|| corrupt("file thread without side"))?)?,
            path: RepoPath(m.file_path.clone().ok_or_else(|| corrupt("file thread without path"))?),
        }),
        CHANGESET => Ok(CommentTarget::Changeset),
        _ => Err(corrupt("thread kind")),
    }
}
