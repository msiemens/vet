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
    SnapshotSettled,
    EditsPending,
    ChangesetOpened,
    ReviewChanged,
    LensChanged,
    TargetChanged,
    ChangesetsChanged,
    ReviewStarted,
    ThreadsChanged,
    ServerInstallOutput,
    ServersChanged,
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
            Event::SnapshotSettled { .. } => Self::SnapshotSettled,
            Event::EditsPending { .. } => Self::EditsPending,
            Event::ChangesetOpened { .. } => Self::ChangesetOpened,
            Event::ReviewChanged { .. } => Self::ReviewChanged,
            Event::LensChanged { .. } => Self::LensChanged,
            Event::TargetChanged { .. } => Self::TargetChanged,
            Event::ChangesetsChanged { .. } => Self::ChangesetsChanged,
            Event::ReviewStarted { .. } => Self::ReviewStarted,
            Event::ThreadsChanged { .. } => Self::ThreadsChanged,
            Event::ServerInstallOutput { .. } => Self::ServerInstallOutput,
            Event::ServersChanged { .. } => Self::ServersChanged,
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
    /// (`CredentialsNeeded`); nothing another store shows changes.
    Query,
    /// Changes what the backend keeps, but nothing a store keeps; why.
    Unannounced(#[expect(dead_code, reason = "read by whoever reads the classification")] &'static str),
    /// Changes what clients show. When it did what it is for, every subscriber, the requester
    /// included, hears each of `events`: those of a project its connections, those of the process
    /// every connection that asked for `Accounts` or `RecentProjects`. `untested` says why the test
    /// cannot drive it, if it cannot.
    Mutation {
        events: &'static [Kind],
        untested: Option<&'static str>,
    },
}

const fn tested(events: &'static [Kind]) -> Effect {
    Effect::Mutation { events, untested: None }
}

const fn untested(events: &'static [Kind], why: &'static str) -> Effect {
    Effect::Mutation {
        events,
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
        | Request::RecentProjects
        | Request::Accounts
        | Request::SemanticCacheUsage
        | Request::Settings
        | Request::LogDirectory
        | Request::OpenProject { .. }
        | Request::ListChangesets { .. }
        // Also deletes the refs of pull requests no longer listed, which nothing shows.
        | Request::ListPullRequests { .. }
        | Request::ChangesetLines { .. }
        | Request::Checks { .. }
        | Request::CheckLog { .. }
        | Request::Tasks { .. }
        | Request::Versions { .. }
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
        | Request::ListDirectory { .. }
        | Request::ListFiles { .. }
        | Request::FileText { .. }
        | Request::ItemBlob { .. }
        | Request::Threads { .. }
        | Request::TargetLine { .. }
        | Request::CommentTarget { .. }
        | Request::PullRequestInfo { .. } => Effect::Query,
        Request::CloseProject { .. } => {
            Effect::Unannounced("unsubscribes the connection; the others see nothing change")
        }
        Request::ClearSemanticCaches => Effect::Unannounced(
            "only speed is lost; the disk use it answers with is read by the Settings page when it opens",
        ),
        Request::SetSettings { .. } => {
            Effect::Unannounced("read by the Settings pages when they open; nothing else shows them")
        }
        Request::RecordRecentProject { .. } | Request::ForgetRecentProject { .. } => tested(&[RecentProjectsChanged]),
        // Not for a token handed back unchecked that is held already. The open pull requests of
        // the host are synced again, in the background.
        Request::SetCredentials { .. } | Request::RemoveCredentials { .. } => {
            untested(&[AccountsChanged], NEEDS_THE_ACCOUNTS)
        }
        // When another changeset comes on view; reopening the pull request on view announces
        // `TargetChanged` if the platform names it differently now.
        Request::OpenChangeset { .. } => tested(&[ChangesetOpened]),
        Request::FinishReview { .. } => tested(&[SnapshotSettled, ChangesetsChanged]),
        Request::SetLens { .. } => tested(&[LensChanged]),
        // The first mark of a review also announces `ReviewStarted`.
        Request::ToggleHunk { .. }
        | Request::ToggleFlag { .. }
        | Request::ToggleItem { .. }
        | Request::ToggleFileSeen { .. }
        | Request::SetFilesSeen { .. }
        | Request::ReconfirmFile { .. } => tested(&[ReviewChanged]),
        Request::SetProjectFilter { .. } | Request::ExcludePath { .. } => tested(&[SnapshotSettled]),
        Request::SetProjectTrusted { .. } => tested(&[TargetChanged, ServersChanged]),
        Request::TrustPullRequest { .. } => untested(&[TargetChanged, ServersChanged], NEEDS_A_PLATFORM),
        // When a remote was fetched. A pull request on view whose head moved settles
        // (`SnapshotSettled`), one the platform names differently says `TargetChanged`, and its
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
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::time::Duration;

    use tokio::sync::mpsc::UnboundedReceiver;
    use vet_protocol::{
        AnchorState, BackendSettings, ChangesetKey, CommentAt, EditChoice, Lens, Manifest, NonTextKind, ProjectId,
        Response, ReviewFilter, Secret, ServerMessage, Side, SymbolId, Thread, ThreadStatus, ThreadTarget, View,
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
            Request::SemanticCacheUsage,
            Request::Settings,
            Request::SetSettings {
                settings: BackendSettings {
                    semantic_cache_mb: 1,
                    finished_review_days: 1,
                },
            },
            Request::LogDirectory,
            Request::OpenProject { repo: path() },
            Request::CloseProject { project },
            Request::ListChangesets { project },
            Request::ListPullRequests { project },
            Request::ChangesetLines {
                project,
                changeset: ChangesetKey::WorkingTree,
            },
            Request::Checks {
                project,
                commits: vec![],
            },
            Request::CheckLog {
                project,
                run: String::new(),
            },
            Request::OpenChangeset {
                project,
                changeset: ChangesetKey::WorkingTree,
            },
            Request::FinishReview { project },
            Request::Tasks { project },
            Request::Versions { project },
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
            Request::Fetch { project },
            Request::SetProjectFilter {
                project,
                filter: ReviewFilter::default(),
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
            Request::FileText {
                project,
                view,
                path: path(),
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
                verdict: String::new(),
                body: None,
            },
            Request::PostDrafts { project },
            Request::StageVote {
                project,
                verdict: String::new(),
            },
            Request::DiscardVote { project },
            Request::PullRequestInfo { project },
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
        inbox: UnboundedReceiver<ServerMessage>,
    }

    impl Listener {
        fn new() -> Self {
            let (events, inbox) = Events::channel();
            Self {
                connection: Connection(events),
                inbox,
            }
        }

        /// The events waiting, by kind.
        fn heard(&mut self) -> Vec<Kind> {
            std::iter::from_fn(|| self.inbox.try_recv().ok())
                .filter_map(|message| match message {
                    ServerMessage::Event { event } => Some(Kind::of(&event)),
                    _ => None,
                })
                .collect()
        }
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
            let Effect::Mutation { events, untested: None } = effect(&request) else {
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

        async fn thread(&mut self, request: Request) -> Thread {
            match self.drive(request).await {
                Response::Thread(thread) => thread,
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

        /// Waits until the tree has been quiet for a moment, so the revision stays put.
        async fn quiet(&mut self) {
            while tokio::time::timeout(Duration::from_millis(700), self.requester.inbox.recv())
                .await
                .is_ok()
            {}
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
        let (project, _) = orchestrator.open(root, requester.connection.clone()).await.unwrap();
        orchestrator.open(root, observer.connection.clone()).await.unwrap();
        for listener in [&requester, &observer] {
            orchestrator
                .answer(Request::RecentProjects, &listener.connection)
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

        let revision = world.manifest().await.revision;
        let view = View::Changeset;
        for request in [
            Request::ToggleHunk {
                project,
                view,
                revision,
                path: "d.txt".into(),
                hunk: 0,
            },
            Request::ToggleFlag {
                project,
                view,
                revision,
                path: "d.txt".into(),
                hunk: 0,
            },
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

        // An edit inside one anchor makes it inferred; the block twice makes the other ambiguous.
        write(p, "a.txt", &text(30, &[3, 11]));
        write(
            p,
            "c.txt",
            &format!("x1\nx2\nx3\n{block}x4\nx5\nx6\nx7\nx8\n{block}y1\ny2\ny3\n"),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        let revision = loop {
            assert!(tokio::time::Instant::now() < deadline, "the anchors did not move");
            world.quiet().await;
            let (revision, threads) = world.threads().await;
            let anchor = |id| threads.iter().find(|t| t.id == id).and_then(|t| t.anchor.clone());
            if matches!(anchor(inferred.id), Some(AnchorState::Inferred { .. }))
                && matches!(anchor(ambiguous.id), Some(AnchorState::Ambiguous { .. }))
            {
                break revision;
            }
        };
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
        world.drive(Request::SetProjectTrusted { project, trusted: true }).await;
        world
            .drive(Request::ExcludePath {
                project,
                path: "notes".into(),
                directory: true,
            })
            .await;

        world.drive(Request::FinishReview { project }).await;

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
        assert!(matches!(diff, Response::BaseDiff(_)), "{diff:?}");
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
