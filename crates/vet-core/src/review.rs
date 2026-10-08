//! Anchors, review state, threads (SPEC §5). Content-anchored, never line-anchored (I3).

use crate::content::{BlobId, FileDiff, Hash, Hunk, LineRange, RepoPath, SnapshotId};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Side {
    Base,
    Target,
}

/// The two sides of `hunk` in `diff`: the lines it covers on each, and the path they are in
/// (`None` on the side the file does not exist on).
pub fn sides<'a>(diff: &'a FileDiff, hunk: &Hunk) -> [(Side, Option<&'a RepoPath>, LineRange); 2] {
    [
        (Side::Base, diff.old_path.as_ref(), hunk.old),
        (Side::Target, diff.new_path.as_ref(), hunk.new),
    ]
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Anchor {
    pub side: Side,
    pub snapshot: SnapshotId,
    pub file: RepoPath,
    pub range: LineRange,
    pub content_hash_raw: Hash,
    /// LF line endings, trailing whitespace stripped.
    pub content_hash_norm: Hash,
    pub context_hashes: Vec<Hash>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Location {
    pub file: RepoPath,
    pub range: LineRange,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strategy {
    ContextWindow,
    CrossFileMove,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StaleReason {
    ContextChanged,
    FileDeleted,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AnchorState {
    Exact {
        whitespace_changed: bool,
    },
    Inferred {
        strategy: Strategy,
        confidence: Confidence,
    },
    Stale {
        reason: StaleReason,
    },
    /// Duplicate matches; distinct from `Stale`.
    Ambiguous {
        candidates: Vec<Location>,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CommentTarget {
    Range(Anchor),
    File { side: Side, path: RepoPath },
    Changeset,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ThreadId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MessageId(pub u64);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Message {
    /// Assigned by the store; `MessageId(0)` for a message that is not stored yet.
    pub id: MessageId,
    pub author: String,
    pub body: String,
    /// Unix seconds.
    pub at: i64,
    /// The viewer wrote it, here or on the platform: it can be edited and removed.
    pub mine: bool,
    /// The platform's picture of the author, if it has one.
    pub avatar: Option<String>,
    /// Unix seconds of the viewer's last edit of the text; `None` if it was never edited.
    pub edited_at: Option<i64>,
    /// Present once the platform has the comment. A message without it is only in Vet: a draft
    /// on a changeset of a platform.
    pub remote: Option<RemoteComment>,
}

impl Message {
    /// A message the viewer wrote, not stored or posted yet.
    pub fn written(author: String, body: String, at: i64) -> Self {
        Self {
            id: MessageId(0),
            author,
            body,
            at,
            mine: true,
            avatar: None,
            edited_at: None,
            remote: None,
        }
    }

    /// The text the platform is known to have, or `None` while it has no such comment.
    pub fn synced(&self) -> Option<&str> {
        self.remote.as_ref().map(|r| r.synced.as_str())
    }

    /// The viewer changed the text of a posted comment after the last sync.
    pub fn edit_pending(&self) -> bool {
        self.synced().is_some_and(|synced| synced != self.body)
    }
}

/// A comment as the platform has it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RemoteComment {
    /// The platform's own id.
    pub id: String,
    /// The text the platform had at the last sync. What the viewer's edit is measured against:
    /// an edit is pending while the text differs from it.
    pub synced: String,
    /// Set when the viewer and the platform both changed the text since the last sync.
    pub conflict: Option<EditConflict>,
}

/// The platform's text of a comment that was edited in both places, kept beside the viewer's
/// until one of them is chosen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EditConflict {
    pub remote_body: String,
    /// Unix seconds the platform's edit was made.
    pub remote_at: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreadStatus {
    Open,
    Resolved,
}

/// What a thread is on a platform.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlatformThread {
    /// The platform's id of the thread; `None` while it is a draft.
    pub remote_id: Option<String>,
    /// A flat conversation comment cannot be resolved.
    pub resolvable: bool,
    /// Who resolved it on the platform.
    pub resolved_by: Option<String>,
    /// The viewer resolved or reopened it after the last sync, which then wins over the platform's
    /// state.
    pub status_pending: bool,
}

impl PlatformThread {
    /// A thread the viewer is about to post.
    pub fn draft(resolvable: bool) -> Self {
        Self {
            remote_id: None,
            resolvable,
            resolved_by: None,
            status_pending: false,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Thread {
    pub id: ThreadId,
    pub target: CommentTarget,
    pub messages: Vec<Message>,
    pub status: ThreadStatus,
    /// `None` for a thread on a changeset that is not on a platform.
    pub platform: Option<PlatformThread>,
}

impl Thread {
    /// The platform has the thread.
    pub fn is_posted(&self) -> bool {
        self.platform.as_ref().is_some_and(|p| p.remote_id.is_some())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanState {
    Seen,
    SeenAgainstOldBase,
}

/// The durable unit of review coverage (§5.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReviewedSpan {
    /// `None` for pure additions.
    pub base: Option<Anchor>,
    /// `None` for pure deletions.
    pub target: Option<Anchor>,
    pub state: SpanState,
}

/// One file on one side of a changeset, from the snapshot an anchor or span was
/// recorded in (`old_*`) to the snapshot it is resolved against (`new_*`). An
/// unchanged file has equal blobs and paths. Callers apply git-detected renames.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileRemapInput {
    pub old_path: Option<RepoPath>,
    pub new_path: Option<RepoPath>,
    pub old_blob: Option<BlobId>,
    pub new_blob: Option<BlobId>,
}

/// The line of the target text that line `line` of `side` faces in a unified diff: itself on the
/// target side; on the base side the last target line of the hunk that deleted or replaced it
/// (where a comment on it is shown, below its replacement), else the line it shifted to. Lines
/// deleted at the end of the file face one past the target's last line.
pub fn target_line(hunks: &[Hunk], side: Side, line: u32) -> u32 {
    if side == Side::Target {
        return line;
    }
    let mut shift = 0i64;
    for h in hunks {
        if line < h.old.start {
            break;
        }
        if line < h.old.end {
            return h.new.start + h.new.len().max(1) - 1;
        }
        shift += i64::from(h.new.len()) - i64::from(h.old.len());
    }
    u32::try_from((i64::from(line) + shift).max(0)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(old: (u32, u32), new: (u32, u32)) -> Hunk {
        Hunk {
            old: LineRange {
                start: old.0,
                end: old.1,
            },
            new: LineRange {
                start: new.0,
                end: new.1,
            },
        }
    }

    #[test]
    fn a_hunk_has_its_old_lines_on_the_base_and_its_new_ones_on_the_target() {
        let path = |s: &str| Some(RepoPath::from(s));
        let hunk = hunk((2, 4), (2, 7));
        let renamed = FileDiff {
            old_path: path("a"),
            new_path: path("b"),
            old_blob: None,
            new_blob: None,
            hunks: vec![hunk],
        };
        assert_eq!(
            sides(&renamed, &hunk),
            [
                (Side::Base, path("a").as_ref(), hunk.old),
                (Side::Target, path("b").as_ref(), hunk.new)
            ]
        );
        let added = FileDiff {
            old_path: None,
            ..renamed
        };
        assert_eq!(sides(&added, &hunk)[0].1, None);
    }

    #[test]
    fn the_target_side_faces_itself() {
        assert_eq!(target_line(&[hunk((10, 12), (10, 11))], Side::Target, 7), 7);
        assert_eq!(target_line(&[hunk((0, 5), (0, 0))], Side::Target, 3), 3);
    }

    #[test]
    fn a_base_line_faces_the_hunk_it_was_changed_in_or_where_it_shifted_to() {
        // Lines 10..12 of the base became one target line at 10; two lines were added at 17.
        let hunks = [hunk((10, 12), (10, 11)), hunk((18, 18), (17, 19))];
        assert_eq!(target_line(&hunks, Side::Base, 5), 5);
        // Inside the replaced region: its last target line.
        assert_eq!(target_line(&hunks, Side::Base, 10), 10);
        assert_eq!(target_line(&hunks, Side::Base, 11), 10);
        // One line fewer after the first hunk, and two more after the second.
        assert_eq!(target_line(&hunks, Side::Base, 12), 11);
        assert_eq!(target_line(&hunks, Side::Base, 14), 13);
        assert_eq!(target_line(&hunks, Side::Base, 30), 31);
    }

    #[test]
    fn a_replaced_region_faces_the_end_of_its_replacement() {
        let hunks = [hunk((4, 6), (4, 9))];
        assert_eq!(
            (4..6).map(|l| target_line(&hunks, Side::Base, l)).collect::<Vec<_>>(),
            [8, 8]
        );
        assert_eq!(target_line(&hunks, Side::Base, 6), 9);
    }

    #[test]
    fn deleted_lines_face_the_line_that_took_their_place() {
        let hunks = [hunk((5, 8), (5, 5))];
        assert_eq!(target_line(&hunks, Side::Base, 6), 5);
        assert_eq!(target_line(&hunks, Side::Base, 8), 5);
        assert_eq!(target_line(&hunks, Side::Base, 0), 0);
        assert_eq!(target_line(&[hunk((0, 2), (0, 0))], Side::Base, 1), 0);
    }

    #[test]
    fn an_insertion_moves_the_lines_below_it_but_not_those_above() {
        let hunks = [hunk((3, 3), (3, 5))];
        assert_eq!(target_line(&hunks, Side::Base, 2), 2);
        assert_eq!(target_line(&hunks, Side::Base, 3), 5);
    }

    #[test]
    fn without_hunks_a_base_line_faces_itself() {
        assert_eq!(target_line(&[], Side::Base, 9), 9);
    }
}
