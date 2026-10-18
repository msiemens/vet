//! The event contract (9.62): what each request changes, and which events tell the clients.
//!
//! Every request that changes what a client shows announces it to every subscriber, the
//! requester included: of the requester's stores only the one that asked reads the answer, and
//! the others learn of the change from the event. A mutation announces its own events and does
//! not leave them to a later sync. The classification is exhaustive, so a new request is
//! classified when it is added; the test drives every mutation that can be driven here with two
//! connections and checks that both hear what is declared.

use vet_protocol::{Event, Request};

/// An event by its kind, without what it carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    CredentialsNeeded,
    AccountsChanged,
    RecentProjectsChanged,
    ProjectRemoved,
    StorageChanged,
    SettingsChanged,
    PreferenceChanged,
    SnapshotMoved,
    FilterChanged,
    ReviewFinished,
    EditsPending,
    ChangesetOpened,
    ChangesetLinesChanged,
    ReviewChanged,
    LensChanged,
    TargetChanged,
    ChangesetsChanged,
    PullRequestListChanged,
    PullRequestsChanged,
    ReviewStarted,
    ThreadsChanged,
    ChecksChanged,
    FileSymbols,
    FileDiagnostics,
    ServerInstallOutput,
    ServersChanged,
    ServerRuntimes,
    RestoreFailed,
    Task,
    PlatformStatus,
    Notification,
}

impl Kind {
    pub fn of(event: &Event) -> Self {
        match event {
            Event::CredentialsNeeded { .. } => Self::CredentialsNeeded,
            Event::AccountsChanged => Self::AccountsChanged,
            Event::RecentProjectsChanged => Self::RecentProjectsChanged,
            Event::ProjectRemoved { .. } => Self::ProjectRemoved,
            Event::StorageChanged => Self::StorageChanged,
            Event::SettingsChanged => Self::SettingsChanged,
            Event::PreferenceChanged { .. } => Self::PreferenceChanged,
            Event::SnapshotMoved { .. } => Self::SnapshotMoved,
            Event::FilterChanged { .. } => Self::FilterChanged,
            Event::ReviewFinished { .. } => Self::ReviewFinished,
            Event::EditsPending { .. } => Self::EditsPending,
            Event::ChangesetOpened { .. } => Self::ChangesetOpened,
            Event::ChangesetLinesChanged { .. } => Self::ChangesetLinesChanged,
            Event::ReviewChanged { .. } => Self::ReviewChanged,
            Event::LensChanged { .. } => Self::LensChanged,
            Event::TargetChanged { .. } => Self::TargetChanged,
            Event::ChangesetsChanged { .. } => Self::ChangesetsChanged,
            Event::PullRequestListChanged { .. } => Self::PullRequestListChanged,
            Event::PullRequestsChanged { .. } => Self::PullRequestsChanged,
            Event::ReviewStarted { .. } => Self::ReviewStarted,
            Event::ThreadsChanged { .. } => Self::ThreadsChanged,
            Event::ChecksChanged { .. } => Self::ChecksChanged,
            Event::FileSymbols { .. } => Self::FileSymbols,
            Event::FileDiagnostics { .. } => Self::FileDiagnostics,
            Event::ServerInstallOutput { .. } => Self::ServerInstallOutput,
            Event::ServersChanged { .. } => Self::ServersChanged,
            Event::ServerRuntimes { .. } => Self::ServerRuntimes,
            Event::RestoreFailed { .. } => Self::RestoreFailed,
            Event::Task(_) => Self::Task,
            Event::PlatformStatus { .. } => Self::PlatformStatus,
            Event::Notification { .. } => Self::Notification,
        }
    }
}

/// What a request does to the state the clients show.
#[derive(Debug)]
pub enum Effect {
    /// Reads. It may subscribe the connection, keep what it computed, or ask for a credential
    /// (`CredentialsNeeded`); nothing another store shows changes. A query whose answer is long
    /// may push it in parts (`ChangesetLinesChanged`): that is the answer arriving early, not a
    /// change another store must hear.
    Query,
    /// Changes what the backend keeps, but nothing a store keeps; why.
    Unannounced(#[expect(dead_code, reason = "read by whoever reads the classification")] &'static str),
    /// Changes what clients show. When it did what it is for, every subscriber, the requester
    /// included, hears each of `events`: those of a project its connections, those of the process
    /// every connection that asked for `Accounts` or `RecentProjects`; none hears one of `never`,
    /// events that a client would answer with reads the change does not call for. `untested` says
    /// why the test cannot drive it, if it cannot.
    Mutation {
        events: &'static [Kind],
        never: &'static [Kind],
        untested: Option<&'static str>,
    },
}

const fn tested(events: &'static [Kind]) -> Effect {
    tested_never(events, &[])
}

const fn tested_never(events: &'static [Kind], never: &'static [Kind]) -> Effect {
    Effect::Mutation {
        events,
        never,
        untested: None,
    }
}

const fn untested(events: &'static [Kind], why: &'static str) -> Effect {
    Effect::Mutation {
        events,
        never: &[],
        untested: Some(why),
    }
}

const NEEDS_A_PLATFORM: &str = "needs a pull request on view, which only a platform has";
const NEEDS_THE_ACCOUNTS: &str = "runs the platforms' CLIs and asks the platform about the credential, and the \
                                  orchestrator's accounts have no fakes";

pub fn effect(request: &Request) -> Effect {
    use Kind::*;
    match request {
        Request::Ping { .. }
        | Request::Subscribe
        | Request::RecentProjects
        | Request::Accounts
        | Request::ProjectsStorage
        | Request::ProjectStorage { .. }
        | Request::Settings
        | Request::Preferences { .. }
        | Request::LogDirectory
        | Request::OpenProject { .. }
        | Request::ListChangesets { .. }
        // Reads the feed's list; the feed lists, prunes refs and reads details on its own, and
        // announces `PullRequestListChanged` and `PullRequestsChanged` for what it finds.
        | Request::ListPullRequests { .. }
        | Request::ChangesetLines { .. }
        | Request::Checks { .. }
        | Request::CheckLog { .. }
        | Request::PipelineSteps { .. }
        | Request::Tasks { .. }
        | Request::WaitForQuiet { .. }
        | Request::Versions { .. }
        | Request::ChangedSince { .. }
        | Request::Commits { .. }
        | Request::Manifest { .. }
        | Request::FileContents { .. }
        | Request::BaseDiff { .. }
        | Request::Hover { .. }
        | Request::Definition { .. }
        | Request::References { .. }
        | Request::Implementations { .. }
        | Request::Diagnostics { .. }
        | Request::FileSymbols { .. }
        | Request::FileOutline { .. }
        | Request::WorkspaceSymbols { .. }
        | Request::SymbolAt { .. }
        | Request::Signature { .. }
        | Request::Calls { .. }
        | Request::Usages { .. }
        | Request::RemovalImpact { .. }
        | Request::Reachability { .. }
        | Request::ListServers { .. }
        | Request::ServedBy { .. }
        | Request::ListDirectory { .. }
        | Request::ListFiles { .. }
        | Request::PatternMatches { .. }
        | Request::FileText { .. }
        | Request::Search { .. }
        | Request::ItemBlob { .. }
        | Request::Threads { .. }
        | Request::TargetLine { .. }
        | Request::MapLines { .. }
        | Request::CommentTarget { .. }
        | Request::PullRequestInfo { .. } => Effect::Query,
        // Announces `ThreadsChanged` when what is unread changed.
        Request::MarkCommentsSeen { .. } => untested(
            &[ThreadsChanged],
            "needs a pull request on view with comments it has not read, which only a platform has",
        ),
        Request::CloseProject { .. } => {
            Effect::Unannounced("unsubscribes the connection; the others see nothing change")
        }
        Request::ChangePipeline { .. } => Effect::Unannounced(
            "changes a run on the platform, which Vet keeps nothing of; the window that asked reads the checks again",
        ),
        Request::ClearSemanticCaches
        | Request::ClearProjectCaches { .. } => tested(&[StorageChanged]),
        Request::RemoveProject { .. } => tested(&[ProjectRemoved, RecentProjectsChanged, StorageChanged]),
        Request::SetSettings { .. } => tested(&[SettingsChanged]),
        Request::SetPreference { .. } => tested(&[PreferenceChanged]),
        Request::RecordRecentProject { .. } | Request::ForgetRecentProject { .. } => tested(&[RecentProjectsChanged]),
        // Not for a token handed back unchecked that is held already. The open pull requests of
        // the host are synced again, in the background.
        Request::SetCredentials { .. } | Request::RemoveCredentials { .. } => {
            untested(&[AccountsChanged], NEEDS_THE_ACCOUNTS)
        }
        // When another changeset comes on view; reopening the pull request on view announces
        // `TargetChanged` if the platform names it differently now.
        Request::OpenChangeset { .. } => tested(&[ChangesetOpened]),
        // The queue's rows are read again for `ReviewFinished` itself.
        Request::FinishReview { .. } => tested_never(&[ReviewFinished], &[ChangesetsChanged, SnapshotMoved]),
        Request::SetLens { .. } => tested(&[LensChanged]),
        // The first mark of a review also announces `ReviewStarted`.
        Request::ToggleHunk { .. }
        | Request::ToggleFlag { .. }
        | Request::ToggleItem { .. }
        | Request::ToggleFileSeen { .. }
        | Request::SetFilesSeen { .. }
        | Request::ReconfirmFile { .. } => tested(&[ReviewChanged]),
        Request::SetProjectFilter { .. } | Request::ExcludePath { .. } | Request::NarrowReview { .. } => {
            tested_never(&[FilterChanged], &[SnapshotMoved, ReviewFinished])
        }
        Request::SetProjectTrusted { .. } => tested(&[TargetChanged, ServersChanged]),
        Request::TrustPullRequest { .. } => untested(&[TargetChanged, ServersChanged], NEEDS_A_PLATFORM),
        // When a remote was fetched. A pull request on view whose head moved settles
        // (`SnapshotMoved`), one the platform names differently says `TargetChanged`, and its
        // sync announces as `SyncThreads` does.
        Request::Fetch { .. } => tested(&[ChangesetsChanged]),
        // The first comment of a review also announces `ReviewStarted`.
        Request::CreateThread { .. }
        | Request::Reply { .. }
        | Request::SetThreadStatus { .. }
        | Request::DeleteThread { .. }
        | Request::EditMessage { .. }
        | Request::RemoveMessage { .. }
        | Request::ConfirmAnchor { .. }
        | Request::AttachAnchor { .. } => tested(&[ThreadsChanged]),
        Request::KeepEdit { .. } => untested(
            &[ThreadsChanged],
            "needs a comment edited both here and on the platform, which only a sync with a platform finds",
        ),
        // The conversation is read with the threads, through `PullRequestInfo`.
        Request::EditConversationComment { .. } | Request::DeleteConversationComment { .. } => {
            untested(&[ThreadsChanged], NEEDS_A_PLATFORM)
        }
        // `ThreadsChanged` when the sync changed threads, and any `Notification`.
        Request::SyncThreads { .. } => untested(&[PlatformStatus], NEEDS_A_PLATFORM),
        // The vote is shown with `PullRequestInfo`, which reloads with the threads.
        Request::StageVote { .. } | Request::DiscardVote { .. } => untested(&[ThreadsChanged], NEEDS_A_PLATFORM),
        // `ThreadsChanged` when a draft was posted, whatever the sync finds.
        Request::SubmitReview { .. } | Request::PostDrafts { .. } => {
            untested(&[PlatformStatus, ThreadsChanged], NEEDS_A_PLATFORM)
        }
        Request::InstallServer { .. } => {
            untested(&[ServersChanged], "downloads and runs the language's installer")
        }
        Request::SetServerPath { .. } | Request::RestartServer { .. } => tested(&[ServersChanged]),
        Request::SetReviewShown { .. } => {
            Effect::Unannounced("decides when servers start; the connection that asked knows what it showed")
        }
        Request::SetOpenFiles { .. } => {
            Effect::Unannounced("orders the background analysis of symbols and problems; nothing a client shows changes")
        }
        Request::RefreshProblems { .. } => Effect::Unannounced(
            "restarts the background analysis; the answers it pushes are its own, not this request's",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::time::Duration;

    use tokio::sync::mpsc::Receiver;
    use vet_protocol::{
        AnchorState, BackendSettings, ChangesetKey, CommentAt, EditChoice, Lens, Manifest, NonTextKind, PipelineAction,
        ProjectId, Response, ReviewFilter, SearchPattern, SearchScope, SearchSide, Secret, ServerMessage, Side,
        SymbolId, Thread, ThreadStatus, ThreadTarget, View,
    };
    use vet_transport::Events;

    use super::*;
    use crate::events::Connection;
    use crate::{Config, Orchestrator, open_store};

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    /// `n` lines, with the given ones edited.
    fn text(n: usize, edited: &[usize]) -> String {
        (0..n)
            .map(|i| {
                if edited.contains(&i) {
                    format!("edited {i}\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect()
    }

    fn tag(request: &Request) -> String {
        serde_json::to_value(request).unwrap()["type"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The tag of every request there is, as the protocol's deserializer lists them.
    fn every_tag() -> BTreeSet<String> {
        let error = serde_json::from_value::<Request>(serde_json::json!({ "type": "-" }))
            .unwrap_err()
            .to_string();
        let tags: BTreeSet<String> = error.split('`').skip(3).step_by(2).map(str::to_owned).collect();
        assert!(tags.contains("ping") && tags.contains("setServerPath"), "{error}");
        tags
    }

    /// One request of every kind, with what it carries made up: for the classification only.
    fn samples() -> Vec<Request> {
        let project = 1;
        let view = View::Changeset;
        let path = || "a.txt".to_owned();
        let symbol = || SymbolId {
            path: path(),
            name: vec!["f".into()],
        };
        vec![
            Request::Ping { payload: String::new() },
            Request::Subscribe,
            Request::RecentProjects,
            Request::RecordRecentProject { path: path() },
            Request::ForgetRecentProject { path: path() },
            Request::Accounts,
            Request::SetCredentials {
                host: "github.com".into(),
                token: Secret("t".into()),
                validate: false,
            },
            Request::RemoveCredentials {
                host: "github.com".into(),
            },
            Request::ClearSemanticCaches,
            Request::ProjectsStorage,
            Request::ProjectStorage { root: path() },
            Request::ClearProjectCaches { root: path() },
            Request::RemoveProject { root: path() },
            Request::Settings,
            Request::SetSettings {
                settings: BackendSettings {
                    semantic_cache_mb: 1,
                    finished_review_days: 1,
                },
            },
            Request::Preferences { scope: None },
            Request::SetPreference {
                scope: None,
                key: "theme".into(),
                value: None,
            },
            Request::LogDirectory,
            Request::OpenProject { repo: path() },
            Request::CloseProject { project },
            Request::ListChangesets { project },
            Request::ListPullRequests { project },
            Request::ChangesetLines {
                project,
                changesets: vec![ChangesetKey::WorkingTree],
            },
            Request::Checks {
                project,
                commits: vec![],
            },
            Request::CheckLog {
                project,
                run: String::new(),
            },
            Request::PipelineSteps {
                project,
                check: String::new(),
            },
            Request::ChangePipeline {
                project,
                action: PipelineAction::Cancel { run: String::new() },
            },
            Request::OpenChangeset {
                project,
                changeset: ChangesetKey::WorkingTree,
            },
            Request::FinishReview { project },
            Request::Tasks { project },
            Request::WaitForQuiet { project },
            Request::Versions { project },
            Request::ChangedSince { project, from: 1 },
            Request::Commits { project },
            Request::SetLens {
                project,
                lens: Lens::Squash,
            },
            Request::Manifest { project, view },
            Request::FileContents {
                project,
                view,
                path: path(),
            },
            Request::ToggleHunk {
                project,
                view,
                revision: 0,
                path: path(),
                hunk: 0,
            },
            Request::ToggleFlag {
                project,
                view,
                revision: 0,
                path: path(),
                hunk: 0,
            },
            Request::ToggleItem {
                project,
                view,
                revision: 0,
                path: path(),
                kind: NonTextKind::Mode,
            },
            Request::ToggleFileSeen {
                project,
                view,
                revision: 0,
                path: path(),
            },
            Request::SetFilesSeen {
                project,
                view,
                revision: 0,
                paths: vec![path()],
                seen: true,
            },
            Request::ReconfirmFile {
                project,
                revision: 0,
                path: path(),
            },
            Request::BaseDiff {
                project,
                revision: 0,
                path: path(),
                hunk: 0,
            },
            Request::Hover {
                project,
                revision: 0,
                side: Side::Target,
                path: path(),
                line: 0,
                col: 0,
            },
            Request::Definition {
                project,
                revision: 0,
                side: Side::Target,
                path: path(),
                line: 0,
                col: 0,
            },
            Request::References {
                project,
                revision: 0,
                side: Side::Target,
                path: path(),
                line: 0,
                col: 0,
                limit: None,
            },
            Request::Implementations {
                project,
                revision: 0,
                side: Side::Target,
                path: path(),
                line: 0,
                col: 0,
            },
            Request::Diagnostics {
                project,
                revision: 0,
                path: path(),
                against_base: false,
            },
            Request::FileSymbols {
                project,
                revision: 0,
                path: path(),
            },
            Request::FileOutline {
                project,
                view: View::Changeset,
                revision: 0,
                path: path(),
            },
            Request::WorkspaceSymbols {
                project,
                revision: 0,
                query: String::new(),
            },
            Request::SymbolAt {
                project,
                revision: 0,
                side: Side::Target,
                path: path(),
                line: 0,
                col: 0,
            },
            Request::Signature {
                project,
                revision: 0,
                symbol: symbol(),
            },
            Request::Calls {
                project,
                revision: 0,
                symbol: symbol(),
            },
            Request::Usages {
                project,
                revision: 0,
                symbol: symbol(),
            },
            Request::RemovalImpact {
                project,
                revision: 0,
                symbol: symbol(),
            },
            Request::Reachability {
                project,
                revision: 0,
                symbol: symbol(),
            },
            Request::ListServers { project },
            Request::ServedBy {
                project,
                paths: vec!["a.rs".into()],
            },
            Request::InstallServer {
                project,
                language: "rust".into(),
            },
            Request::SetServerPath {
                project,
                language: "rust".into(),
                path: None,
            },
            Request::RestartServer {
                project,
                language: "rust".into(),
            },
            Request::SetReviewShown { project, shown: true },
            Request::SetOpenFiles {
                project,
                paths: Vec::new(),
            },
            Request::RefreshProblems { project },
            Request::Fetch { project },
            Request::SetProjectFilter {
                project,
                filter: ReviewFilter::default(),
            },
            Request::NarrowReview {
                project,
                changeset: ChangesetKey::WorkingTree,
                filter: None,
            },
            Request::SetProjectTrusted { project, trusted: true },
            Request::TrustPullRequest { project },
            Request::ExcludePath {
                project,
                path: path(),
                directory: false,
            },
            Request::ListDirectory {
                project,
                path: String::new(),
            },
            Request::ListFiles { project },
            Request::PatternMatches {
                project,
                filter: ReviewFilter::default(),
            },
            Request::FileText {
                project,
                view,
                path: path(),
            },
            Request::Search {
                project,
                scope: SearchScope::Repository,
                pattern: SearchPattern {
                    text: "needle".into(),
                    match_case: false,
                    whole_word: false,
                    regex: false,
                },
                mask: String::new(),
                side: SearchSide::Both,
                only_changes: false,
                not_seen: false,
                limit: 10,
            },
            Request::ItemBlob {
                project,
                view,
                revision: 0,
                path: path(),
                kind: NonTextKind::Mode,
                side: Side::Target,
            },
            Request::Threads { project, view },
            Request::TargetLine {
                project,
                view,
                revision: 0,
                side: Side::Target,
                path: path(),
                line: 0,
            },
            Request::MapLines {
                project,
                revision: 0,
                from: 1,
                path: path(),
                lines: vec![0],
            },
            Request::CommentTarget {
                project,
                view,
                revision: 0,
                path: path(),
                at: CommentAt::File,
            },
            Request::CreateThread {
                project,
                view,
                revision: 0,
                target: ThreadTarget::Range {
                    side: Side::Target,
                    path: path(),
                    start: 0,
                    len: 1,
                },
                body: String::new(),
            },
            Request::Reply {
                project,
                thread: 0,
                body: String::new(),
            },
            Request::SetThreadStatus {
                project,
                thread: 0,
                status: ThreadStatus::Resolved,
            },
            Request::DeleteThread { project, thread: 0 },
            Request::EditMessage {
                project,
                message: 0,
                body: String::new(),
            },
            Request::RemoveMessage { project, message: 0 },
            Request::KeepEdit {
                project,
                message: 0,
                keep: EditChoice::Vet,
            },
            Request::EditConversationComment {
                project,
                comment: String::new(),
                body: String::new(),
            },
            Request::DeleteConversationComment {
                project,
                comment: String::new(),
            },
            Request::SyncThreads { project },
            Request::SubmitReview {
                project,
                verdict: vet_protocol::Verdict::Comment,
                body: None,
            },
            Request::PostDrafts { project },
            Request::StageVote {
                project,
                verdict: vet_protocol::Verdict::Comment,
            },
            Request::DiscardVote { project },
            Request::PullRequestInfo { project },
            Request::MarkCommentsSeen { project },
            Request::ConfirmAnchor {
                project,
                revision: 0,
                thread: 0,
            },
            Request::AttachAnchor {
                project,
                revision: 0,
                thread: 0,
                candidate: 0,
            },
        ]
    }

    struct Listener {
        connection: Connection,
        inbox: Receiver<ServerMessage>,
        /// The events the last `heard` took, for checking what they carry.
        last: Vec<Event>,
    }

    impl Listener {
        fn new() -> Self {
            let (events, inbox) = Events::channel();
            Self {
                connection: Connection(events),
                inbox,
                last: Vec::new(),
            }
        }

        /// The events waiting, by kind.
        fn heard(&mut self) -> Vec<Kind> {
            self.last = std::iter::from_fn(|| self.inbox.try_recv().ok())
                .filter_map(|message| match message {
                    ServerMessage::Event { event } => Some(event),
                    _ => None,
                })
                .collect();
            self.last.iter().map(Kind::of).collect()
        }

        /// The `reviewed_at` of each `ReviewChanged` the last `heard` took.
        fn reviewed_at(&self) -> Vec<Option<u32>> {
            (self.last.iter())
                .filter_map(|event| match event {
                    Event::ReviewChanged { reviewed_at, .. } => Some(*reviewed_at),
                    _ => None,
                })
                .collect()
        }
    }

    /// The `version` of each `SnapshotMoved` among `events`.
    fn snapshot_versions(events: &[Event]) -> Vec<Option<u32>> {
        (events.iter())
            .filter_map(|event| match event {
                Event::SnapshotMoved { version, .. } => Some(*version),
                _ => None,
            })
            .collect()
    }

    /// An orchestrator with a project that two connections opened: one asks, the other looks on.
    struct World {
        orchestrator: Orchestrator,
        requester: Listener,
        observer: Listener,
        project: ProjectId,
        /// The requests driven so far, by tag.
        driven: BTreeSet<String>,
    }

    impl World {
        /// Answers a request that only prepares the next one.
        async fn ask(&self, request: Request) -> Response {
            let name = tag(&request);
            (self.orchestrator.answer(request, &self.requester.connection).await)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
        }

        /// Answers a mutation and checks that the requester and the observer both heard what it
        /// declares.
        async fn drive(&mut self, request: Request) -> Response {
            let name = tag(&request);
            let Effect::Mutation {
                events,
                never,
                untested: None,
            } = effect(&request)
            else {
                panic!("{name} is not a mutation the test drives");
            };
            self.requester.heard();
            self.observer.heard();
            let response = self.ask(request).await;
            for (who, listener) in [("requester", &mut self.requester), ("observer", &mut self.observer)] {
                let heard = listener.heard();
                for kind in events {
                    assert!(
                        heard.contains(kind),
                        "{name}: the {who} did not hear {kind:?}, only {heard:?}"
                    );
                }
                for kind in never {
                    assert!(!heard.contains(kind), "{name}: the {who} heard {kind:?}, in {heard:?}");
                }
            }
            self.driven.insert(name);
            response
        }

        async fn manifest(&self) -> Manifest {
            let request = Request::Manifest {
                project: self.project,
                view: View::Changeset,
            };
            match self.ask(request).await {
                Response::Manifest(manifest) => *manifest,
                other => panic!("{other:?}"),
            }
        }

        async fn threads(&self) -> (u32, Vec<Thread>) {
            let request = Request::Threads {
                project: self.project,
                view: View::Changeset,
            };
            match self.ask(request).await {
                Response::Threads { revision, threads } => (revision, threads),
                other => panic!("{other:?}"),
            }
        }

        /// The thread a mutation answers with, checked to carry the revision it was resolved at.
        async fn thread(&mut self, request: Request) -> Thread {
            let revision = self.manifest().await.revision;
            match self.drive(request).await {
                Response::Thread(thread) => {
                    assert_eq!(
                        thread.revision, revision,
                        "the backend did not fill the thread's revision"
                    );
                    thread
                }
                other => panic!("{other:?}"),
            }
        }

        async fn comment(&mut self, path: &str, start: u32, len: u32) -> Thread {
            let revision = self.manifest().await.revision;
            self.thread(Request::CreateThread {
                project: self.project,
                view: View::Changeset,
                revision,
                target: ThreadTarget::Range {
                    side: Side::Target,
                    path: path.into(),
                    start,
                    len,
                },
                body: "why?".into(),
            })
            .await
        }

        /// Waits until the tree has been quiet for a moment, so the revision stays put. Returns the
        /// events the requester heard meanwhile.
        async fn quiet(&mut self) -> Vec<Event> {
            let mut heard = Vec::new();
            while let Ok(Some(message)) =
                tokio::time::timeout(Duration::from_millis(700), self.requester.inbox.recv()).await
            {
                if let ServerMessage::Event { event } = message {
                    heard.push(event);
                }
            }
            heard
        }

        /// Checks that both listeners heard `FilterChanged` at the revision the next manifest has.
        async fn assert_filter_changed_at_the_manifest_revision(&self) {
            let revision = self.manifest().await.revision;
            for (who, listener) in [("requester", &self.requester), ("observer", &self.observer)] {
                let heard: Vec<u32> = (listener.last.iter())
                    .filter_map(|event| match event {
                        Event::FilterChanged { revision, .. } => Some(*revision),
                        _ => None,
                    })
                    .collect();
                assert_eq!(heard, [revision], "the {who}'s FilterChanged");
            }
        }
    }

    #[tokio::test]
    async fn every_mutation_tells_the_requester_and_every_other_subscriber() {
        let repo = tempfile::tempdir().unwrap();
        let p = repo.path();
        git(p, &["init", "-q", "-b", "main"]);
        write(p, "a.txt", &text(30, &[]));
        write(p, "run.txt", "echo\n");
        write(p, "c.txt", "x\n");
        write(p, "d.txt", &text(40, &[]));
        write(p, "e.txt", &text(40, &[]));
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", "base"]);
        git(p, &["checkout", "-q", "-b", "feature"]);
        write(p, "e.txt", &text(40, &[7, 8, 9]));
        git(p, &["commit", "-q", "-a", "-m", "one"]);
        write(p, "e.txt", &text(40, &[7, 8, 9, 30]));
        git(p, &["commit", "-q", "-a", "-m", "two"]);
        // A branch of its own is checked out, so that `main` can move on in a worktree.
        git(p, &["checkout", "-q", "-b", "work", "main"]);
        // The working tree: a hunk to mark and comment on, a mode change to acknowledge, a block
        // to comment on that turns up twice later, and lines whose base side a commit changes.
        write(p, "a.txt", &text(30, &[3]));
        let made_executable = std::process::Command::new("chmod")
            .args(["+x", "run.txt"])
            .current_dir(p)
            .status()
            .unwrap();
        assert!(made_executable.success());
        let block = "dup A\ndup B\ndup C\n";
        write(p, "c.txt", &format!("p1\np2\np3\n{block}q1\nq2\nq3\n"));
        write(p, "d.txt", &text(40, &[7, 8, 9]));

        let data = tempfile::tempdir().unwrap();
        let config = Config {
            debounce: Duration::from_millis(150),
            poll: Duration::from_millis(300),
            ..Config::default()
        };
        let orchestrator = Orchestrator::new(open_store(data.path()).await.unwrap(), config);
        let (requester, observer) = (Listener::new(), Listener::new());
        let root = p.to_str().unwrap();
        let (project, canonical) = orchestrator.open(root, requester.connection.clone()).await.unwrap();
        orchestrator.open(root, observer.connection.clone()).await.unwrap();
        for listener in [&requester, &observer] {
            orchestrator
                .answer(Request::Subscribe, &listener.connection)
                .await
                .unwrap();
        }
        let language = orchestrator.backends[0].config().backend.0.clone();
        let mut world = World {
            orchestrator,
            requester,
            observer,
            project,
            driven: BTreeSet::new(),
        };
        world.quiet().await;

        world.drive(Request::RecordRecentProject { path: root.into() }).await;
        world.drive(Request::ForgetRecentProject { path: root.into() }).await;
        world
            .drive(Request::SetSettings {
                settings: BackendSettings {
                    semantic_cache_mb: 128,
                    finished_review_days: 14,
                },
            })
            .await;
        world
            .drive(Request::SetPreference {
                scope: Some(root.into()),
                key: "tabs".into(),
                value: Some("[]".into()),
            })
            .await;
        world.drive(Request::ClearSemanticCaches).await;
        world
            .drive(Request::ClearProjectCaches {
                root: canonical.clone(),
            })
            .await;

        let revision = world.manifest().await.revision;
        let view = View::Changeset;
        let hunk = || Request::ToggleHunk {
            project,
            view,
            revision,
            path: "d.txt".into(),
            hunk: 0,
        };
        // Only the first seen-mark of a version moves where the review last was; the mark that
        // starts the review is the first.
        world.drive(hunk()).await;
        assert_eq!(world.requester.reviewed_at(), [Some(1)], "the first seen-mark");
        assert_eq!(world.observer.reviewed_at(), [Some(1)], "the first seen-mark, observed");
        world
            .drive(Request::ToggleFlag {
                project,
                view,
                revision,
                path: "d.txt".into(),
                hunk: 0,
            })
            .await;
        assert_eq!(world.requester.reviewed_at(), [None], "a flag");
        world.drive(hunk()).await;
        assert_eq!(world.requester.reviewed_at(), [None], "an unmark");
        world.drive(hunk()).await;
        assert_eq!(
            world.requester.reviewed_at(),
            [None],
            "a seen-mark in the version marked in already"
        );
        for request in [
            Request::ToggleItem {
                project,
                view,
                revision,
                path: "run.txt".into(),
                kind: NonTextKind::Mode,
            },
            Request::ToggleFileSeen {
                project,
                view,
                revision,
                path: "a.txt".into(),
            },
            Request::SetFilesSeen {
                project,
                view,
                revision,
                paths: vec!["a.txt".into(), "c.txt".into()],
                seen: true,
            },
        ] {
            world.drive(request).await;
        }

        let inferred = world.comment("a.txt", 10, 3).await;
        let ambiguous = world.comment("c.txt", 3, 3).await;
        let replied = world
            .thread(Request::Reply {
                project,
                thread: inferred.id,
                body: "because".into(),
            })
            .await;
        world
            .drive(Request::EditMessage {
                project,
                message: inferred.messages[0].id,
                body: "why, really?".into(),
            })
            .await;
        world
            .drive(Request::RemoveMessage {
                project,
                message: replied.messages[1].id,
            })
            .await;
        let doomed = world.comment("a.txt", 20, 1).await;
        world
            .drive(Request::SetThreadStatus {
                project,
                thread: doomed.id,
                status: ThreadStatus::Resolved,
            })
            .await;
        world
            .drive(Request::DeleteThread {
                project,
                thread: doomed.id,
            })
            .await;

        // The comment target carries the revision it was computed at.
        let target = world
            .ask(Request::CommentTarget {
                project,
                view,
                revision,
                path: "a.txt".into(),
                at: CommentAt::File,
            })
            .await;
        let Response::CommentTarget { revision: answered, .. } = target else {
            panic!("{target:?}");
        };
        assert_eq!(
            answered, revision,
            "the backend did not fill the comment target's revision"
        );

        // An edit inside one anchor makes it inferred; the block twice makes the other ambiguous.
        write(p, "a.txt", &text(30, &[3, 11]));
        write(
            p,
            "c.txt",
            &format!("x1\nx2\nx3\n{block}x4\nx5\nx6\nx7\nx8\n{block}y1\ny2\ny3\n"),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let mut moved = Vec::new();
        let revision = loop {
            assert!(tokio::time::Instant::now() < deadline, "the anchors did not move");
            moved.extend(world.quiet().await);
            let (revision, threads) = world.threads().await;
            let anchor = |id| threads.iter().find(|t| t.id == id).and_then(|t| t.anchor.clone());
            if matches!(anchor(inferred.id), Some(AnchorState::Inferred { .. }))
                && matches!(anchor(ambiguous.id), Some(AnchorState::Ambiguous { .. }))
            {
                break revision;
            }
        };
        // A settle with a review open records a version, which the event names.
        let Response::Versions { versions } = world.ask(Request::Versions { project }).await else {
            panic!("not the versions");
        };
        let latest = versions.last().map(|v| v.n);
        assert!(latest > Some(1), "the settle recorded no version");
        assert_eq!(
            snapshot_versions(&moved).last(),
            Some(&latest),
            "a settle with a review open"
        );
        world
            .drive(Request::ConfirmAnchor {
                project,
                revision,
                thread: inferred.id,
            })
            .await;
        world
            .drive(Request::AttachAnchor {
                project,
                revision,
                thread: ambiguous.id,
                candidate: 1,
            })
            .await;

        world
            .drive(Request::SetServerPath {
                project,
                language: language.clone(),
                path: Some("/nonexistent/server".into()),
            })
            .await;
        // The repository has no file of the language, so nothing is started to fail.
        world.drive(Request::RestartServer { project, language }).await;

        let filter = ReviewFilter {
            exclude: vec!["generated/**".into()],
            ..ReviewFilter::default()
        };
        world.drive(Request::SetProjectFilter { project, filter }).await;
        world.assert_filter_changed_at_the_manifest_revision().await;
        let narrowing = ReviewFilter {
            roots: vec!["src".into()],
            ..ReviewFilter::default()
        };
        world
            .drive(Request::NarrowReview {
                project,
                changeset: ChangesetKey::WorkingTree,
                filter: Some(narrowing),
            })
            .await;
        world.assert_filter_changed_at_the_manifest_revision().await;
        world.drive(Request::SetProjectTrusted { project, trusted: true }).await;
        world
            .drive(Request::ExcludePath {
                project,
                path: "notes".into(),
                directory: true,
            })
            .await;
        world.assert_filter_changed_at_the_manifest_revision().await;

        world.drive(Request::FinishReview { project }).await;
        let finished = world.manifest().await.revision;
        for (who, listener) in [("requester", &world.requester), ("observer", &world.observer)] {
            assert!(
                listener
                    .last
                    .iter()
                    .any(|e| matches!(e, Event::ReviewFinished { revision, .. } if *revision == finished)),
                "the {who}'s ReviewFinished is not at the manifest's revision"
            );
        }

        // Browsing records no version.
        write(p, "a.txt", &text(30, &[3, 12]));
        let browsing = world.quiet().await;
        let versions = snapshot_versions(&browsing);
        assert!(!versions.is_empty(), "the edit did not settle");
        assert!(
            versions.iter().all(Option::is_none),
            "a settle while browsing: {versions:?}"
        );

        let scratch = tempfile::tempdir().unwrap();
        let remote = scratch.path().join("remote.git");
        git(scratch.path(), &["init", "-q", "--bare", remote.to_str().unwrap()]);
        git(p, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(p, &["push", "-q", "origin", "main"]);
        world.drive(Request::Fetch { project }).await;

        world
            .drive(Request::OpenChangeset {
                project,
                changeset: ChangesetKey::Branch { name: "feature".into() },
            })
            .await;
        // The branch is rebased onto a `main` that dropped a line the branch edited, keeping the
        // branch's version: what was seen is now seen against another base.
        let revision = world.manifest().await.revision;
        world
            .ask(Request::ToggleHunk {
                project,
                view,
                revision,
                path: "e.txt".into(),
                hunk: 0,
            })
            .await;
        let elsewhere = scratch.path().join("elsewhere");
        let dir = elsewhere.to_str().unwrap();
        git(p, &["worktree", "add", "-q", dir, "main"]);
        write(&elsewhere, "e.txt", &text(40, &[]).replace("line 8\n", ""));
        git(&elsewhere, &["commit", "-q", "-a", "-m", "main drops a line"]);
        git(p, &["worktree", "remove", "--force", dir]);
        git(p, &["worktree", "add", "-q", dir, "feature"]);
        git(&elsewhere, &["rebase", "-q", "-X", "theirs", "main"]);
        git(p, &["worktree", "remove", "--force", dir]);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let revision = loop {
            world.quiet().await;
            let manifest = world.manifest().await;
            assert!(
                tokio::time::Instant::now() < deadline,
                "the base did not move: {:?} {:?}",
                manifest.base.commit.subject,
                manifest
                    .files
                    .iter()
                    .map(|f| (&f.path, &f.review.hunks))
                    .collect::<Vec<_>>()
            );
            let moved = manifest
                .files
                .iter()
                .find(|f| f.path == "e.txt")
                .is_some_and(|f| f.review.hunks.contains(&vet_protocol::HunkState::SeenAgainstOldBase));
            if moved {
                break manifest.revision;
            }
        };
        let diff = world
            .ask(Request::BaseDiff {
                project,
                revision,
                path: "e.txt".into(),
                hunk: 0,
            })
            .await;
        let Response::BaseDiff(diff) = diff else {
            panic!("{diff:?}");
        };
        assert_eq!(
            diff.revision, revision,
            "the backend did not fill the base diff's revision"
        );
        world
            .drive(Request::ReconfirmFile {
                project,
                revision,
                path: "e.txt".into(),
            })
            .await;
        let healed = world.manifest().await;
        assert!(
            healed
                .files
                .iter()
                .find(|f| f.path == "e.txt")
                .is_some_and(|f| f.review.hunks[0] == vet_protocol::HunkState::Seen)
        );

        world
            .drive(Request::SetLens {
                project,
                lens: Lens::PerCommit,
            })
            .await;

        world.drive(Request::RemoveProject { root: canonical }).await;

        // Every request is classified, and every mutation that can be driven was.
        let samples = samples();
        let sampled: BTreeSet<String> = samples.iter().map(tag).collect();
        assert_eq!(sampled, every_tag(), "a request has no sample");
        for request in &samples {
            if let Effect::Mutation { untested: None, .. } = effect(request) {
                let name = tag(request);
                assert!(world.driven.contains(&name), "{name} was not driven");
            }
        }
    }
}
