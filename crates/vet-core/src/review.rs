//! Anchors, review state, threads (SPEC §5). Content-anchored, never line-anchored (I2).

use serde::{Deserialize, Serialize};

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
    /// The anchored lines were deleted; the anchor is where they were.
    Deleted,
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
    /// How far a draft is on its way to the platform; says nothing once `remote` is set, nor on a
    /// changeset outside a platform.
    pub outgoing: Outgoing,
}

/// Where a draft of the viewer's is on its way to the platform.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Outgoing {
    /// Waits for the viewer to post it.
    #[default]
    Draft,
    /// Handed to the platform at `since` (unix seconds) without Vet having heard back. Written
    /// before the platform is asked, so that a post Vet did not see through is looked for on the
    /// platform (posted after `since`, by the viewer, with the same text in the same place) before
    /// it is posted again.
    Posting { since: i64 },
    /// The platform refused it; it waits to be posted again.
    Failed { error: String },
}

/// One of the ways to close a review, in terms that hold for every platform (SPEC §7.2). The
/// adapters own how their platform spells one on the wire, and the UI owns the words shown. How
/// each reads: `Comment` "Comment", `Approve` "Approve", `RequestChanges` "Request changes",
/// `ApproveWithSuggestions` "Approve with suggestions", `WaitForAuthor` "Wait for author",
/// `Reject` "Reject", `Other` as the text it carries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Reviews without a side taken (a plain comment).
    Comment,
    Approve,
    /// A review that asks the author for changes.
    RequestChanges,
    /// An approval that points the author to follow-up work.
    ApproveWithSuggestions,
    /// An approval held back until the author has answered.
    WaitForAuthor,
    /// A review that says the pull request is not good to merge.
    Reject,
    /// A verdict whose spelling no adapter owns. It carries what Vet found spelled, is shown as
    /// that, and is spelled so again where it is kept, so the platform still knows it.
    Other(String),
}

impl Verdict {
    /// What the verdict reads as, where it is named.
    pub fn reads(&self) -> &str {
        match self {
            Verdict::Other(text) => text,
            Verdict::Comment => "Comment",
            Verdict::Approve => "Approve",
            Verdict::RequestChanges => "Request changes",
            Verdict::ApproveWithSuggestions => "Approve with suggestions",
            Verdict::WaitForAuthor => "Wait for author",
            Verdict::Reject => "Reject",
        }
    }

    /// The spell the verdict is kept under where Vet stores it itself (the vote that waits to be
    /// sent, a platform's verdict the viewer cast). Not the platform's wire spelling, which their
    /// adapters own; not a label, which the UI gives.
    pub fn spell(&self) -> &str {
        match self {
            Verdict::Other(text) => text,
            _ => self.kebab(),
        }
    }

    /// The verdict spelled that way, if Vet owns the spelling.
    pub fn of_spell(spelled: &str) -> Option<Verdict> {
        use Verdict::*;
        Some(match spelled {
            "comment" => Comment,
            "approve" => Approve,
            "request-changes" => RequestChanges,
            "approve-with-suggestions" => ApproveWithSuggestions,
            "wait-for-author" => WaitForAuthor,
            "reject" => Reject,
            _ => return None,
        })
    }

    fn kebab(&self) -> &'static str {
        match self {
            Verdict::Comment => "comment",
            Verdict::Approve => "approve",
            Verdict::RequestChanges => "request-changes",
            Verdict::ApproveWithSuggestions => "approve-with-suggestions",
            Verdict::WaitForAuthor => "wait-for-author",
            Verdict::Reject => "reject",
            Verdict::Other(_) => unreachable!("spelled as its own text"),
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reads())
    }
}

impl Serialize for Verdict {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.spell())
    }
}

impl<'de> Deserialize<'de> for Verdict {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Spell;
        impl<'de> serde::de::Visitor<'de> for Spell {
            type Value = Verdict;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a verdict")
            }
            fn visit_str<E: serde::de::Error>(self, spelled: &str) -> Result<Verdict, E> {
                Ok(Verdict::of_spell(spelled).unwrap_or_else(|| Verdict::Other(spelled.to_owned())))
            }
        }
        deserializer.deserialize_str(Spell)
    }
}

/// The vote of the viewer's on a pull request that waits to be sent, on a platform that keeps the
/// vote apart from posting. There is at most one per pull request, and it is on its way to the
/// platform like a draft: `outgoing` is never a comment's posted state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PendingVote {
    /// A verdict of the platform's ([`crate::review::Verdict`]).
    pub verdict: Verdict,
    pub outgoing: Outgoing,
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
            outgoing: Outgoing::Draft,
        }
    }

    /// The text the platform is known to have, or `None` while it has no such comment.
    pub fn synced(&self) -> Option<&str> {
        self.remote.as_ref().map(|r| r.synced.as_str())
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

    /// A comment on the whole pull request in the platform's flat conversation: it has no status
    /// and takes no replies.
    pub fn is_flat(&self) -> bool {
        self.target == CommentTarget::Changeset && self.platform.as_ref().is_some_and(|p| !p.resolvable)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanState {
    Seen,
    SeenAgainstOldBase,
    /// Not coverage: the lines are flagged for later review, independent of being seen.
    Flagged,
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
    fn a_verdict_is_spelled_back_as_itself_and_an_unknown_one_is_kept() {
        for (verdict, spelled, reads) in [
            (Verdict::Comment, "comment", "Comment"),
            (Verdict::Approve, "approve", "Approve"),
            (Verdict::RequestChanges, "request-changes", "Request changes"),
            (
                Verdict::ApproveWithSuggestions,
                "approve-with-suggestions",
                "Approve with suggestions",
            ),
            (Verdict::WaitForAuthor, "wait-for-author", "Wait for author"),
            (Verdict::Reject, "reject", "Reject"),
        ] {
            assert_eq!(verdict.spell(), spelled);
            assert_eq!(Verdict::of_spell(spelled), Some(verdict.clone()));
            assert_eq!(verdict.reads(), reads);
        }
        for spelled in ["approved", "REQUEST_CHANGES", "strange new verdict"] {
            assert_eq!(Verdict::of_spell(spelled), None);
        }
        assert_eq!(
            serde_json::from_str::<Verdict>("\"wait-for-author\"").unwrap(),
            Verdict::WaitForAuthor
        );
        assert_eq!(
            serde_json::from_str::<Verdict>("\"strange new verdict\"").unwrap(),
            Verdict::Other("strange new verdict".into())
        );
        assert_eq!(serde_json::to_string(&Verdict::Other("odd".into())).unwrap(), "\"odd\"");
    }
}
