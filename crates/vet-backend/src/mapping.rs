//! vet-app views to protocol DTOs, and back for the request enums.

use vet_app::pin::{BaseChoice, CommitId};
use vet_app::ports::{BaseCandidates, Degraded};
use vet_app::session::{
    BaseView, DirEntryKind, DirListing, EntryInfo, FileStatus, FileView, ItemView, ManifestView, NewThread, NoText,
    ReviewUpdateView, ReviewView, TargetText, ThreadView, ThreadsView, path_str,
};
use vet_core::completion::{Completion, HunkState};
use vet_core::content::{EntryKind, Hunk, LineRange, NonTextKind, RepoPath};
use vet_core::filter::ReviewFilter;
use vet_core::review::{AnchorState, CommentTarget, Confidence, Location, Side, StaleReason, Strategy, ThreadStatus};
use vet_protocol as p;

fn completion(c: &Completion) -> p::Completion {
    p::Completion {
        lines: c.lines,
        seen_lines: c.seen_lines,
        old_base_lines: c.old_base_lines,
        items: c.items,
        acknowledged_items: c.acknowledged_items,
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
    }
}

fn hunk_state(state: HunkState) -> p::HunkState {
    match state {
        HunkState::Unseen => p::HunkState::Unseen,
        HunkState::Seen => p::HunkState::Seen,
        HunkState::SeenAgainstOldBase => p::HunkState::SeenAgainstOldBase,
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
        size: info.size.map(|s| u32::try_from(s).unwrap_or(u32::MAX)),
    }
}

fn item(i: &ItemView) -> p::NonTextItem {
    p::NonTextItem {
        kind: non_text_kind(i.kind),
        before: i.before.as_ref().map(entry),
        after: i.after.as_ref().map(entry),
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
        path: path_str(&f.path),
        old_path: f.old_path.as_ref().map(path_str),
        status: status(f.status),
        base_lines: f.base_lines,
        target_lines: f.target_lines,
        hunks: f.hunks.iter().map(hunk).collect(),
        non_text: f.non_text.iter().map(item).collect(),
        review: review(&f.review),
        completion: completion(&f.completion),
    }
}

pub fn core_base_choice(choice: p::BaseChoice) -> BaseChoice {
    match choice {
        p::BaseChoice::Pin => BaseChoice::Pin,
        p::BaseChoice::MergeBase { reference } => BaseChoice::MergeBase { reference },
        p::BaseChoice::Commit { id } => BaseChoice::Commit { id: CommitId(id) },
    }
}

pub fn core_filter(f: p::ReviewFilter) -> ReviewFilter {
    ReviewFilter {
        roots: (f.roots.iter())
            .map(|r| r.trim_matches('/'))
            .filter(|r| !r.is_empty())
            .map(|r| RepoPath(r.as_bytes().to_vec()))
            .collect(),
        include: f.include,
        exclude: f.exclude,
        hide_binary: f.hide_binary,
    }
}

fn filter(f: &ReviewFilter) -> p::ReviewFilter {
    p::ReviewFilter {
        roots: f.roots.iter().map(path_str).collect(),
        include: f.include.clone(),
        exclude: f.exclude.clone(),
        hide_binary: f.hide_binary,
    }
}

pub fn base(b: &BaseView) -> p::Base {
    p::Base {
        choice: match &b.choice {
            BaseChoice::Pin => p::BaseChoice::Pin,
            BaseChoice::MergeBase { reference } => p::BaseChoice::MergeBase {
                reference: reference.clone(),
            },
            BaseChoice::Commit { id } => p::BaseChoice::Commit { id: id.0.clone() },
        },
        commit: b.commit.0.clone(),
        label: b.label.clone(),
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
        commits: c
            .commits
            .iter()
            .map(|c| p::CommitInfo {
                id: c.id.0.clone(),
                short_id: c.id.0.chars().take(7).collect(),
                subject: c.subject.clone(),
                author: c.author.clone(),
                time: u32::try_from(c.time).unwrap_or(0),
            })
            .collect(),
    }
}

pub fn manifest(session: p::SessionId, m: &ManifestView) -> p::Manifest {
    p::Manifest {
        session,
        revision: m.revision,
        name: m.name.clone(),
        base: base(&m.base),
        filter: filter(&m.filter),
        files: m.files.iter().map(file).collect(),
        completion: completion(&m.completion),
        excluded_lines: m.completion.excluded_lines,
        excluded_items: m.completion.excluded_items,
        degraded: m.degraded.map(|d| match d {
            Degraded::MountedWindowsDrive => p::WatchDegraded::MountedWindowsDrive,
            Degraded::WatchLimit => p::WatchDegraded::WatchLimit,
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

pub fn target_text(t: &TargetText) -> p::Response {
    p::Response::FileText {
        revision: t.revision,
        content: match &t.content {
            Ok(text) => p::TextContent::Text { text: text.clone() },
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

fn side(s: Side) -> p::Side {
    match s {
        Side::Base => p::Side::Base,
        Side::Target => p::Side::Target,
    }
}

fn core_side(s: p::Side) -> Side {
    match s {
        p::Side::Base => Side::Base,
        p::Side::Target => Side::Target,
    }
}

fn line_location(file: &RepoPath, range: LineRange) -> p::LineLocation {
    p::LineLocation {
        path: path_str(file),
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
            path: RepoPath(path.into_bytes()),
            range: LineRange {
                start,
                end: start.saturating_add(len),
            },
        },
        p::ThreadTarget::File { side, path } => NewThread::File {
            side: core_side(side),
            path: RepoPath(path.into_bytes()),
        },
        p::ThreadTarget::Changeset => NewThread::Changeset,
    }
}

pub fn core_thread_status(s: p::ThreadStatus) -> ThreadStatus {
    match s {
        p::ThreadStatus::Open => ThreadStatus::Open,
        p::ThreadStatus::Resolved => ThreadStatus::Resolved,
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
        id: u32::try_from(t.id.0).unwrap_or(u32::MAX),
        kind,
        side: side_of,
        path: t.path.as_ref().map(path_str),
        original,
        location: t.location.as_ref().map(location),
        anchor: t.state.as_ref().map(anchor_state),
        messages: t
            .messages
            .iter()
            .map(|m| p::ThreadMessage {
                author: m.author.clone(),
                body: m.body.clone(),
                at: u32::try_from(m.at).unwrap_or(0),
            })
            .collect(),
        status: match t.status {
            ThreadStatus::Open => p::ThreadStatus::Open,
            ThreadStatus::Resolved => p::ThreadStatus::Resolved,
        },
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
        path: path_str(&u.path),
        review: review(&u.review),
        completion: completion(&u.completion),
        changeset: completion(&u.changeset),
    }
}
