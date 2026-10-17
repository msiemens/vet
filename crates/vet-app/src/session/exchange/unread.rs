//! What counts as unread (D96): someone else's comment written after the viewer last looked, and
//! the time they last looked, kept per pull request. The backend decides; the shell only words it.

use vet_core::review::{CommentTarget, Thread};

use crate::changeset::Source;
use crate::platform::{ConversationItem, ConversationKind, RemoteKey};
use crate::ports::{GitRepo, LiveStore};
use crate::session::{LiveSession, Result};

/// The app preference the seen times are kept under, as one JSON object, so it outlasts a restart.
const SEEN_KEY: &str = "commentsSeen";

/// How many pull requests' seen times are kept; the least recently written go first.
const KEPT: usize = 500;

/// What the viewer has not read of a pull request's comments.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Unread {
    /// Comments on the pull request as a whole: its conversation and its changeset threads.
    pub all: u32,
    /// Replies to the viewer in the threads they are in.
    pub replies: u32,
}

/// Whether the viewer has not read a comment: someone else's, written after `seen`. Nothing is
/// unread while there is no such time.
pub fn is_unread(mine: bool, at: i64, seen: Option<i64>) -> bool {
    seen.is_some_and(|seen| !mine && at > seen)
}

/// Whether the conversation entry was written by the viewer.
fn entry_mine(e: &ConversationItem) -> bool {
    match e.kind {
        ConversationKind::Comment { mine } | ConversationKind::Review { mine, .. } => mine,
    }
}

/// The comments on the pull request as a whole: its conversation, and the messages of its changeset
/// threads. Comments on lines show at their lines and do not count.
fn on_the_whole<'a>(
    conversation: &'a [ConversationItem],
    threads: &'a [Thread],
) -> impl Iterator<Item = (bool, i64)> + 'a {
    conversation.iter().map(|e| (entry_mine(e), e.at)).chain(
        threads
            .iter()
            .filter(|t| matches!(t.target, CommentTarget::Changeset))
            .flat_map(|t| t.messages.iter().map(|m| (m.mine, m.at))),
    )
}

/// How many comments on the pull request as a whole the viewer has not read.
pub fn unread_all(conversation: &[ConversationItem], threads: &[Thread], seen: Option<i64>) -> u32 {
    on_the_whole(conversation, threads)
        .filter(|&(mine, at)| is_unread(mine, at, seen))
        .count() as u32
}

/// How many messages someone else wrote after the viewer last looked, in the threads they are in.
pub fn replies_for_you(threads: &[Thread], seen: Option<i64>) -> u32 {
    threads
        .iter()
        .filter(|t| t.messages.iter().any(|m| m.mine))
        .flat_map(|t| t.messages.iter())
        .filter(|m| is_unread(m.mine, m.at, seen))
        .count() as u32
}

/// The time to remember as seen: now, or the newest comment by someone else if the platform's
/// clock is ahead.
pub fn seen_now(conversation: &[ConversationItem], threads: &[Thread], now: i64) -> i64 {
    on_the_whole(conversation, threads)
        .filter(|&(mine, _)| !mine)
        .map(|(_, at)| at)
        .fold(now, i64::max)
}

/// What the time is remembered under: the pull request, whichever clone of its repository shows it.
pub fn seen_key(remote: &RemoteKey, number: u64) -> String {
    format!("{}/{}#{number}", remote.host, remote.path)
}

/// The seen times as the preference keeps them: least recently written first, so the oldest can be
/// dropped. A value that is not a time is dropped.
type Seen = Vec<(String, i64)>;

fn parse_seen(json: &str) -> Seen {
    let value: serde_json::Value = serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
    let serde_json::Value::Object(fields) = value else {
        return Seen::new();
    };
    fields
        .into_iter()
        .filter_map(|(key, value)| Some((key, value.as_i64()?)).filter(|(_, at)| *at > 0))
        .collect()
}

fn render_seen(seen: &Seen) -> String {
    serde_json::Value::Object(seen.iter().map(|(k, v)| (k.clone(), (*v).into())).collect()).to_string()
}

/// `seen` with `key` last written at `at`, and the oldest dropped beyond `KEPT`.
fn with_seen(seen: Seen, key: String, at: i64) -> Seen {
    let mut seen: Seen = seen.into_iter().filter(|(other, _)| other != &key).collect();
    seen.push((key, at));
    let extra = seen.len().saturating_sub(KEPT);
    seen.drain(..extra);
    seen
}

impl<G: GitRepo + Send + Sync + 'static, S: LiveStore + Send + Sync + 'static> LiveSession<G, S> {
    /// The pull request on view, with the repository and number its seen time is kept under.
    fn pull_request_on_view(&self) -> Option<(RemoteKey, u64)> {
        match &self.published.read().source {
            Source::PullRequest { remote, number } => Some((remote.clone(), *number)),
            _ => None,
        }
    }

    /// When the viewer last looked at the pull request on view's comments; none for another
    /// changeset, and before they ever looked.
    pub async fn comments_seen(&self) -> Result<Option<i64>> {
        let Some((remote, number)) = self.pull_request_on_view() else {
            return Ok(None);
        };
        let key = seen_key(&remote, number);
        Ok(self
            .stored_seen()
            .await?
            .and_then(|json| parse_seen(&json).into_iter().find(|(k, _)| *k == key))
            .map(|(_, at)| at))
    }

    /// The seen times the preference holds, as one JSON object.
    async fn stored_seen(&self) -> Result<Option<String>> {
        Ok(self
            .acquisition
            .store
            .preferences(None)
            .await?
            .into_iter()
            .find(|(key, _)| key == SEEN_KEY)
            .map(|(_, value)| value))
    }

    /// Marks the pull request on view's comments seen up to now (or the newest comment, where the
    /// platform's clock is ahead). Answers whether what is unread changed, so the caller announces
    /// only then.
    pub async fn mark_comments_seen(&self, now: i64) -> Result<bool> {
        let Some((remote, number)) = self.pull_request_on_view() else {
            return Ok(false);
        };
        let conversation = self
            .discussion()
            .await
            .map(|d| d.discussion.conversation)
            .unwrap_or_default();
        let seen = self.comments_seen().await?;
        let before = self.unread(&conversation, seen);
        let at = {
            let st = self.published.read();
            seen_now(&conversation, &st.threads, now)
        };
        let mut times = self
            .stored_seen()
            .await?
            .map(|json| parse_seen(&json))
            .unwrap_or_default();
        times = with_seen(times, seen_key(&remote, number), at);
        self.acquisition
            .store
            .set_preference(None, SEEN_KEY.to_owned(), Some(render_seen(&times)))
            .await?;
        Ok(self.unread(&conversation, Some(at)) != before)
    }

    /// What the pull request on view has unread, from the time the viewer last looked.
    pub fn unread(&self, conversation: &[ConversationItem], seen: Option<i64>) -> Unread {
        let st = self.published.read();
        Unread {
            all: unread_all(conversation, &st.threads, seen),
            replies: replies_for_you(&st.threads, seen),
        }
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::RepoPath;
    use vet_core::review::{CommentTarget, Message, MessageId, Outgoing, Side, Thread, ThreadId, ThreadStatus};

    use super::*;
    use crate::platform::{ConversationItem, ConversationKind, ReviewVerdict};

    fn message(at: i64, mine: bool) -> Message {
        Message {
            id: MessageId(at as u64),
            author: if mine { "you".into() } else { "lena".into() },
            body: String::new(),
            at,
            mine,
            avatar: None,
            edited_at: None,
            remote: None,
            outgoing: Outgoing::Draft,
        }
    }

    fn thread(target: CommentTarget, messages: Vec<Message>) -> Thread {
        Thread {
            id: ThreadId(1),
            target,
            status: ThreadStatus::Open,
            messages,
            platform: None,
        }
    }

    fn changeset(messages: Vec<Message>) -> Thread {
        thread(CommentTarget::Changeset, messages)
    }

    /// A comment on lines, which the whole-pull-request count leaves out.
    fn on_a_line(messages: Vec<Message>) -> Thread {
        thread(
            CommentTarget::File {
                side: Side::Target,
                path: RepoPath::from("a.rs"),
            },
            messages,
        )
    }

    fn entry(at: i64, mine: bool) -> ConversationItem {
        ConversationItem {
            id: format!("e{at}"),
            author: "lena".into(),
            avatar: None,
            at,
            kind: ConversationKind::Review {
                verdict: Some(ReviewVerdict::Approved),
                mine,
            },
            body: String::new(),
        }
    }

    #[test]
    fn only_what_others_wrote_after_the_viewer_last_looked_is_unread() {
        assert!(is_unread(false, 11, Some(10)));
        assert!(!is_unread(false, 10, Some(10)));
        assert!(!is_unread(true, 11, Some(10)));
        assert!(!is_unread(false, 11, None), "nothing is new before the first look");
    }

    #[test]
    fn the_whole_pull_request_counts_its_conversation_and_changeset_threads_but_not_line_threads() {
        let conversation = [entry(100, false), entry(200, true)];
        let threads = [
            changeset(vec![message(150, false), message(250, false)]),
            on_a_line(vec![message(500, false)]),
        ];
        assert_eq!(unread_all(&conversation, &threads, None), 0);
        assert_eq!(unread_all(&conversation, &threads, Some(120)), 2);
        assert_eq!(unread_all(&conversation, &threads, Some(240)), 1);
        assert_eq!(unread_all(&conversation, &threads, Some(300)), 0);
    }

    #[test]
    fn replies_for_you_are_the_unread_messages_of_threads_the_viewer_is_in() {
        let mine = changeset(vec![message(100, true), message(250, false), message(260, true)]);
        let theirs = changeset(vec![message(500, false)]);
        assert_eq!(replies_for_you(&[mine.clone(), theirs.clone()], Some(200)), 1);
        assert_eq!(replies_for_you(&[mine, theirs], Some(600)), 0);
    }

    #[test]
    fn opening_the_comments_sees_up_to_now_or_the_newest_comment_where_the_clock_is_ahead() {
        let conversation = [entry(900, false)];
        let threads = [changeset(vec![message(950, false)])];
        assert_eq!(seen_now(&conversation, &threads, 1_000), 1_000);
        assert_eq!(seen_now(&conversation, &threads, 800), 950);
        assert_eq!(
            unread_all(&conversation, &threads, Some(seen_now(&conversation, &threads, 800))),
            0
        );
    }

    #[test]
    fn the_time_is_kept_per_pull_request_and_the_oldest_go_first() {
        let mut times = Seen::new();
        for n in 0..KEPT as u64 + 10 {
            times = with_seen(times, format!("host/repo#{n}"), n as i64 + 1);
        }
        assert_eq!(times.len(), KEPT);
        assert!(times.iter().all(|(key, _)| key != "host/repo#0"), "the oldest went");
        assert_eq!(
            times.last().map(|(k, _)| k.as_str()),
            Some(format!("host/repo#{}", KEPT + 9).as_str())
        );
        let remote = RemoteKey {
            host: "host".into(),
            path: "repo".into(),
        };
        assert_eq!(seen_key(&remote, 7), "host/repo#7");
    }
}
