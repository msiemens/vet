//! vet-app views to protocol DTOs, and back for the request enums.

pub mod account;
pub mod progress;
pub mod semantic;

use vet_app::changeset::{BaseChoice, CommitId, Lens, RemoteKey, Source, VersionOrigin};
use vet_app::platform::{
    CheckLog, CiState, CommitChecks, PlatformError, PlatformKind, PullRequestState, ReviewVerdict, ReviewerState,
};
use vet_app::ports::{BaseCandidates, CommitInfo, Degraded};
use vet_app::pull_request::PullRequestList;
use vet_app::session::{
    BaseDiffKind, BaseDiffView, BaseView, ChangesetList, CommentAt, CommentSpot, CommitKind, CommitsView, DirEntryKind,
    DirListing, DiscussionView, EntryInfo, FileStatus, FileView, ItemBlob, ItemView, Lifecycle, ManifestView,
    ModeChange, NewThread, NoText, ReviewUpdateView, ReviewView, RowReview, SubmitReport, SyncNotice, TargetText,
    ThreadView, ThreadsView, VersionView, View,
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
        p::View::Commit { index } => View::Commit { index },
    }
}

pub fn core_lens(lens: p::Lens) -> Lens {
    match lens {
        p::Lens::Squash => Lens::Squash,
        p::Lens::PerCommit => Lens::PerCommit,
    }
}

fn lens(lens: Lens) -> p::Lens {
    match lens {
        Lens::Squash => p::Lens::Squash,
        Lens::PerCommit => p::Lens::PerCommit,
    }
}

pub fn commits(list: &CommitsView) -> p::Response {
    p::Response::Commits {
        revision: list.revision,
        lens: lens(list.lens),
        commits: list
            .commits
            .iter()
            .map(|c| p::CommitUnit {
                commit: commit_info(&c.info),
                body: c.body.clone(),
                parents: c.parents.iter().map(|p| p.0.clone()).collect(),
                kind: match c.kind {
                    CommitKind::Commit => p::CommitKind::Commit,
                    CommitKind::Merge { clean, remerged } => p::CommitKind::Merge { clean, remerged },
                },
                delta: line_delta(c.delta),
                completion: completion(&c.completion),
            })
            .collect(),
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
        old_base_lines: r.old_base_lines.clone(),
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

pub fn base_diff(d: &BaseDiffView) -> p::Response {
    p::Response::BaseDiff(p::BaseDiff {
        from: d.from.as_ref().map(commit_info),
        to: commit_info(&d.to),
        rows: (d.rows.iter())
            .map(|r| p::BaseDiffRow {
                kind: match r.kind {
                    BaseDiffKind::Context => p::BaseDiffKind::Context,
                    BaseDiffKind::Removed => p::BaseDiffKind::Removed,
                    BaseDiffKind::Added => p::BaseDiffKind::Added,
                },
                line: r.line,
                text: r.text.clone(),
            })
            .collect(),
    })
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
            BaseChoice::MergeBase { reference } => p::BaseChoice::MergeBase {
                reference: reference.clone(),
            },
            BaseChoice::Commit { id } => p::BaseChoice::Commit { id: id.0.clone() },
        },
        commit: commit_info(&b.commit),
        name: b.name.clone(),
        behind: b.behind,
        contained_in: b.contained_in.clone(),
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
        Source::PullRequest { remote, number } => p::ChangesetKey::PullRequest {
            remote: remote.to_string(),
            number: u32::try_from(*number).unwrap_or(u32::MAX),
        },
    }
}

/// The source `key` names. The error is fit to show.
pub fn core_changeset_key(key: p::ChangesetKey) -> Result<Source, String> {
    Ok(match key {
        p::ChangesetKey::WorkingTree => Source::WorkingTree,
        p::ChangesetKey::Branch { name } => Source::Branch { name },
        p::ChangesetKey::PullRequest { remote, number } => {
            let (host, path) = remote
                .split_once('/')
                .filter(|(host, path)| !host.is_empty() && !path.is_empty())
                .ok_or_else(|| format!("\"{remote}\" is not a repository, e.g. github.com/acme/app"))?;
            Source::PullRequest {
                remote: RemoteKey {
                    host: host.to_owned(),
                    path: path.to_owned(),
                },
                number: u64::from(number),
            }
        }
    })
}

fn row_review(review: RowReview) -> p::RowReview {
    match review {
        RowReview::None => p::RowReview::None,
        RowReview::Open {
            version,
            started_at,
            last_reviewed,
        } => p::RowReview::Open {
            version,
            started_at: unix_secs(started_at),
            last_reviewed,
        },
        RowReview::Finished {
            complete,
            started_at,
            finished_at,
            last_reviewed,
        } => p::RowReview::Finished {
            complete,
            started_at: unix_secs(started_at),
            finished_at: unix_secs(finished_at),
            last_reviewed,
        },
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
                branch: r.branch.clone(),
                open_threads: r.open_threads,
                divergence: r.divergence.map(|d| p::Divergence {
                    ahead: d.ahead,
                    behind: d.behind,
                }),
                last_commit: r.last_commit.map(unix_secs),
                review: row_review(r.review),
            })
            .collect(),
    }
}

fn ci_state(state: CiState) -> p::CiState {
    match state {
        CiState::Pending => p::CiState::Pending,
        CiState::Success => p::CiState::Success,
        CiState::Failure => p::CiState::Failure,
        CiState::Neutral => p::CiState::Neutral,
        CiState::Skipped => p::CiState::Skipped,
    }
}

/// The key of pull request `number` of the repository `remote`.
pub fn pull_request_source(remote: &RemoteKey, number: u64) -> Source {
    Source::PullRequest {
        remote: remote.clone(),
        number,
    }
}

/// The answer to `ListPullRequests`. `reviews` has what Vet knows of the review of each row, in
/// the order of `list.rows`.
pub fn pull_requests(list: &PullRequestList, reviews: &[RowReview]) -> p::Response {
    p::Response::PullRequests {
        rows: list
            .rows
            .iter()
            .zip(reviews)
            .map(|(listed, review)| {
                let row = &listed.row;
                p::PullRequestRow {
                    key: changeset_key(&pull_request_source(&listed.remote, row.number)),
                    title: row.title.clone(),
                    author: row.author.clone(),
                    source_branch: row.source_branch.clone(),
                    target_branch: row.target_branch.clone(),
                    fork: row.fork.clone(),
                    lines: row.lines.map(|l| p::LineDelta {
                        added: l.added,
                        removed: l.removed,
                    }),
                    comments: row.comments,
                    ci: row.ci.map(ci_state),
                    draft: row.draft,
                    waiting_for_me: row.waiting_for_me,
                    unresolved_threads: row.unresolved_threads,
                    updated_at: unix_secs(row.updated_at),
                    review: row_review(*review),
                    url: row.url.clone(),
                }
            })
            .collect(),
        sources: list
            .sources
            .iter()
            .map(|source| p::PullRequestSource {
                remote: source.remote.to_string(),
                error: source.error.as_ref().map(ToString::to_string),
                signed_out: matches!(source.error, Some(PlatformError::SignedOut(_))),
            })
            .collect(),
    }
}

/// The answer to `Checks`; `error` is why some commit could not be answered for.
pub fn checks(found: &[CommitChecks], error: Option<&PlatformError>) -> p::Response {
    p::Response::Checks {
        checks: found
            .iter()
            .map(|commit| p::CommitChecks {
                commit: commit.commit.clone(),
                state: commit.state().map(ci_state),
                runs: commit
                    .checks
                    .iter()
                    .map(|check| p::CheckRun {
                        id: check.id.clone(),
                        name: check.name.clone(),
                        state: ci_state(check.state),
                        duration: check
                            .started
                            .zip(check.finished)
                            .map(|(from, to)| unix_secs((to - from).max(0))),
                        url: check.url.clone(),
                        log: check.log,
                    })
                    .collect(),
                url: commit.url.clone(),
            })
            .collect(),
        error: error.map(ToString::to_string),
    }
}

pub fn check_log(log: &CheckLog) -> p::Response {
    p::Response::CheckLog(p::CheckLog {
        text: log.text.clone(),
        failure: log.failure.as_ref().map(|f| p::LogFailure {
            line: f.line,
            step: f.step.clone(),
        }),
    })
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
            head: m.target.head.clone(),
            title: None,
            draft: false,
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
        lens: lens(m.lens),
        commits: m.commits,
        changeset_files: m.changeset_files,
        changeset_completion: completion(&m.changeset_completion),
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
                id: sat_u32(m.id.0),
                author: m.author.clone(),
                body: m.body.clone(),
                at: unix_secs(m.at),
                mine: m.mine,
                avatar: m.avatar.clone(),
                posted: m.remote.is_some(),
                edited_at: m.edited_at.map(unix_secs),
                conflict: m
                    .remote
                    .as_ref()
                    .and_then(|r| r.conflict.as_ref())
                    .map(|c| p::EditConflict {
                        remote_body: c.remote_body.clone(),
                        remote_at: unix_secs(c.remote_at),
                    }),
            })
            .collect(),
        status: thread_status(t.status),
        platform: t.platform.as_ref().map(|pt| p::ThreadPlatform {
            posted: pt.remote_id.is_some(),
            resolvable: pt.resolvable,
            resolved_by: pt.resolved_by.clone(),
        }),
    }
}

fn reviewer_state(s: ReviewerState) -> p::ReviewerState {
    match s {
        ReviewerState::Waiting => p::ReviewerState::Waiting,
        ReviewerState::Approved => p::ReviewerState::Approved,
        ReviewerState::ChangesRequested => p::ReviewerState::ChangesRequested,
        ReviewerState::Commented => p::ReviewerState::Commented,
    }
}

/// The properties of a platform the UI branches on.
pub fn platform_traits(kind: PlatformKind) -> p::PlatformTraits {
    match kind {
        PlatformKind::GitHub => p::PlatformTraits {
            conversation_feed: true,
            batched_review: true,
            verdicts: [
                ("COMMENT", "Comment"),
                ("APPROVE", "Approve"),
                ("REQUEST_CHANGES", "Request changes"),
            ]
            .into_iter()
            .map(|(id, label)| p::Verdict {
                id: id.into(),
                label: label.into(),
            })
            .collect(),
            verdict_separate: false,
        },
        PlatformKind::AzureDevOps => p::PlatformTraits {
            conversation_feed: false,
            batched_review: false,
            verdicts: Vec::new(),
            verdict_separate: true,
        },
    }
}

/// How the backend gets on with a platform, after `result` of talking to it.
pub fn platform_connection(result: Option<&PlatformError>) -> p::PlatformConnection {
    match result {
        None => p::PlatformConnection::Ok,
        Some(PlatformError::SignedOut(_)) => p::PlatformConnection::SignedOut,
        Some(PlatformError::Rejected(message)) => p::PlatformConnection::TokenExpired {
            message: message.clone(),
        },
        Some(other) => p::PlatformConnection::Failed {
            message: other.to_string(),
        },
    }
}

pub fn platform_status(host: &str, kind: PlatformKind, connection: p::PlatformConnection) -> p::PlatformStatus {
    p::PlatformStatus {
        host: host.to_owned(),
        kind: match kind {
            PlatformKind::GitHub => p::PlatformKind::GitHub,
            PlatformKind::AzureDevOps => p::PlatformKind::AzureDevOps,
        },
        connection,
    }
}

/// What the Comments tool window shows above the threads of the pull request `v` is.
pub fn pull_request_info(v: &DiscussionView, kind: PlatformKind, platform: p::PlatformStatus) -> p::PullRequestInfo {
    let d = &v.discussion;
    let traits = platform_traits(kind);
    let mut blocking = Vec::new();
    if v.unresolved_threads > 0 {
        blocking.push(p::Blocker::Threads {
            count: v.unresolved_threads,
        });
    }
    if matches!(d.ci, Some(CiState::Failure | CiState::Pending)) {
        blocking.push(p::Blocker::Checks);
    }
    if d.reviewers.iter().any(|r| r.state == ReviewerState::ChangesRequested) {
        blocking.push(p::Blocker::ChangesRequested);
    }
    p::PullRequestInfo {
        number: sat_u32(v.number),
        title: d.title.clone(),
        url: d.url.clone(),
        draft: d.draft,
        state: match d.state {
            PullRequestState::Open => p::PullRequestState::Open,
            PullRequestState::Closed => p::PullRequestState::Closed,
            PullRequestState::Merged => p::PullRequestState::Merged,
        },
        author: d.author.clone(),
        author_avatar: d.author_avatar.clone(),
        source_branch: d.source_branch.clone(),
        target_branch: d.target_branch.clone(),
        reviewers: (d.reviewers.iter())
            .map(|r| p::Reviewer {
                name: r.name.clone(),
                state: reviewer_state(r.state),
            })
            .collect(),
        labels: d.labels.clone(),
        ci: d.ci.map(ci_state),
        blocking,
        conversation: if traits.conversation_feed {
            (d.conversation.iter())
                .map(|c| p::ConversationEntry {
                    author: c.author.clone(),
                    avatar: c.avatar.clone(),
                    at: unix_secs(c.at),
                    verdict: c.verdict.and_then(|v| match v {
                        ReviewVerdict::Approved => Some(p::ReviewerState::Approved),
                        ReviewVerdict::ChangesRequested => Some(p::ReviewerState::ChangesRequested),
                        ReviewVerdict::Commented => Some(p::ReviewerState::Commented),
                        ReviewVerdict::Dismissed => None,
                    }),
                    body: c.body.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        traits,
        synced_at: Some(unix_secs(v.synced_at)),
        platform,
    }
}

pub fn notification(n: &SyncNotice) -> p::Notification {
    match n {
        SyncNotice::ThreadDeleted { place, dropped } => p::Notification {
            kind: p::NotificationKind::Info,
            text: if dropped.is_empty() {
                format!("Thread on {place} was deleted on GitHub.")
            } else {
                format!("Thread on {place} was deleted on GitHub. Your draft reply was removed.")
            },
            copy_text: (!dropped.is_empty()).then(|| dropped.join("\n\n")),
            host: None,
        },
    }
}

pub fn review_submitted(r: &SubmitReport) -> p::Response {
    p::Response::ReviewSubmitted(p::ReviewSubmitted {
        posted: r.posted,
        skipped: r.skipped,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pull_request_key_names_its_repository_by_host_and_path_and_comes_back_as_it_went() {
        for remote in ["github.com/acme/app", "ghe.corp.example:8443/acme/app"] {
            let key = p::ChangesetKey::PullRequest {
                remote: remote.to_owned(),
                number: 482,
            };
            let source = core_changeset_key(key.clone()).unwrap();
            assert_eq!(changeset_key(&source), key);
        }
        let Source::PullRequest { remote, number } = core_changeset_key(p::ChangesetKey::PullRequest {
            remote: "ghe.corp.example:8443/acme/app".to_owned(),
            number: 7,
        })
        .unwrap() else {
            panic!("not a pull request")
        };
        assert_eq!(
            (remote.host.as_str(), remote.path.as_str(), number),
            ("ghe.corp.example:8443", "acme/app", 7)
        );
    }

    #[test]
    fn a_repository_that_is_not_host_and_path_is_refused_with_a_message() {
        for remote in ["github.com", "github.com/", "/acme/app", ""] {
            let key = p::ChangesetKey::PullRequest {
                remote: remote.to_owned(),
                number: 1,
            };
            let error = core_changeset_key(key).unwrap_err();
            assert!(error.contains("is not a repository"), "{remote:?}: {error}");
        }
    }
}
