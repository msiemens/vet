use vet_core::review::{EditConflict, MessageId, Outgoing, ThreadId};

use super::{Keep, comment_ref, find_message, pull_request};
use crate::platform::ConversationKind;
use crate::platform::{CommentKind, CommentRef, Platform, PlatformError, RemoteKey};
use crate::ports::{GitRepo, LiveStore, ReviewWrites};
use crate::session::threads::{ThreadView, edit_thread, find_thread, put_message};
use crate::session::{LiveSession, Result, SessionError};

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// Changes the text of one of the viewer's own comments. A draft or a comment outside a
    /// platform changes here. A posted one is also changed on the platform at once; if the platform
    /// has another text than the one this was based on, nothing is sent and the thread shows both
    /// texts until the viewer keeps one (see [`LiveSession::keep_edit`]). A comment the platform
    /// cannot be told about keeps the new text here, and the next sync sends it.
    pub async fn edit_message<P: Platform + Sync>(
        &self,
        platform: &P,
        message: MessageId,
        body: String,
        now: i64,
    ) -> Result<ThreadView> {
        if body.trim().is_empty() {
            return Err(SessionError::Invalid("a message cannot be empty".into()));
        }
        let _exchange = self.exchange.lock().await;
        let (thread, mut edited, remote, comment) = {
            let mut st = self.hold().await;
            let (thread, old) = find_message(&st, message)?;
            if !old.mine {
                return Err(SessionError::Invalid("only your own comments can be edited".into()));
            }
            if old.remote.as_ref().is_some_and(|r| r.conflict.is_some()) {
                return Err(SessionError::Invalid(
                    "this comment was edited in two places: keep one of the two texts first".into(),
                ));
            }
            let mut edited = old.clone();
            let id = thread.id;
            edited.body = body;
            edited.edited_at = Some(now);
            let comment = comment_ref(thread, old);
            put_message(&mut st, id, &edited).await?;
            let remote = comment.as_ref().map(|_| pull_request(&st)).transpose()?;
            (id, edited, remote, comment)
        };
        let (Some((remote, _)), Some(comment)) = (remote, comment) else {
            return self.view_of(thread).await;
        };

        // The platform's text is compared with the one the edit started from.
        let sent = async {
            let current = platform.comment(&remote, &comment).await?;
            let synced = edited.remote.as_ref().expect("posted").synced.clone();
            if current.body != synced && current.body != edited.body {
                return Ok(Err(current));
            }
            platform.edit_comment(&remote, &comment, &edited.body).await.map(Ok)
        }
        .await;
        match sent {
            Ok(Ok(posted)) => {
                let r = edited.remote.as_mut().expect("posted");
                r.synced = posted.body;
                r.conflict = None;
            }
            Ok(Err(current)) => {
                edited.remote.as_mut().expect("posted").conflict = Some(EditConflict {
                    remote_body: current.body,
                    remote_at: current.edited_at,
                });
            }
            Err(e) => return Err(e.into()),
        }
        self.store_message(thread, edited).await?;
        self.view_of(thread).await
    }

    /// Settles an edit made in two places by keeping one of the texts.
    pub async fn keep_edit<P: Platform + Sync>(
        &self,
        platform: &P,
        message: MessageId,
        keep: Keep,
    ) -> Result<ThreadView> {
        let _exchange = self.exchange.lock().await;
        let (thread, mut kept, comment, remote) = {
            let st = self.published.read();
            let (thread, old) = find_message(&st, message)?;
            let conflict = old
                .remote
                .as_ref()
                .and_then(|r| r.conflict.clone())
                .ok_or_else(|| SessionError::Invalid("the comment was not edited in two places".into()))?;
            let mut kept = old.clone();
            if keep == Keep::Remote {
                let r = kept.remote.as_mut().expect("posted");
                kept.body = conflict.remote_body.clone();
                r.synced = conflict.remote_body;
                r.conflict = None;
                kept.edited_at = None;
            }
            (
                thread.id,
                kept,
                comment_ref(thread, old).expect("posted"),
                pull_request(&st)?.0,
            )
        };
        if keep == Keep::Vet {
            let posted = platform.edit_comment(&remote, &comment, &kept.body).await?;
            let r = kept.remote.as_mut().expect("posted");
            r.synced = posted.body;
            r.conflict = None;
        }
        self.store_message(thread, kept).await?;
        self.view_of(thread).await
    }

    /// Removes one of the viewer's comments. A thread's first removes the thread, unless the thread
    /// is posted to a platform that keeps it with its other posted comments
    /// ([`crate::platform::PlatformTraits::first_comment_ends_thread`]). A posted one is removed on
    /// the platform first. One that is being posted stays until that is settled.
    pub async fn remove_message<P: Platform + Sync>(&self, platform: &P, message: MessageId) -> Result<()> {
        let _exchange = self.exchange.lock().await;
        let (thread, ends_thread, posted) = {
            let st = self.published.read();
            let (thread, old) = find_message(&st, message)?;
            if !old.mine {
                return Err(SessionError::Invalid("only your own comments can be removed".into()));
            }
            if matches!(old.outgoing, Outgoing::Posting { .. }) {
                return Err(SessionError::Invalid(
                    "this comment is being posted: remove it once that is done".into(),
                ));
            }
            let posted = comment_ref(thread, old)
                .map(|c| pull_request(&st).map(|(remote, _)| (remote, c)))
                .transpose()?;
            let first = thread.messages.first().map(|m| m.id) == Some(message);
            let others_posted = thread.messages.iter().any(|m| m.id != message && m.remote.is_some());
            let ends_thread =
                first && (posted.is_none() || platform.traits().first_comment_ends_thread || !others_posted);
            (thread.id, ends_thread, posted)
        };
        if let Some((remote, comment)) = posted {
            match platform.delete_comment(&remote, &comment).await {
                Ok(()) | Err(PlatformError::NotFound(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let mut st = self.hold().await;
        if ends_thread {
            st.writes().delete_thread(thread).await?;
            st.threads.retain(|t| t.id != thread);
            if let Some(changes) = st.changes.as_mut() {
                changes.remapped.resolutions.remove(&thread);
            }
        } else {
            st.writes().delete_message(message).await?;
            edit_thread(&mut st, thread, |t| t.messages.retain(|m| m.id != message));
        }
        Ok(())
    }

    /// Deletes a thread; this is also how a detached one is discarded. A thread the platform has
    /// is removed there too, which only its author can, by removing its first comment: on a
    /// platform that keeps a thread with its other comments, they stay.
    pub async fn delete_thread<P: Platform + Sync>(&self, platform: &P, thread: ThreadId) -> Result<()> {
        let first = {
            let st = self.published.read();
            let thread = find_thread(&st, thread)?;
            thread
                .is_posted()
                .then(|| thread.messages.first().map(|m| m.id))
                .flatten()
        };
        if let Some(first) = first {
            return self.remove_message(platform, first).await;
        }
        let mut st = self.hold().await;
        find_thread(&st, thread)?;
        st.writes().delete_thread(thread).await?;
        st.threads.retain(|t| t.id != thread);
        if let Some(changes) = st.changes.as_mut() {
            changes.remapped.resolutions.remove(&thread);
        }
        Ok(())
    }

    /// Changes the text of one of the viewer's comments in the conversation of the pull request on
    /// view, on the platform at once.
    pub async fn edit_conversation_comment<P: Platform + Sync>(
        &self,
        platform: &P,
        comment: &str,
        body: String,
    ) -> Result<()> {
        if body.trim().is_empty() {
            return Err(SessionError::Invalid("a message cannot be empty".into()));
        }
        let _exchange = self.exchange.lock().await;
        let (remote, number) = self.own_conversation_comment(comment)?;
        let posted = platform
            .edit_comment(&remote, &conversation_ref(comment), &body)
            .await?;
        self.exchange.cache.amend_discussion(&remote, number, |discussion| {
            if let Some(item) = discussion.conversation.iter_mut().find(|i| i.id == comment) {
                item.body = posted.body;
            }
        });
        Ok(())
    }

    /// Deletes one of the viewer's comments in the conversation of the pull request on view, on the
    /// platform at once.
    pub async fn remove_conversation_comment<P: Platform + Sync>(&self, platform: &P, comment: &str) -> Result<()> {
        let _exchange = self.exchange.lock().await;
        let (remote, number) = self.own_conversation_comment(comment)?;
        match platform.delete_comment(&remote, &conversation_ref(comment)).await {
            Ok(()) | Err(PlatformError::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
        }
        self.exchange.cache.amend_discussion(&remote, number, |discussion| {
            discussion.conversation.retain(|i| i.id != comment);
        });
        Ok(())
    }

    /// The pull request on view, if `comment` is a comment of the viewer's in the conversation the
    /// last sync found; fails otherwise.
    fn own_conversation_comment(&self, comment: &str) -> Result<(RemoteKey, u64)> {
        let (remote, number) = pull_request(&self.published.read())?;
        let item = (self.exchange.cache.discussion(&remote, number))
            .and_then(|(_, d)| d.conversation.into_iter().find(|i| i.id == comment))
            .ok_or_else(|| SessionError::NotFound(format!("comment {comment}")))?;
        match item.kind {
            ConversationKind::Comment { mine: true } => Ok((remote, number)),
            _ => Err(SessionError::Invalid(
                "only your own comments on the pull request can be changed".into(),
            )),
        }
    }
}

fn conversation_ref(comment: &str) -> CommentRef {
    CommentRef {
        id: comment.to_owned(),
        kind: CommentKind::Conversation,
    }
}
