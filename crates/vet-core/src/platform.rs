//! Platform-neutral description of a code host (SPEC §11). Core and the UI
//! branch on these properties, never on a platform's name.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangesetThreads {
    Resolvable,
    Flat,
    Either,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Posting {
    PerComment,
    BatchedReview,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockingSource {
    UnresolvedThreads,
    OpenTasks,
    Labels,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlatformTraits {
    pub changeset_threads: ChangesetThreads,
    pub conversation_feed: bool,
    pub posting: Posting,
    /// The platform's own verdict choices, in its order.
    pub verdicts: Vec<String>,
    pub verdict_separate: bool,
    pub blocking: Vec<BlockingSource>,
    pub revision_scoped: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlatformThread {
    pub remote_id: String,
    /// Shown as-is, e.g. ADO "Won't fix".
    pub remote_status: Option<String>,
    /// `false` for a flat conversation comment.
    pub resolvable: bool,
    /// As reported by the platform.
    pub blocks_merge: bool,
}
