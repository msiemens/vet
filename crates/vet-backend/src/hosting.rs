//! The platforms pull requests are on, and the one way they are called: every call goes through
//! [`Hosting`], which notes how it went in the platforms' [`Health`], keeps what it learns of pull
//! requests in their [`PullRequestCache`], and tells the open projects when the standing of an
//! account changed.

use std::sync::{Arc, Mutex, Weak};

use vet_app::platform::{
    CheckLog, CommentRef, CommitChecks, Discussion, Health, Outcome, PipelineAction, Platform, PlatformComment,
    PlatformError, PlatformKind, PlatformTraits, PullRequestCache, PullRequestDetail, PullRequestListing,
    PullRequestQuery, RemoteKey, Standing, Step, Submission, Submitted, Viewer, Watched,
};
use vet_app::registry::Subscribers;
use vet_core::review::Verdict;
use vet_mapping as mapping;
use vet_protocol::{Event, Notification, NotificationKind, PlatformConnection, ProjectId};

use crate::events::Connection;
use crate::mutex::lock;
use crate::{AzureDevOps, GitHub};

/// An open project, which hears of the platforms' standing.
pub(crate) struct Announcer {
    pub project: ProjectId,
    pub subscribers: Subscribers<Connection>,
}

/// The clients of the hosting platforms, for every project, and how they are doing.
pub(crate) struct Platforms {
    github: Arc<GitHub>,
    azure_devops: Arc<AzureDevOps>,
    health: Arc<Health>,
    known: Arc<PullRequestCache>,
    announcers: Arc<Mutex<Vec<Weak<Announcer>>>>,
}

impl Platforms {
    /// `process` hears when a platform no longer accepts a credential.
    pub(crate) fn new(github: Arc<GitHub>, azure_devops: Arc<AzureDevOps>, process: Subscribers<Connection>) -> Self {
        let announcers: Arc<Mutex<Vec<Weak<Announcer>>>> = Arc::default();
        let listeners = announcers.clone();
        Self {
            github,
            azure_devops,
            health: Arc::new(Health::new(move |outcome| announce(&listeners, &process, outcome))),
            known: Arc::default(),
            announcers,
        }
    }

    /// Lets `announcer` hear of the platforms' standing for as long as it lives.
    pub(crate) fn tell(&self, announcer: &Arc<Announcer>) {
        let mut announcers = lock(&self.announcers);
        announcers.retain(|a| a.strong_count() > 0);
        announcers.push(Arc::downgrade(announcer));
    }

    /// The client of the platform `kind`.
    pub(crate) fn hosting(self: &Arc<Self>, kind: PlatformKind) -> Hosting {
        let client = match kind {
            PlatformKind::GitHub => Client::GitHub(self.github.clone()),
            PlatformKind::AzureDevOps => Client::AzureDevOps(self.azure_devops.clone()),
        };
        Watched::new(client, kind, self.health.clone(), self.known.clone())
    }

    /// What the platforms said of pull requests.
    pub(crate) fn known(&self) -> Arc<PullRequestCache> {
        self.known.clone()
    }

    /// How the last call to the platform of `account` went, if there was one.
    pub(crate) fn standing(&self, account: &str) -> Option<Standing> {
        self.health.standing(account)
    }
}

/// Tells what an `outcome` means to the viewer: a platform that no longer accepts the credential
/// needs a new one every time it says so; a standing that changed is told to every open project,
/// and a sync that stopped for lack of a credential is told once.
fn announce(announcers: &Mutex<Vec<Weak<Announcer>>>, process: &Subscribers<Connection>, outcome: &Outcome) {
    if let Standing::Failing(PlatformError::Rejected(_)) = &outcome.now {
        process.broadcast(Event::CredentialsNeeded {
            host: outcome.account.clone(),
        });
    }
    if !outcome.changed() {
        return;
    }
    let connection = mapping::platform_connection(outcome.now.error());
    let host = &outcome.account;
    let stopped = match &connection {
        PlatformConnection::SignedOut => Some(format!("Not signed in to {host}: comments are not synced.")),
        PlatformConnection::TokenExpired { .. } => Some(format!(
            "The token for {host} is no longer accepted: comments are not synced."
        )),
        _ => None,
    };
    let live: Vec<Arc<Announcer>> = lock(announcers).iter().filter_map(Weak::upgrade).collect();
    for announcer in live {
        if let Some(text) = &stopped {
            announcer.subscribers.broadcast(Event::Notification {
                project: announcer.project,
                notification: Notification {
                    kind: NotificationKind::SyncStopped,
                    text: text.clone(),
                    copy_text: None,
                    host: Some(host.clone()),
                },
            });
        }
        announcer.subscribers.broadcast(Event::PlatformStatus {
            project: announcer.project,
            status: mapping::platform_status(host, outcome.kind, connection.clone()),
        });
    }
}

/// A call to a platform, watched and learned from.
pub(crate) type Hosting = Watched<Client>;

/// The client of one of the platforms.
pub(crate) enum Client {
    GitHub(Arc<GitHub>),
    AzureDevOps(Arc<AzureDevOps>),
}

/// Calls `$call` on the client the platform is.
macro_rules! on {
    ($self:ident, $client:ident => $call:expr) => {
        match $self {
            Client::GitHub($client) => $call,
            Client::AzureDevOps($client) => $call,
        }
    };
}

impl Platform for Client {
    fn traits(&self) -> PlatformTraits {
        on!(self, p => p.traits())
    }

    async fn viewer(&self, remote: &RemoteKey) -> Result<Viewer, PlatformError> {
        on!(self, p => p.viewer(remote).await)
    }

    async fn list_pull_requests(
        &self,
        remote: &RemoteKey,
        query: &PullRequestQuery,
    ) -> Result<PullRequestListing, PlatformError> {
        on!(self, p => p.list_pull_requests(remote, query).await)
    }

    async fn pull_request(&self, remote: &RemoteKey, number: u64) -> Result<PullRequestDetail, PlatformError> {
        on!(self, p => p.pull_request(remote, number).await)
    }

    async fn checks(&self, remote: &RemoteKey, commit: &str) -> Result<CommitChecks, PlatformError> {
        on!(self, p => p.checks(remote, commit).await)
    }

    fn forget_checks(&self, remote: &RemoteKey) {
        match self {
            Client::GitHub(platform) => platform.forget_checks(remote),
            Client::AzureDevOps(platform) => platform.forget_checks(remote),
        }
    }

    async fn check_log(&self, remote: &RemoteKey, check: &str) -> Result<CheckLog, PlatformError> {
        on!(self, p => p.check_log(remote, check).await)
    }

    async fn steps(&self, remote: &RemoteKey, check: &str) -> Result<Vec<Step>, PlatformError> {
        on!(self, p => p.steps(remote, check).await)
    }

    async fn pipeline_action(&self, remote: &RemoteKey, action: &PipelineAction) -> Result<(), PlatformError> {
        on!(self, p => p.pipeline_action(remote, action).await)
    }

    async fn discussion(&self, remote: &RemoteKey, number: u64) -> Result<Discussion, PlatformError> {
        on!(self, p => p.discussion(remote, number).await)
    }

    async fn comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<PlatformComment, PlatformError> {
        on!(self, p => p.comment(remote, comment).await)
    }

    async fn edit_comment(
        &self,
        remote: &RemoteKey,
        comment: &CommentRef,
        body: &str,
    ) -> Result<PlatformComment, PlatformError> {
        on!(self, p => p.edit_comment(remote, comment, body).await)
    }

    async fn delete_comment(&self, remote: &RemoteKey, comment: &CommentRef) -> Result<(), PlatformError> {
        on!(self, p => p.delete_comment(remote, comment).await)
    }

    async fn set_thread_resolved(&self, remote: &RemoteKey, thread: &str, resolved: bool) -> Result<(), PlatformError> {
        on!(self, p => p.set_thread_resolved(remote, thread, resolved).await)
    }

    async fn submit_review(
        &self,
        remote: &RemoteKey,
        number: u64,
        submission: &Submission,
    ) -> Result<Submitted, PlatformError> {
        on!(self, p => p.submit_review(remote, number, submission).await)
    }

    async fn vote(&self, remote: &RemoteKey, number: u64, verdict: &Verdict) -> Result<(), PlatformError> {
        on!(self, p => p.vote(remote, number, verdict).await)
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::UnboundedReceiver;
    use vet_protocol::ServerMessage;
    use vet_transport::Events;

    use super::*;

    fn listener() -> (Subscribers<Connection>, UnboundedReceiver<ServerMessage>) {
        let (events, inbox) = Events::channel();
        let subscribers = Subscribers::default();
        subscribers.subscribe(Connection(events));
        (subscribers, inbox)
    }

    fn heard(inbox: &mut UnboundedReceiver<ServerMessage>) -> Vec<Event> {
        std::iter::from_fn(|| inbox.try_recv().ok())
            .filter_map(|m| match m {
                ServerMessage::Event { event } => Some(event),
                _ => None,
            })
            .collect()
    }

    fn outcome(before: Option<Standing>, now: Standing) -> Outcome {
        Outcome {
            account: "github.com".to_owned(),
            kind: PlatformKind::GitHub,
            before,
            now,
        }
    }

    fn announcing_to(
        project: ProjectId,
        subscribers: Subscribers<Connection>,
    ) -> (Arc<Announcer>, Mutex<Vec<Weak<Announcer>>>) {
        let announcer = Arc::new(Announcer { project, subscribers });
        let announcers = Mutex::new(vec![Arc::downgrade(&announcer)]);
        (announcer, announcers)
    }

    #[test]
    fn a_standing_that_changed_is_told_to_every_open_project_and_a_stopped_sync_once() {
        let (project, mut inbox) = listener();
        let (process, mut process_inbox) = listener();
        let (_announcer, announcers) = announcing_to(4, project);
        let signed_out = || Standing::Failing(PlatformError::SignedOut("github.com".to_owned()));

        announce(&announcers, &process, &outcome(None, signed_out()));
        announce(&announcers, &process, &outcome(Some(signed_out()), signed_out()));

        let events = heard(&mut inbox);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(
            &events[0],
            Event::Notification { project: 4, notification } if notification.kind == NotificationKind::SyncStopped
        ));
        assert!(matches!(
            &events[1],
            Event::PlatformStatus { project: 4, status } if status.connection == PlatformConnection::SignedOut
        ));
        assert!(
            heard(&mut process_inbox).is_empty(),
            "a missing credential is asked for elsewhere"
        );
    }

    #[test]
    fn a_rate_limit_reaches_the_projects_with_when_it_ends_and_a_rejected_token_is_asked_for_each_time() {
        let (project, mut inbox) = listener();
        let (process, mut process_inbox) = listener();
        let (_announcer, announcers) = announcing_to(4, project);
        let limited = Standing::Failing(PlatformError::RateLimited {
            message: "slow down".to_owned(),
            retry_at: Some(1_700),
        });
        let rejected = || Standing::Failing(PlatformError::Rejected("expired".to_owned()));

        announce(&announcers, &process, &outcome(Some(Standing::Fine), limited));
        announce(&announcers, &process, &outcome(None, rejected()));
        announce(&announcers, &process, &outcome(Some(rejected()), rejected()));

        let statuses: Vec<_> = heard(&mut inbox)
            .into_iter()
            .filter_map(|e| match e {
                Event::PlatformStatus { status, .. } => Some(status.connection),
                _ => None,
            })
            .collect();
        assert_eq!(statuses.len(), 2, "the second rejection changes nothing");
        assert_eq!(
            statuses[0],
            PlatformConnection::Failed {
                message: "slow down".to_owned(),
                retry_at: Some(1_700)
            }
        );
        let asked = heard(&mut process_inbox)
            .into_iter()
            .filter(|e| matches!(e, Event::CredentialsNeeded { .. }))
            .count();
        assert_eq!(asked, 2);
    }

    #[test]
    fn a_project_that_is_gone_hears_nothing_and_does_not_stop_the_others() {
        let (kept, mut inbox) = listener();
        let (gone, _) = listener();
        let (first, mut announcers) = announcing_to(1, gone);
        let (_second, other) = announcing_to(2, kept);
        announcers.get_mut().unwrap().extend(other.into_inner().unwrap());
        drop(first);

        announce(
            &announcers,
            &Subscribers::default(),
            &outcome(None, Standing::Failing(PlatformError::Unreachable("down".to_owned()))),
        );

        assert_eq!(heard(&mut inbox).len(), 1);
    }
}
