//! Review threads: creating and answering them, placing their anchors, and
//! the lines comments attach to.

use std::collections::BTreeMap;

use vet_core::content::{EntryKind, LineRange, RepoPath, Snapshot, is_binary};
use vet_core::resolve::Resolution;
use vet_core::review::{
    Anchor, AnchorState, CommentTarget, Location, Message, Side, StaleReason, Thread, ThreadId, ThreadStatus,
    target_line,
};

use super::interdiff::{View, check_commentable, markable};
use super::manifest::{old_path_of, status_of};
use super::state::{Changes, State, check_revision, find_diff, outcome, shown_items};
use super::views::{FileStatus, Outcome};
use super::{LiveSession, Result, SessionError};
use crate::editor::last_text_line;
use crate::ports::{GitRepo, LiveStore};

const NEW_SIDE_ONLY: &str = "a comment on an interdiff goes on its new side; the old side is the older version rebased \
     onto the newer base, which is not a place to comment";

/// What a new thread is attached to. Ranges are in the text of the revision the
/// request names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NewThread {
    Range {
        side: Side,
        path: RepoPath,
        range: LineRange,
    },
    File {
        side: Side,
        path: RepoPath,
    },
    Changeset,
}

/// What a comment is about, in the terms of a changeset file's view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommentAt {
    File,
    /// The hunk with this index among the file's hunks.
    Hunk(u32),
    /// `len` lines from line `start` of one side's text.
    Lines {
        side: Side,
        start: u32,
        len: u32,
    },
}

/// Where a comment attaches, and where its form is drawn.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommentSpot {
    /// A file at its target side, a deleted file at its base; a hunk at its target lines, a
    /// pure deletion at its base lines.
    pub target: NewThread,
    /// For a range: the target line a unified diff shows the range's last line at.
    pub line: Option<u32>,
}

/// A thread with its anchor resolved against the current revision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThreadView {
    pub id: ThreadId,
    /// As recorded, never replaced by a resolution.
    pub target: CommentTarget,
    /// The file the thread is shown at: where the anchor or file is now, else
    /// where it was. `None` for a changeset thread.
    pub path: Option<RepoPath>,
    /// `None` for a changeset thread.
    pub state: Option<AnchorState>,
    /// The lines of a range thread now; `None` when it is detached.
    pub location: Option<Location>,
    /// The line of the target text the thread's card is shown below in a unified diff; `Some`
    /// exactly when `location` is.
    pub target_line: Option<u32>,
    pub messages: Vec<Message>,
    pub status: ThreadStatus,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThreadsView {
    pub revision: u32,
    pub threads: Vec<ThreadView>,
}

impl<G: GitRepo + Sync, S: LiveStore + Sync> LiveSession<G, S> {
    /// Every thread, with range anchors resolved at the current revision. An interdiff shows the
    /// threads on the new side of the latest version, and none for another.
    pub async fn threads(&self, view: View) -> Result<ThreadsView> {
        let mut st = self.state.lock().await;
        self.ensure_view(&mut st, view).await?;
        let shown = |t: &&Thread| match view {
            View::Changeset => true,
            View::Interdiff { .. } => {
                markable(&st, view) && matches!(&t.target, CommentTarget::Range(a) if a.side == Side::Target)
            }
            View::Commit { .. } => false,
        };
        Ok(ThreadsView {
            revision: st.revision,
            threads: st.threads.iter().filter(shown).map(|t| thread_view(&st, t)).collect(),
        })
    }

    /// The target line a unified diff shows `line` of `side` of the file at `path` at, by the rule
    /// that places a thread's card (`ThreadView::target_line`). Fails if `revision` is no longer
    /// current.
    pub async fn line_in_target(&self, revision: u32, view: View, side: Side, path: &str, line: u32) -> Result<u32> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_view(&mut st, view).await?;
        let location = Location {
            file: RepoPath::from(path),
            range: LineRange {
                start: line,
                end: line.saturating_add(1),
            },
        };
        Ok(card_line(st.changes_in(view), side, &location))
    }

    /// What a comment on part of the changeset file at `path` attaches to (see [`CommentSpot`]).
    /// Fails if `revision` is no longer current, or the file or hunk is not in the changeset. In an
    /// interdiff a comment goes on the new side only, and only on the latest version's.
    pub async fn comment_target(&self, revision: u32, view: View, path: &str, at: CommentAt) -> Result<CommentSpot> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_view(&mut st, view).await?;
        check_commentable(&st, view)?;
        let spot = self.spot(&st, view, path, at)?;
        match (&spot.target, view) {
            (
                NewThread::Range { side: Side::Base, .. } | NewThread::File { side: Side::Base, .. },
                View::Interdiff { .. },
            ) => Err(SessionError::Invalid(NEW_SIDE_ONLY.into())),
            _ => Ok(spot),
        }
    }

    fn spot(&self, st: &State, view: View, path: &str, at: CommentAt) -> Result<CommentSpot> {
        let changes = st.changes_in(view);
        let diff = find_diff(&changes.diff, path);
        let items = shown_items(&changes.diff, &st.filter, path);
        if diff.is_none() && items.is_empty() {
            return Err(SessionError::NotFound(format!("file {path}")));
        }
        let now = RepoPath::from(path);
        let before = old_path_of(&now, diff, &items).unwrap_or_else(|| now.clone());
        let on = |side| {
            if side == Side::Base {
                before.clone()
            } else {
                now.clone()
            }
        };
        let (side, range) = match at {
            CommentAt::File => {
                let side = if status_of(diff, &items) == FileStatus::Deleted {
                    Side::Base
                } else {
                    Side::Target
                };
                return Ok(CommentSpot {
                    target: NewThread::File { side, path: on(side) },
                    line: None,
                });
            }
            CommentAt::Hunk(index) => {
                let hunk = diff
                    .and_then(|d| d.hunks.get(index as usize))
                    .ok_or_else(|| SessionError::NotFound(format!("hunk {index}")))?;
                if !hunk.new.is_empty() {
                    (Side::Target, hunk.new)
                } else {
                    (Side::Base, hunk.old)
                }
            }
            CommentAt::Lines { side, start, len } if len > 0 => (
                side,
                LineRange {
                    start,
                    end: start.saturating_add(len),
                },
            ),
            CommentAt::Lines { .. } => return Err(SessionError::Invalid("no lines are selected".into())),
        };
        let file = on(side);
        let line = card_line(
            changes,
            side,
            &Location {
                file: file.clone(),
                range,
            },
        );
        Ok(CommentSpot {
            target: NewThread::Range {
                side,
                path: file,
                range,
            },
            line: Some(line),
        })
    }

    /// Opens a thread with its first message, and starts the review if none is
    /// open. `now` is unix seconds.
    pub async fn create_thread(
        &self,
        revision: u32,
        view: View,
        target: NewThread,
        body: String,
        now: i64,
    ) -> Result<Outcome<ThreadView>> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_view(&mut st, view).await?;
        check_commentable(&st, view)?;
        if let (
            View::Interdiff { .. },
            NewThread::Range { side: Side::Base, .. } | NewThread::File { side: Side::Base, .. },
        ) = (view, &target)
        {
            return Err(SessionError::Invalid(NEW_SIDE_ONLY.into()));
        }
        let opener = self.message(body, now)?;
        let target = match target {
            NewThread::Range { side, path, range } => {
                CommentTarget::Range(self.anchor_at(&st, side, &path, range).await?)
            }
            NewThread::File { side, path } => {
                if !side_snapshot(&st, side).entries.contains_key(&path) {
                    return Err(SessionError::NotFound(format!("file {}", path.to_string_lossy())));
                }
                CommentTarget::File { side, path }
            }
            NewThread::Changeset => CommentTarget::Changeset,
        };
        let started = self.begin(&mut st).await?;
        let review = st.review.as_ref().expect("begun").id;
        let changeset = st.changeset.expect("begun");
        let id = self
            .acquisition
            .store
            .add_thread(changeset, Some(review), target.clone(), opener.clone())
            .await?;
        st.threads.push(Thread {
            id,
            target,
            messages: vec![opener],
            status: ThreadStatus::Open,
        });
        self.remap(&mut st).await?;
        Ok(outcome(&st, thread_by_id(&st, id)?, started))
    }

    pub async fn reply(&self, thread: ThreadId, body: String, now: i64) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        find_thread(&st, thread)?;
        let message = self.message(body, now)?;
        self.acquisition.store.add_message(thread, message.clone()).await?;
        edit_thread(&mut st, thread, |t| t.messages.push(message));
        thread_by_id(&st, thread)
    }

    /// Resolves or reopens a thread.
    pub async fn set_thread_status(&self, thread: ThreadId, status: ThreadStatus) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        self.ensure_changes(&mut st).await?;
        find_thread(&st, thread)?;
        self.acquisition.store.set_thread_status(thread, status).await?;
        edit_thread(&mut st, thread, |t| t.status = status);
        thread_by_id(&st, thread)
    }

    /// Deletes a thread; this is also how a detached one is discarded.
    pub async fn delete_thread(&self, thread: ThreadId) -> Result<()> {
        let mut st = self.state.lock().await;
        find_thread(&st, thread)?;
        self.acquisition.store.delete_thread(thread).await?;
        st.threads.retain(|t| t.id != thread);
        if let Some(changes) = st.changes.as_mut() {
            changes.resolutions.remove(&thread);
        }
        Ok(())
    }

    /// Re-anchors an `Inferred` thread at its current location, which makes it
    /// `Exact`. Fails if `revision` is no longer current.
    pub async fn confirm_anchor(&self, revision: u32, thread: ThreadId) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let location = match resolution_of(&st, thread)? {
            Resolution {
                state: AnchorState::Inferred { .. },
                location: Some(location),
            } => location.clone(),
            _ => return Err(SessionError::Invalid("the anchor was not moved".into())),
        };
        self.reanchor(&mut st, thread, &location).await
    }

    /// Re-anchors an `Ambiguous` thread at one of its candidates (an index into
    /// the candidates of its state). Fails if `revision` is no longer current.
    pub async fn attach_anchor(&self, revision: u32, thread: ThreadId, candidate: usize) -> Result<ThreadView> {
        let mut st = self.state.lock().await;
        check_revision(&st, revision)?;
        self.ensure_changes(&mut st).await?;
        let location = match &resolution_of(&st, thread)?.state {
            AnchorState::Ambiguous { candidates } => candidates
                .get(candidate)
                .cloned()
                .ok_or_else(|| SessionError::NotFound(format!("candidate {candidate}")))?,
            _ => return Err(SessionError::Invalid("the anchor is not ambiguous".into())),
        };
        self.reanchor(&mut st, thread, &location).await
    }

    fn message(&self, body: String, at: i64) -> Result<Message> {
        if body.trim().is_empty() {
            return Err(SessionError::Invalid("a message cannot be empty".into()));
        }
        Ok(Message {
            author: self.author.clone(),
            body,
            at,
        })
    }

    /// Records the thread's anchor anew, from the current text at `location`.
    async fn reanchor(&self, st: &mut State, thread: ThreadId, location: &Location) -> Result<ThreadView> {
        let side = match &find_thread(st, thread)?.target {
            CommentTarget::Range(a) => a.side,
            _ => return Err(SessionError::Invalid("only a range thread has an anchor".into())),
        };
        let target = CommentTarget::Range(self.anchor_at(st, side, &location.file, location.range).await?);
        self.acquisition.store.set_thread_target(thread, target.clone()).await?;
        edit_thread(st, thread, |t| t.target = target);
        self.remap(st).await?;
        thread_by_id(st, thread)
    }

    /// An anchor on `range` of `path` in the current snapshot of `side`.
    async fn anchor_at(&self, st: &State, side: Side, path: &RepoPath, range: LineRange) -> Result<Anchor> {
        let snapshot = side_snapshot(st, side);
        let entry = snapshot
            .entries
            .get(path)
            .filter(|e| e.kind == EntryKind::File)
            .ok_or_else(|| SessionError::NotFound(format!("file {}", path.to_string_lossy())))?;
        let mut blobs = BTreeMap::new();
        if !self.fetch(&mut blobs, path, entry.blob, true).await? || is_binary(&blobs[&entry.blob]) {
            return Err(SessionError::NotText(path.to_string_lossy()));
        }
        Anchor::from_text(side, snapshot.id, path.clone(), &blobs[&entry.blob], range)
            .ok_or_else(|| SessionError::Invalid("the lines are outside the file".into()))
    }
}

fn side_snapshot(st: &State, side: Side) -> &Snapshot {
    match side {
        Side::Base => &st.base,
        Side::Target => &st.target,
    }
}

fn find_thread(st: &State, id: ThreadId) -> Result<&Thread> {
    st.threads
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| SessionError::NotFound(format!("thread {}", id.0)))
}

fn edit_thread(st: &mut State, id: ThreadId, edit: impl FnOnce(&mut Thread)) {
    if let Some(t) = st.threads.iter_mut().find(|t| t.id == id) {
        edit(t);
    }
}

/// The resolution of a range thread at the current revision.
fn resolution_of(st: &State, id: ThreadId) -> Result<&Resolution> {
    find_thread(st, id)?;
    let changes = st.changes.as_ref().expect("computed");
    changes
        .resolutions
        .get(&id)
        .ok_or_else(|| SessionError::Invalid("only a range thread has an anchor".into()))
}

fn thread_by_id(st: &State, id: ThreadId) -> Result<ThreadView> {
    Ok(thread_view(st, find_thread(st, id)?))
}

/// File threads follow a git-detected rename and detach only when the file is
/// gone; changeset threads never detach.
fn thread_view(st: &State, t: &Thread) -> ThreadView {
    let changes = st.changes.as_ref().expect("computed");
    let (path, state, location) = match &t.target {
        CommentTarget::Range(a) => {
            let r = &changes.resolutions[&t.id];
            let path = r.location.as_ref().map_or(&a.file, |l| &l.file);
            (Some(path.clone()), Some(r.state.clone()), r.location.clone())
        }
        CommentTarget::File { side, path } => {
            let moved = (*side == Side::Target && !st.target.entries.contains_key(path))
                .then(|| changes.diff.renames.0.get(path))
                .flatten()
                .filter(|new| st.target.entries.contains_key(*new));
            let state = if moved.is_some() || side_snapshot(st, *side).entries.contains_key(path) {
                AnchorState::Exact {
                    whitespace_changed: false,
                }
            } else {
                AnchorState::Stale {
                    reason: StaleReason::FileDeleted,
                }
            };
            (Some(moved.unwrap_or(path).clone()), Some(state), None)
        }
        CommentTarget::Changeset => (None, None, None),
    };
    let target_line = match (&t.target, &location) {
        (CommentTarget::Range(a), Some(l)) => Some(card_line(changes, a.side, l)),
        _ => None,
    };
    ThreadView {
        id: t.id,
        target: t.target.clone(),
        path,
        state,
        location,
        target_line,
        messages: t.messages.clone(),
        status: t.status,
    }
}

/// The target line a range thread's card is shown below: the line its range ends on, as the
/// target text has it, but not past the last line of the file.
fn card_line(changes: &Changes, side: Side, location: &Location) -> u32 {
    let diff = changes.diff.files.iter().find(|d| {
        let path = if side == Side::Base { &d.old_path } else { &d.new_path };
        path.as_ref() == Some(&location.file)
    });
    let hunks = diff.map_or(&[][..], |d| &d.hunks[..]);
    let last = location.range.end.saturating_sub(1).max(location.range.start);
    let line = target_line(hunks, side, last);
    // A deleted file has no target text; an unchanged one is not in the diff and cannot overrun.
    // The empty line after a final line break holds no text, so no card sits below it.
    let last = diff.map(|d| {
        d.new_blob
            .and_then(|b| changes.blobs.get(&b))
            .map_or(0, |t| last_text_line(t))
    });
    last.map_or(line, |last| line.min(last))
}
