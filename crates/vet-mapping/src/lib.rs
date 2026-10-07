//! vet-app views to protocol DTOs, and back for the request enums.

pub mod account;
pub mod progress;
pub mod semantic;

use vet_app::changeset::{BaseChoice, CommitId, Source, VersionOrigin};
use vet_app::ports::{BaseCandidates, CommitInfo, Degraded};
use vet_app::session::{
    BaseView, ChangesetList, CommentAt, CommentSpot, DirEntryKind, DirListing, EntryInfo, FileStatus, FileView,
    ItemBlob, ItemView, Lifecycle, ManifestView, ModeChange, NewThread, NoText, ReviewUpdateView, ReviewView,
    RowReview, TargetText, ThreadView, ThreadsView, VersionView, View,
};
use vet_core::completion::{Completion, HunkState};
use vet_core::content::{EntryKind, Hunk, LineDelta, LineRange, NonTextKind, RepoPath};
use vet_core::filter::ReviewFilter;
use vet_core::inline::TextChange;
use vet_core::review::{AnchorState, CommentTarget, Confidence, Location, Side, StaleReason, Strategy, ThreadStatus};
use vet_protocol as p;
use vet_semantic::ports::RestoreFailure;

fn completion(c: &Completion) -> p::Completion {
    p::Completion {
        lines: c.lines,
        seen_lines: c.seen_lines,
        old_base_lines: c.old_base_lines,
        hunks: c.hunks,
        seen_hunks: c.seen_hunks,
        items: c.items,
        acknowledged_items: c.acknowledged_items,
        complete: c.is_complete(),
        seen_percent: c.seen_percent(),
        old_base_percent: c.old_base_percent(),
    }
}

pub fn non_text_kind(kind: NonTextKind) -> p::NonTextKind {
    match kind {
        NonTextKind::Mode => p::NonTextKind::Mode,
        NonTextKind::Symlink => p::NonTextKind::Symlink,
        NonTextKind::Submodule => p::NonTextKind::Submodule,
        NonTextKind::Binary => p::NonTextKind::Binary,
        NonTextKind::ContentUnavailable => p::NonTextKind::ContentUnavailable,
        NonTextKind::Rename => p::NonTextKind::Rename,
        NonTextKind::Empty => p::NonTextKind::Empty,
    }
}

pub fn core_non_text_kind(kind: p::NonTextKind) -> NonTextKind {
    match kind {
        p::NonTextKind::Mode => NonTextKind::Mode,
        p::NonTextKind::Symlink => NonTextKind::Symlink,
        p::NonTextKind::Submodule => NonTextKind::Submodule,
        p::NonTextKind::Binary => NonTextKind::Binary,
        p::NonTextKind::ContentUnavailable => NonTextKind::ContentUnavailable,
        p::NonTextKind::Rename => NonTextKind::Rename,
        p::NonTextKind::Empty => NonTextKind::Empty,
    }
}

fn hunk_state(state: HunkState) -> p::HunkState {
    match state {
        HunkState::Unseen => p::HunkState::Unseen,
        HunkState::Seen => p::HunkState::Seen,
        HunkState::SeenAgainstOldBase => p::HunkState::SeenAgainstOldBase,
        HunkState::Informational => p::HunkState::Informational,
        HunkState::Unapplied => p::HunkState::Unapplied,
    }
}

pub fn core_view(view: p::View) -> View {
    match view {
        p::View::Changeset => View::Changeset,
        p::View::Interdiff { from, to } => View::Interdiff { from, to },
    }
}

pub fn versions(versions: &[VersionView]) -> p::Response {
    p::Response::Versions {
        versions: versions
            .iter()
            .map(|v| p::VersionInfo {
                n: v.n,
                at: unix_secs(v.at),
                origin: match v.origin {
                    VersionOrigin::Settled => p::VersionOrigin::Settled,
                    VersionOrigin::BaseMoved => p::VersionOrigin::BaseMoved,
                    VersionOrigin::Observed => p::VersionOrigin::Observed,
                },
                tip: v.tip.as_ref().map(commit_info),
                base: v.base.as_ref().map(commit_info),
                reviewed_here: v.reviewed_here,
                files: (v.change.files.iter())
                    .map(|f| p::VersionFile {
                        path: f.path.to_string_lossy(),
                        delta: line_delta(f.delta),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn hunk(h: &Hunk) -> p::Hunk {
    p::Hunk {
        base_start: h.old.start,
        base_len: h.old.len(),
        target_start: h.new.start,
        target_len: h.new.len(),
    }
}

fn line_delta(d: LineDelta) -> p::LineDelta {
    p::LineDelta {
        added: d.added,
        removed: d.removed,
    }
}

pub fn text_change(c: &TextChange) -> p::TextChange {
    p::TextChange {
        from_base: c.from_base,
        to_base: c.to_base,
        from_target: c.from_target,
        to_target: c.to_target,
    }
}

fn status(s: FileStatus) -> p::FileStatus {
    match s {
        FileStatus::Added => p::FileStatus::Added,
        FileStatus::Deleted => p::FileStatus::Deleted,
        FileStatus::Modified => p::FileStatus::Modified,
        FileStatus::Renamed => p::FileStatus::Renamed,
    }
}

fn entry(info: &EntryInfo) -> p::EntryDesc {
    p::EntryDesc {
        kind: match info.entry.kind {
            EntryKind::File => p::EntryKind::File,
            EntryKind::Symlink => p::EntryKind::Symlink,
            EntryKind::Submodule => p::EntryKind::Submodule,
        },
        mode: info.entry.mode,
        target: info.target.clone(),
        size: info.size.map(sat_u32),
    }
}

fn item(i: &ItemView) -> p::NonTextItem {
    p::NonTextItem {
        kind: non_text_kind(i.kind),
        before: i.before.as_ref().map(entry),
        after: i.after.as_ref().map(entry),
        has_preview: i.has_preview,
        mode_change: i.mode_change.map(|c| match c {
            ModeChange::GainedExecutable => p::ModeChange::GainedExecutable,
            ModeChange::LostExecutable => p::ModeChange::LostExecutable,
            ModeChange::Other => p::ModeChange::Other,
        }),
    }
}

fn review(r: &ReviewView) -> p::FileReview {
    p::FileReview {
        hunks: r.hunks.iter().copied().map(hunk_state).collect(),
        acknowledged: r.acknowledged.iter().copied().map(non_text_kind).collect(),
    }
}

fn file(f: &FileView) -> p::FileChange {
    p::FileChange {
        path: f.path.to_string_lossy(),
        old_path: f.old_path.as_ref().map(RepoPath::to_string_lossy),
        status: status(f.status),
        base_lines: f.base_lines,
        target_lines: f.target_lines,
        hunks: f.hunks.iter().map(hunk).collect(),
        delta: line_delta(f.delta),
        non_text: f.non_text.iter().map(item).collect(),
        review: review(&f.review),
        completion: completion(&f.completion),
    }
}

pub fn core_base_choice(choice: p::BaseChoice) -> BaseChoice {
    match choice {
        p::BaseChoice::Branch { name } => BaseChoice::Branch { name },
        p::BaseChoice::MergeBase { reference } => BaseChoice::MergeBase { reference },
        p::BaseChoice::Commit { id } => BaseChoice::Commit { id: CommitId(id) },
    }
}

pub fn core_filter(f: p::ReviewFilter) -> ReviewFilter {
    ReviewFilter {
        roots: f.roots.into_iter().map(RepoPath::from).collect(),
        include: f.include,
        exclude: f.exclude,
        hide_binary: f.hide_binary,
    }
}

fn filter(f: &ReviewFilter) -> p::ReviewFilter {
    p::ReviewFilter {
        roots: f.roots.iter().map(RepoPath::to_string_lossy).collect(),
        include: f.include.clone(),
        exclude: f.exclude.clone(),
        hide_binary: f.hide_binary,
    }
}

fn commit_info(c: &CommitInfo) -> p::CommitInfo {
    p::CommitInfo {
        id: c.id.0.clone(),
        short_id: c.id.short().to_owned(),
        subject: c.subject.clone(),
        author: c.author.clone(),
        time: unix_secs(c.time),
    }
}

/// `n`, or the largest `u32` if it does not fit.
pub fn sat_u32<T: TryInto<u32>>(n: T) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

fn unix_secs(time: i64) -> u32 {
    u32::try_from(time).unwrap_or(0)
}

pub fn base(b: &BaseView) -> p::Base {
    p::Base {
        choice: match &b.choice {
            BaseChoice::Branch { name } => p::BaseChoice::Branch { name: name.clone() },
            BaseChoice::MergeBase { reference } => p::BaseChoice::MergeBase {
                reference: reference.clone(),
            },
            BaseChoice::Commit { id } => p::BaseChoice::Commit { id: id.0.clone() },
        },
        commit: commit_info(&b.commit),
        name: b.name.clone(),
        behind: b.behind,
    }
}

pub fn review_info(l: Lifecycle) -> p::ReviewInfo {
    match l {
        Lifecycle::Browsing => p::ReviewInfo::Browsing,
        Lifecycle::Open { version, updated_at } => p::ReviewInfo::Open {
            version,
            updated_at: unix_secs(updated_at),
        },
    }
}

pub fn base_candidates(c: &BaseCandidates) -> p::Response {
    p::Response::BaseCandidates {
        branches: c
            .branches
            .iter()
            .map(|b| p::BranchInfo {
                name: b.name.clone(),
                commit: b.commit.0.clone(),
                current: b.current,
                default: b.default,
            })
            .collect(),
        commits: c.commits.iter().map(commit_info).collect(),
    }
}

pub fn changeset_key(source: &Source) -> p::ChangesetKey {
    match source {
        Source::WorkingTree => p::ChangesetKey::WorkingTree,
        Source::Branch { name } => p::ChangesetKey::Branch { name: name.clone() },
        // Not offered over the protocol yet; a pull request is shown as the branch it comes from.
        Source::PullRequest { .. } => p::ChangesetKey::WorkingTree,
    }
}

pub fn core_changeset_key(key: p::ChangesetKey) -> Source {
    match key {
        p::ChangesetKey::WorkingTree => Source::WorkingTree,
        p::ChangesetKey::Branch { name } => Source::Branch { name },
    }
}

pub fn changesets(list: &ChangesetList) -> p::Response {
    p::Response::Changesets {
        on_view: changeset_key(&list.on_view),
        rows: list
            .rows
            .iter()
            .map(|r| p::ChangesetRow {
                key: changeset_key(&r.source),
                title: r.title.clone(),
                checked_out: r.checked_out,
                divergence: r.divergence.map(|d| p::Divergence {
                    ahead: d.ahead,
                    behind: d.behind,
                }),
                last_commit: r.last_commit.map(unix_secs),
                review: match r.review {
                    RowReview::None => p::RowReview::None,
                    RowReview::Open { version } => p::RowReview::Open { version },
                    RowReview::Finished { complete } => p::RowReview::Finished { complete },
                },
            })
            .collect(),
    }
}

pub fn manifest(project: p::ProjectId, m: &ManifestView, restore_failures: &[RestoreFailure]) -> p::Manifest {
    p::Manifest {
        project,
        revision: m.revision,
        name: m.name.clone(),
        review: review_info(m.lifecycle),
        base: base(&m.base),
        target: p::Target {
            changeset: changeset_key(&m.target.source),
            name: m.target.name.clone(),
            branch: m.target.branch.clone(),
        },
        filter: filter(&m.filter),
        files: m.files.iter().map(file).collect(),
        delta: line_delta(m.delta),
        completion: completion(&m.completion),
        excluded_lines: m.completion.excluded_lines,
        excluded_items: m.completion.excluded_items,
        degraded: m.degraded.map(|d| match d {
            Degraded::MountedWindowsDrive => p::WatchDegraded::MountedWindowsDrive,
            Degraded::WatchLimit => p::WatchDegraded::WatchLimit,
        }),
        restore_failures: semantic::restore_failures(restore_failures),
        interdiff: m.interdiff.as_ref().map(|i| p::InterdiffInfo {
            from: i.from,
            to: i.to,
            rebased: i.rebased,
            markable: i.markable,
            limits: i.limits.clone(),
        }),
    }
}

pub fn directory(l: &DirListing) -> p::Response {
    p::Response::Directory {
        revision: l.revision,
        entries: l
            .entries
            .iter()
            .map(|e| p::DirEntry {
                name: e.name.clone(),
                kind: match e.kind {
                    DirEntryKind::File => p::DirEntryKind::File,
                    DirEntryKind::Dir => p::DirEntryKind::Dir,
                    DirEntryKind::Symlink => p::DirEntryKind::Symlink,
                    DirEntryKind::Submodule => p::DirEntryKind::Submodule,
                },
                changed: e.changed,
                excluded: e.excluded,
            })
            .collect(),
    }
}

pub fn target_text(t: &TargetText, highlights: p::Highlights) -> p::Response {
    p::Response::FileText {
        revision: t.revision,
        content: match &t.content {
            Ok(text) => p::TextContent::Text {
                text: text.clone(),
                highlights,
            },
            Err(reason) => p::TextContent::NoText {
                reason: match reason {
                    NoText::Binary => p::NoTextReason::Binary,
                    NoText::Unavailable => p::NoTextReason::Unavailable,
                    NoText::NotAFile => p::NoTextReason::NotAFile,
                },
            },
        },
    }
}

pub fn item_blob(blob: ItemBlob) -> p::Response {
    use base64::Engine;
    let content = match blob {
        ItemBlob::Bytes { media_type, bytes } => p::BlobContent::Bytes {
            media_type: media_type.map(String::from),
            bytes: base64::engine::general_purpose::STANDARD.encode(bytes),
        },
        ItemBlob::Absent => p::BlobContent::Absent,
        ItemBlob::TooLarge => p::BlobContent::TooLarge,
        ItemBlob::Unavailable => p::BlobContent::Unavailable,
    };
    p::Response::ItemBlob { content }
}

fn side(s: Side) -> p::Side {
    match s {
        Side::Base => p::Side::Base,
        Side::Target => p::Side::Target,
    }
}

pub fn core_side(s: p::Side) -> Side {
    match s {
        p::Side::Base => Side::Base,
        p::Side::Target => Side::Target,
    }
}

fn line_location(file: &RepoPath, range: LineRange) -> p::LineLocation {
    p::LineLocation {
        path: file.to_string_lossy(),
        start: range.start,
        len: range.len(),
    }
}

fn location(l: &Location) -> p::LineLocation {
    line_location(&l.file, l.range)
}

pub fn new_thread(t: p::ThreadTarget) -> NewThread {
    match t {
        p::ThreadTarget::Range { side, path, start, len } => NewThread::Range {
            side: core_side(side),
            path: RepoPath::from(path),
            range: LineRange {
                start,
                end: start.saturating_add(len),
            },
        },
        p::ThreadTarget::File { side, path } => NewThread::File {
            side: core_side(side),
            path: RepoPath::from(path),
        },
        p::ThreadTarget::Changeset => NewThread::Changeset,
    }
}

pub fn core_comment_at(at: p::CommentAt) -> CommentAt {
    match at {
        p::CommentAt::File => CommentAt::File,
        p::CommentAt::Hunk { index } => CommentAt::Hunk(index),
        p::CommentAt::Lines { side, start, len } => CommentAt::Lines {
            side: core_side(side),
            start,
            len,
        },
    }
}

pub fn comment_spot(spot: &CommentSpot) -> p::Response {
    let target = match &spot.target {
        NewThread::Range { side: s, path, range } => p::ThreadTarget::Range {
            side: side(*s),
            path: path.to_string_lossy(),
            start: range.start,
            len: range.len(),
        },
        NewThread::File { side: s, path } => p::ThreadTarget::File {
            side: side(*s),
            path: path.to_string_lossy(),
        },
        NewThread::Changeset => p::ThreadTarget::Changeset,
    };
    p::Response::CommentTarget {
        target,
        line: spot.line,
    }
}

pub fn core_thread_status(s: p::ThreadStatus) -> ThreadStatus {
    match s {
        p::ThreadStatus::Open => ThreadStatus::Open,
        p::ThreadStatus::Resolved => ThreadStatus::Resolved,
    }
}

fn thread_status(s: ThreadStatus) -> p::ThreadStatus {
    match s {
        ThreadStatus::Open => p::ThreadStatus::Open,
        ThreadStatus::Resolved => p::ThreadStatus::Resolved,
    }
}

fn anchor_state(s: &AnchorState) -> p::AnchorState {
    match s {
        AnchorState::Exact { whitespace_changed } => p::AnchorState::Exact {
            whitespace_changed: *whitespace_changed,
        },
        AnchorState::Inferred { strategy, confidence } => p::AnchorState::Inferred {
            strategy: match strategy {
                Strategy::ContextWindow => p::AnchorStrategy::ContextWindow,
                Strategy::CrossFileMove => p::AnchorStrategy::CrossFileMove,
            },
            confidence: match confidence {
                Confidence::Low => p::AnchorConfidence::Low,
                Confidence::Medium => p::AnchorConfidence::Medium,
                Confidence::High => p::AnchorConfidence::High,
            },
        },
        AnchorState::Stale { reason } => p::AnchorState::Stale {
            reason: match reason {
                StaleReason::ContextChanged => p::StaleReason::ContextChanged,
                StaleReason::FileDeleted => p::StaleReason::FileDeleted,
            },
        },
        AnchorState::Ambiguous { candidates } => p::AnchorState::Ambiguous {
            candidates: candidates.iter().map(location).collect(),
        },
    }
}

pub fn thread(t: &ThreadView) -> p::Thread {
    let (kind, side_of, original) = match &t.target {
        CommentTarget::Range(a) => (
            p::ThreadKind::Range,
            Some(side(a.side)),
            Some(line_location(&a.file, a.range)),
        ),
        CommentTarget::File { side: s, .. } => (p::ThreadKind::File, Some(side(*s)), None),
        CommentTarget::Changeset => (p::ThreadKind::Changeset, None, None),
    };
    p::Thread {
        id: sat_u32(t.id.0),
        kind,
        side: side_of,
        path: t.path.as_ref().map(RepoPath::to_string_lossy),
        original,
        location: t.location.as_ref().map(location),
        target_line: t.target_line,
        anchor: t.state.as_ref().map(anchor_state),
        messages: t
            .messages
            .iter()
            .map(|m| p::ThreadMessage {
                author: m.author.clone(),
                body: m.body.clone(),
                at: unix_secs(m.at),
            })
            .collect(),
        status: thread_status(t.status),
    }
}

pub fn threads(v: &ThreadsView) -> p::Response {
    p::Response::Threads {
        revision: v.revision,
        threads: v.threads.iter().map(thread).collect(),
    }
}

pub fn review_update(u: &ReviewUpdateView) -> p::ReviewUpdate {
    p::ReviewUpdate {
        path: u.path.to_string_lossy(),
        review: review(&u.review),
        completion: completion(&u.completion),
        changeset: completion(&u.changeset),
    }
}
