//! The handler against a real temporary repository: open, manifest, mark,
//! edit, settle, remap, revert (SPEC §12 criterion 1's split and revert cases,
//! and criterion 4's settle path). `git` is used only to build the fixture.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use vet_backend::{Config, Orchestrator, open_store};
use vet_protocol::{
    AnchorState, Base, BaseChoice, BlobContent, ChangesetKey, Completion, DirEntry, DirEntryKind, Divergence, Event,
    FileStatus, HunkState, LineDelta, LineLocation, Manifest, NoTextReason, NonTextKind, ProjectId, Request, Response,
    ReviewFilter, ReviewInfo, ReviewUpdate, RowReview, ServerMessage, Side, TextContent, Thread, ThreadStatus,
    ThreadTarget,
};
use vet_transport::{Events, Handler};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn write(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
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

struct Client {
    handler: std::sync::Arc<Orchestrator>,
    events: Events,
    inbox: UnboundedReceiver<ServerMessage>,
    /// What `ReviewStarted` events said, which `event` and `no_event` skip.
    started: Vec<ReviewInfo>,
}

impl Client {
    fn new(handler: &std::sync::Arc<Orchestrator>) -> Self {
        let (events, inbox) = Events::channel();
        Self {
            handler: handler.clone(),
            events,
            inbox,
            started: Vec::new(),
        }
    }

    async fn call(&self, request: Request) -> Result<Response, String> {
        self.handler.handle(request, self.events.clone()).await
    }

    async fn manifest(&self, project: ProjectId) -> Manifest {
        match self
            .call(Request::Manifest {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .unwrap()
        {
            Response::Manifest(m) => *m,
            other => panic!("{other:?}"),
        }
    }

    async fn toggle_hunk(&self, project: ProjectId, revision: u32, hunk: u32) -> ReviewUpdate {
        let request = Request::ToggleHunk {
            view: vet_protocol::View::Changeset,
            project,
            revision,
            path: "a.txt".into(),
            hunk,
        };
        match self.call(request).await.unwrap() {
            Response::Review(update) => update,
            other => panic!("{other:?}"),
        }
    }

    /// The next event other than `ReviewStarted` and `Task`; panics if none arrives.
    async fn event(&mut self) -> Event {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), self.inbox.recv())
                .await
                .expect("no event")
                .expect("connection closed");
            match message {
                ServerMessage::Event {
                    event: Event::ReviewStarted { review, .. },
                } => self.started.push(review),
                ServerMessage::Event {
                    event: Event::Task(_), ..
                } => {}
                ServerMessage::Event { event } => return event,
                other => panic!("{other:?}"),
            }
        }
    }

    /// The reviews that `ReviewStarted` events have announced so far.
    fn started(&mut self) -> Vec<ReviewInfo> {
        while let Ok(message) = self.inbox.try_recv() {
            if let ServerMessage::Event {
                event: Event::ReviewStarted { review, .. },
            } = message
            {
                self.started.push(review);
            }
        }
        self.started.clone()
    }

    /// The revision of the next `SnapshotSettled`, past the `ReviewChanged` and `ThreadsChanged`
    /// that the client's own marks and comments announced to it.
    async fn settled(&mut self) -> u32 {
        loop {
            match self.event().await {
                Event::SnapshotSettled { revision, .. } => return revision,
                Event::ReviewChanged { .. } | Event::ThreadsChanged { .. } => {}
                other => panic!("{other:?}"),
            }
        }
    }

    /// Whether no event other than `ReviewStarted` and `Task` is waiting.
    fn no_event(&mut self) -> bool {
        while let Ok(message) = self.inbox.try_recv() {
            match message {
                ServerMessage::Event {
                    event: Event::ReviewStarted { review, .. },
                } => self.started.push(review),
                ServerMessage::Event {
                    event: Event::Task(_), ..
                } => {}
                _ => return false,
            }
        }
        true
    }
}

fn hunk_states(m: &Manifest) -> Vec<HunkState> {
    m.files[0].review.hunks.clone()
}

struct Fixture {
    repo: tempfile::TempDir,
    _data: tempfile::TempDir,
    handler: std::sync::Arc<Orchestrator>,
}

async fn fixture(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Fixture {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    write(repo.path(), ".gitignore", "target/\n");
    for (name, content) in base {
        write(repo.path(), name, content);
    }
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    for (name, content) in dirty {
        write(repo.path(), name, content);
    }
    let data = tempfile::tempdir().unwrap();
    let store = open_store(data.path()).await.unwrap();
    let config = Config {
        debounce: Duration::from_millis(150),
        poll: Duration::from_millis(300),
        orphan_grace: Duration::from_millis(400),
        ..Config::default()
    };
    Fixture {
        repo,
        _data: data,
        handler: std::sync::Arc::new(Orchestrator::new(store, config)),
    }
}

async fn open(client: &Client, repo: &Path) -> ProjectId {
    let request = Request::OpenProject {
        repo: repo.to_str().unwrap().into(),
    };
    match client.call(request).await.unwrap() {
        Response::Project { project, .. } => project,
        other => panic!("{other:?}"),
    }
}

fn lines(c: &Completion) -> (u32, u32) {
    (c.lines, c.seen_lines)
}

impl Client {
    async fn thread(&self, request: Request) -> Thread {
        match self.call(request).await.unwrap() {
            Response::Thread(thread) => thread,
            other => panic!("{other:?}"),
        }
    }

    async fn threads(&self, project: ProjectId) -> (u32, Vec<Thread>) {
        match self
            .call(Request::Threads {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .unwrap()
        {
            Response::Threads { revision, threads } => (revision, threads),
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn a_base_line_is_placed_in_the_target_text_where_a_unified_diff_shows_it() {
    // Base lines 3 and 4 became one line, at 3.
    let base = text(10, &[]);
    let dirty = base.replace("line 3\nline 4\n", "changed\n");
    let fx = fixture(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    let client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    let place = |side, line, revision| {
        client.call(Request::TargetLine {
            view: vet_protocol::View::Changeset,
            project,
            revision,
            side,
            path: "a.txt".into(),
            line,
        })
    };
    let at = |line| Ok(Response::TargetLine { revision: 0, line });

    assert_eq!(place(Side::Base, 4, 0).await, at(3));
    assert_eq!(place(Side::Base, 8, 0).await, at(7));
    assert_eq!(place(Side::Target, 7, 0).await, at(7));
    assert!(place(Side::Base, 4, 1).await.is_err(), "a stale revision is refused");
}

#[tokio::test]
async fn threads_follow_edits_and_notify_every_connection() {
    let dirty = text(30, &[3]);
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[("a.txt", &dirty)]).await;
    let mut one = Client::new(&fx.handler);
    let mut two = Client::new(&fx.handler);
    let project = open(&one, fx.repo.path()).await;
    assert_eq!(open(&two, fx.repo.path()).await, project);

    let created = one
        .thread(Request::CreateThread {
            view: vet_protocol::View::Changeset,
            project,
            revision: 0,
            target: ThreadTarget::Range {
                side: Side::Target,
                path: "a.txt".into(),
                start: 10,
                len: 3,
            },
            body: "why?".into(),
        })
        .await;
    assert_eq!(
        created.anchor,
        Some(AnchorState::Exact {
            whitespace_changed: false
        })
    );
    assert_eq!(created.messages.len(), 1);
    assert!(!created.messages[0].author.is_empty());
    let Event::ThreadsChanged { revision: 0, .. } = two.event().await else {
        panic!("no threads event")
    };
    let Event::ThreadsChanged { revision: 0, .. } = one.event().await else {
        panic!("the requester is told too")
    };

    // An edit above shifts the lines; the next fetch resolves them at the new revision.
    write(fx.repo.path(), "a.txt", &format!("new 1\nnew 2\n{dirty}"));
    assert_eq!(one.settled().await, 1);
    assert_eq!(two.settled().await, 1);
    let (revision, threads) = one.threads(project).await;
    assert_eq!(revision, 1);
    let at = |start| {
        Some(LineLocation {
            path: "a.txt".into(),
            start,
            len: 3,
        })
    };
    assert_eq!(
        threads[0].anchor,
        Some(AnchorState::Exact {
            whitespace_changed: false
        })
    );
    assert_eq!(
        (threads[0].location.clone(), threads[0].original.clone()),
        (at(12), at(10))
    );

    // An edit inside the range makes the anchor inferred, and confirming it re-anchors.
    write(
        fx.repo.path(),
        "a.txt",
        &format!("new 1\nnew 2\n{}", dirty.replace("line 11", "changed")),
    );
    assert_eq!(one.settled().await, 2);
    two.settled().await;
    let (_, threads) = one.threads(project).await;
    assert!(
        matches!(threads[0].anchor, Some(AnchorState::Inferred { .. })),
        "{:?}",
        threads[0].anchor
    );
    let id = threads[0].id;
    let confirmed = one
        .thread(Request::ConfirmAnchor {
            project,
            revision: 2,
            thread: id,
        })
        .await;
    assert_eq!(
        confirmed.anchor,
        Some(AnchorState::Exact {
            whitespace_changed: false
        })
    );
    assert_eq!(confirmed.location, at(12));
    assert!(matches!(two.event().await, Event::ThreadsChanged { revision: 2, .. }));

    // Reply, resolve and delete reach the other connection too.
    let replied = one
        .thread(Request::Reply {
            project,
            thread: id,
            body: "because".into(),
        })
        .await;
    assert_eq!(replied.messages.len(), 2);
    let resolved = one
        .thread(Request::SetThreadStatus {
            project,
            thread: id,
            status: ThreadStatus::Resolved,
        })
        .await;
    assert_eq!(resolved.status, ThreadStatus::Resolved);
    one.call(Request::DeleteThread { project, thread: id }).await.unwrap();
    for _ in 0..3 {
        assert!(matches!(two.event().await, Event::ThreadsChanged { .. }));
    }
    assert!(one.threads(project).await.1.is_empty());
}

#[tokio::test]
async fn mark_edit_settle_remap_and_revert() {
    let base = text(40, &[]);
    let dirty = text(40, &[5]);
    let fx = fixture(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;

    let m = client.manifest(project).await;
    assert_eq!((m.revision, m.files.len()), (0, 1));
    assert_eq!(m.files[0].status, FileStatus::Modified);
    assert_eq!(hunk_states(&m), [HunkState::Unseen]);
    assert_eq!(lines(&m.completion), (2, 0));
    assert!(m.degraded.is_none());

    // Mark the hunk seen.
    let update = client.toggle_hunk(project, 0, 0).await;
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    assert_eq!(lines(&update.changeset), (2, 2));

    // The agent edits elsewhere: the mark survives, the new hunk is unseen.
    write(fx.repo.path(), "a.txt", &text(40, &[5, 30]));
    assert_eq!(client.settled().await, 1);
    let m = client.manifest(project).await;
    assert_eq!(m.revision, 1);
    assert_eq!(hunk_states(&m), [HunkState::Seen, HunkState::Unseen]);
    assert_eq!(lines(&m.completion), (4, 2));

    // A request made against revision 0 is rejected, and changes nothing.
    let stale = Request::ToggleHunk {
        view: vet_protocol::View::Changeset,
        project,
        revision: 0,
        path: "a.txt".into(),
        hunk: 1,
    };
    assert!(client.call(stale).await.unwrap_err().contains("stale"));

    // Editing the marked line uncovers it; restoring it, byte for byte, covers it again.
    write(
        fx.repo.path(),
        "a.txt",
        &text(40, &[30]).replace("line 5\n", "something else\n"),
    );
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(hunk_states(&m), [HunkState::Unseen, HunkState::Unseen]);
    write(fx.repo.path(), "a.txt", &text(40, &[5, 30]));
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(hunk_states(&m), [HunkState::Seen, HunkState::Unseen]);

    // Reverting the second edit leaves the original state, and the file text is current.
    write(fx.repo.path(), "a.txt", &dirty);
    let revision = client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(hunk_states(&m), [HunkState::Seen]);
    assert_eq!(lines(&m.completion), (2, 2));
    let Response::FileContents {
        revision: at,
        base: b,
        target: t,
        ..
    } = client
        .call(Request::FileContents {
            view: vet_protocol::View::Changeset,
            project,
            path: "a.txt".into(),
        })
        .await
        .unwrap()
    else {
        panic!("not file contents");
    };
    assert_eq!(
        (at, b.as_deref(), t.as_deref()),
        (revision, Some(base.as_str()), Some(dirty.as_str()))
    );

    // Un-marking.
    let update = client.toggle_hunk(project, revision, 0).await;
    assert_eq!(update.review.hunks, [HunkState::Unseen]);
    assert_eq!(lines(&update.changeset), (2, 0));

    client.call(Request::CloseProject { project }).await.unwrap();
    assert!(
        client
            .call(Request::Manifest {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .is_err()
    );
}

/// Whether the content store in `data` holds `content`, waiting up to 10 s for it.
async fn stored(data: &Path, content: &str) -> bool {
    let hex = vet_core::content::BlobId::of(content.as_bytes()).0.to_hex();
    let path = data.join("content").join(&hex[..2]).join(&hex[2..]);
    for _ in 0..200 {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn the_working_trees_content_is_copied_into_the_store_before_a_review_starts() {
    let base = text(40, &[]);
    let dirty = text(40, &[5]);
    let fx = fixture(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    client.manifest(project).await;
    assert!(stored(fx._data.path(), &dirty).await, "copied once the project opened");

    let edited = text(40, &[5, 30]);
    write(fx.repo.path(), "a.txt", &edited);
    client.settled().await;
    assert!(stored(fx._data.path(), &edited).await, "copied once the tree settled");
    assert_eq!(client.manifest(project).await.review, ReviewInfo::Browsing);
}

#[tokio::test]
async fn a_seen_region_with_an_edit_in_the_middle_is_split() {
    let region: Vec<usize> = (5..25).collect();
    let dirty = text(40, &region);
    let fx = fixture(&[("a.txt", &text(40, &[]))], &[("a.txt", &dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    client.toggle_hunk(project, 0, 0).await;

    let middle = dirty
        .replace("edited 14", "mid a")
        .replace("edited 15", "mid b")
        .replace("edited 16", "mid c");
    write(fx.repo.path(), "a.txt", &middle);
    client.settled().await;

    let m = client.manifest(project).await;
    assert_eq!(hunk_states(&m), [HunkState::Unseen]);
    // All 20 base lines are still seen, and 17 of the 20 target lines.
    assert_eq!(lines(&m.completion), (40, 37));
}

#[tokio::test]
async fn non_text_changes_are_acknowledged_and_every_connection_is_told() {
    let fx = fixture(
        &[("run.sh", "echo\n"), ("a.txt", &text(10, &[]))],
        &[("a.txt", &text(10, &[2]))],
    )
    .await;
    Command::new("chmod")
        .args(["+x", "run.sh"])
        .current_dir(fx.repo.path())
        .status()
        .unwrap();
    let mut first = Client::new(&fx.handler);
    let mut second = Client::new(&fx.handler);
    let project = open(&first, fx.repo.path()).await;
    assert_eq!(open(&second, fx.repo.path()).await, project, "same repo, same project");

    let m = first.manifest(project).await;
    let run = m.files.iter().find(|f| f.path == "run.sh").unwrap();
    assert_eq!(run.non_text.len(), 1);
    assert_eq!(run.non_text[0].kind, NonTextKind::Mode);
    assert_eq!(run.non_text[0].after.as_ref().unwrap().mode, 0o100755);
    assert_eq!((m.completion.items, m.completion.acknowledged_items), (1, 0));

    let request = Request::ToggleItem {
        view: vet_protocol::View::Changeset,
        project,
        revision: 0,
        path: "run.sh".into(),
        kind: NonTextKind::Mode,
    };
    let Response::Review(update) = first.call(request).await.unwrap() else {
        panic!("not a review update");
    };
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);

    for client in [&mut second, &mut first] {
        match client.event().await {
            Event::ReviewChanged {
                revision, update: u, ..
            } => assert_eq!((revision, u), (0, update.clone())),
            other => panic!("{other:?}"),
        }
    }

    // Marking a hunk is reported the same way.
    first.toggle_hunk(project, 0, 0).await;
    assert!(matches!(second.event().await, Event::ReviewChanged { .. }));

    // The first connection leaves; the project lives on for the second.
    first.call(Request::CloseProject { project }).await.unwrap();
    assert_eq!(second.manifest(project).await.completion.acknowledged_items, 1);
}

#[tokio::test]
async fn empty_files_are_non_text_items_and_can_be_acknowledged() {
    let fx = fixture(
        &[("gone.txt", ""), ("emptied.txt", &text(3, &[]))],
        &[("new.txt", ""), ("emptied.txt", "")],
    )
    .await;
    std::fs::remove_file(fx.repo.path().join("gone.txt")).unwrap();
    let client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;

    let m = client.manifest(project).await;
    let file = |path: &str| m.files.iter().find(|f| f.path == path).unwrap();
    let (new, gone) = (file("new.txt"), file("gone.txt"));
    assert_eq!(
        (new.status, new.non_text.len(), new.non_text[0].kind),
        (FileStatus::Added, 1, NonTextKind::Empty)
    );
    assert!(new.non_text[0].before.is_none() && new.non_text[0].after.is_some());
    assert_eq!(
        (gone.status, gone.non_text.len(), gone.non_text[0].kind),
        (FileStatus::Deleted, 1, NonTextKind::Empty)
    );
    // A file with text on one side keeps its text diff and is not also an item.
    let emptied = file("emptied.txt");
    assert!(emptied.non_text.is_empty());
    assert_eq!(emptied.review.hunks.len(), 1);
    assert_eq!((m.completion.items, m.completion.acknowledged_items), (2, 0));

    let request = Request::ToggleItem {
        view: vet_protocol::View::Changeset,
        project,
        revision: 0,
        path: "new.txt".into(),
        kind: NonTextKind::Empty,
    };
    let Response::Review(update) = client.call(request).await.unwrap() else {
        panic!("not a review update");
    };
    assert_eq!(update.review.acknowledged, [NonTextKind::Empty]);
    assert_eq!(client.manifest(project).await.completion.acknowledged_items, 1);
}

#[tokio::test]
async fn binary_items_serve_the_bytes_of_either_side() {
    let big = format!("\0{}", "x".repeat(8 * 1024 * 1024));
    let fx = fixture(
        &[("logo.PNG", "\0old"), ("gone.gif", "\0gone")],
        &[("logo.PNG", "\0new")],
    )
    .await;
    write(fx.repo.path(), "new.ico", "\0ico");
    write(fx.repo.path(), "big.bin", &big);
    std::fs::remove_file(fx.repo.path().join("gone.gif")).unwrap();
    let client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    let blob = |revision, path: &str, side| {
        let request = Request::ItemBlob {
            view: vet_protocol::View::Changeset,
            project,
            revision,
            path: path.into(),
            kind: NonTextKind::Binary,
            side,
        };
        client.call(request)
    };
    let bytes = |media_type: &str, content: &[u8]| {
        use base64::Engine;
        Response::ItemBlob {
            content: BlobContent::Bytes {
                media_type: Some(media_type.into()),
                bytes: base64::engine::general_purpose::STANDARD.encode(content),
            },
        }
    };
    let absent = Response::ItemBlob {
        content: BlobContent::Absent,
    };

    assert_eq!(
        blob(0, "logo.PNG", Side::Base).await.unwrap(),
        bytes("image/png", b"\0old")
    );
    assert_eq!(
        blob(0, "logo.PNG", Side::Target).await.unwrap(),
        bytes("image/png", b"\0new")
    );
    assert_eq!(blob(0, "new.ico", Side::Base).await.unwrap(), absent);
    assert_eq!(
        blob(0, "new.ico", Side::Target).await.unwrap(),
        bytes("image/x-icon", b"\0ico")
    );
    assert_eq!(
        blob(0, "gone.gif", Side::Base).await.unwrap(),
        bytes("image/gif", b"\0gone")
    );
    assert_eq!(blob(0, "gone.gif", Side::Target).await.unwrap(), absent);
    assert_eq!(
        blob(0, "big.bin", Side::Target).await.unwrap(),
        Response::ItemBlob {
            content: BlobContent::TooLarge
        }
    );

    assert!(
        blob(1, "logo.PNG", Side::Base)
            .await
            .unwrap_err()
            .starts_with("stale revision")
    );
    assert!(blob(0, "a.txt", Side::Base).await.is_err());
}

#[tokio::test]
async fn ignored_paths_do_not_settle_snapshots() {
    let fx = fixture(&[("a.txt", &text(5, &[]))], &[]).await;
    let mut client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;

    write(fx.repo.path(), "target/debug/out", "build output\n");
    write(fx.repo.path(), "target/debug/more", "more\n");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(client.no_event(), "ignored writes must not produce snapshots");

    write(fx.repo.path(), "a.txt", &text(5, &[1]));
    assert_eq!(client.settled().await, 1);
    let m = client.manifest(project).await;
    assert_eq!(m.files.len(), 1);
}

impl Client {
    async fn list(&self, project: ProjectId, path: &str) -> Result<(u32, Vec<DirEntry>), String> {
        match self
            .call(Request::ListDirectory {
                project,
                path: path.into(),
            })
            .await?
        {
            Response::Directory { revision, entries } => Ok((revision, entries)),
            other => panic!("{other:?}"),
        }
    }

    async fn file_text(&self, project: ProjectId, path: &str) -> Result<TextContent, String> {
        match self
            .call(Request::FileText {
                project,
                path: path.into(),
            })
            .await?
        {
            Response::FileText { content, .. } => Ok(content),
            other => panic!("{other:?}"),
        }
    }
}

fn names(entries: &[DirEntry]) -> Vec<(&str, DirEntryKind, bool)> {
    entries.iter().map(|e| (e.name.as_str(), e.kind, e.changed)).collect()
}

#[tokio::test]
async fn the_whole_snapshot_can_be_browsed_and_read() {
    use DirEntryKind::{Dir, File};
    let fx = fixture(
        &[
            ("src/a/deep/f.rs", "fn f() {}\n"),
            ("src/lib.rs", "// lib\n"),
            ("blob.bin", "x\0y"),
            ("README.md", "hello\n"),
        ],
        &[
            ("src/a/deep/f.rs", "fn f() { 1; }\n"),
            ("src/new.rs", "// new\n"),
            ("target/debug/out", "ignored\n"),
            (".pnpm-store/v11/x", "store\n"),
            ("pkg/.pnpm-store/v11/y", "store\n"),
        ],
    )
    .await;
    let client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;

    let (revision, root) = client.list(project, "").await.unwrap();
    assert_eq!(revision, 0);
    // No `target/`, `.pnpm-store/` or `pkg/` (which holds only a tool store).
    assert_eq!(
        names(&root),
        [
            ("src", Dir, true),
            (".gitignore", File, false),
            ("README.md", File, false),
            ("blob.bin", File, false)
        ]
    );
    let (_, src) = client.list(project, "src").await.unwrap();
    assert_eq!(
        names(&src),
        [("a", Dir, true), ("lib.rs", File, false), ("new.rs", File, true)]
    );
    let (_, deep) = client.list(project, "src/a/deep").await.unwrap();
    assert_eq!(names(&deep), [("f.rs", File, true)]);
    assert!(client.list(project, "target").await.is_err());
    assert!(client.list(project, ".pnpm-store").await.is_err());

    let text = |text: &str, highlights: &[u32]| TextContent::Text {
        text: text.into(),
        highlights: highlights.into(),
    };
    // A comment: no gap, six units, kind 3.
    assert_eq!(
        client.file_text(project, "src/lib.rs").await.unwrap(),
        text("// lib\n", &[0, 6, 3])
    );
    assert_eq!(
        client.file_text(project, "src/new.rs").await.unwrap(),
        text("// new\n", &[0, 6, 3])
    );
    // No grammar for the extension of a plain text.
    assert_eq!(
        client.file_text(project, "README.md").await.unwrap(),
        text("hello\n", &[])
    );
    assert_eq!(
        client.file_text(project, "blob.bin").await.unwrap(),
        TextContent::NoText {
            reason: NoTextReason::Binary
        }
    );
    assert!(client.file_text(project, "target/debug/out").await.is_err());
}

/// The ranges of an encoding of highlights as (start, length, kind).
fn highlighted(encoded: &[u32]) -> Vec<(u32, u32, u32)> {
    let mut end = 0;
    encoded
        .chunks(3)
        .map(|c| {
            let start = end + c[0];
            end = start + c[1];
            (start, c[1], c[2])
        })
        .collect()
}

#[tokio::test]
async fn both_sides_of_a_changed_file_come_with_highlights() {
    let fx = fixture(
        &[("a.rs", "fn f() { 1 }\n"), ("notes.txt", "fn f() { 1 }\n")],
        &[("a.rs", "// new\nfn g() { \"s\" }\n"), ("notes.txt", "fn g() { 2 }\n")],
    )
    .await;
    let client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    let contents = |path: &str| {
        let call = client.call(Request::FileContents {
            view: vet_protocol::View::Changeset,
            project,
            path: path.into(),
        });
        async {
            match call.await.unwrap() {
                Response::FileContents {
                    base_highlights,
                    target_highlights,
                    ..
                } => (highlighted(&base_highlights), highlighted(&target_highlights)),
                other => panic!("{other:?}"),
            }
        }
    };

    let (base, target) = contents("a.rs").await;
    // `fn` is a keyword, `f` a function, `1` a number.
    assert_eq!(base, [(0, 2, 0), (3, 1, 5), (9, 1, 2)]);
    // The comment, then the same for the new line; the string is in the target only.
    assert_eq!(target, [(0, 6, 3), (7, 2, 0), (10, 1, 5), (16, 3, 1)]);
    // The same text again is answered from the cache, equally.
    assert_eq!(contents("a.rs").await, (base, target));

    // A language without a grammar has none, on either side.
    assert_eq!(contents("notes.txt").await, (vec![], vec![]));
}

/// What a review of the first changeset of a fresh store holds from its start.
const FIRST_VERSION_REFS: &str = "refs/vet/changesets/1/1/base\nrefs/vet/changesets/1/1/tip\n";

fn project_refs(repo: &Path) -> String {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["for-each-ref", "--format=%(refname)", "refs/vet"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

#[tokio::test]
async fn reopening_a_repository_resumes_its_review_across_a_restart() {
    let dirty = text(30, &[3, 20]);
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[("a.txt", &dirty)]).await;
    let first = Client::new(&fx.handler);
    let project = open(&first, fx.repo.path()).await;
    first.toggle_hunk(project, 0, 0).await;
    first
        .thread(Request::CreateThread {
            view: vet_protocol::View::Changeset,
            project,
            revision: 0,
            target: ThreadTarget::Range {
                side: Side::Target,
                path: "a.txt".into(),
                start: 20,
                len: 1,
            },
            body: "why?".into(),
        })
        .await;
    first.call(Request::CloseProject { project }).await.unwrap();

    // The app quits, and a file is edited while it is closed: two lines go in at the top.
    let restarted = std::sync::Arc::new(Orchestrator::new(
        open_store(fx._data.path()).await.unwrap(),
        Config {
            debounce: Duration::from_millis(150),
            poll: Duration::from_millis(300),
            orphan_grace: Duration::from_secs(60),
            ..Config::default()
        },
    ));
    write(fx.repo.path(), "a.txt", &format!("new 1\nnew 2\n{dirty}"));
    let second = Client::new(&restarted);
    let project = open(&second, fx.repo.path()).await;

    let m = second.manifest(project).await;
    assert!(
        matches!(m.review, ReviewInfo::Open { version: 2, .. }),
        "{:?}",
        m.review
    );
    assert_eq!(m.revision, 1, "what is on disk now is a new settled snapshot");
    assert_eq!(
        hunk_states(&m),
        [HunkState::Unseen, HunkState::Seen, HunkState::Unseen],
        "the mark followed its lines"
    );
    let (_, threads) = second.threads(project).await;
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].messages[0].body, "why?");
    assert_eq!(threads[0].location.as_ref().map(|l| l.start), Some(22));
}

#[tokio::test]
async fn an_orphaned_project_is_released_but_stays_open_for_the_next_open() {
    let fx = fixture(&[("a.txt", &text(5, &[]))], &[("a.txt", &text(5, &[1]))]).await;
    let first = Client::new(&fx.handler);
    let project = open(&first, fx.repo.path()).await;
    assert_eq!(project_refs(fx.repo.path()), "", "browsing holds nothing");
    first.toggle_hunk(project, 0, 0).await;
    assert_eq!(project_refs(fx.repo.path()), FIRST_VERSION_REFS);

    // The connection goes away without closing; a reload within the grace
    // period finds the project again.
    drop(first);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = Client::new(&fx.handler);
    assert_eq!(open(&second, fx.repo.path()).await, project);

    drop(second);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let third = Client::new(&fx.handler);
    assert!(
        third
            .call(Request::Manifest {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .is_err()
    );
    // Not closed: its refs stay, and opening the repository again resumes it.
    assert_eq!(project_refs(fx.repo.path()), FIRST_VERSION_REFS);
    let resumed = open(&third, fx.repo.path()).await;
    assert!(matches!(
        third.manifest(resumed).await.review,
        ReviewInfo::Open { version: 1, .. }
    ));
}

#[tokio::test]
async fn a_subfolder_opens_the_repository_root_and_both_paths_share_one_project() {
    let fx = fixture(&[("sub/dir/a.txt", "x\n")], &[]).await;
    let client = Client::new(&fx.handler);
    let call = |path: &Path| {
        client.call(Request::OpenProject {
            repo: path.to_str().unwrap().into(),
        })
    };

    let Response::Project { project, root } = call(&fx.repo.path().join("sub/dir")).await.unwrap() else {
        panic!("not a project");
    };
    assert_eq!(
        Path::new(&root).canonicalize().unwrap(),
        fx.repo.path().canonicalize().unwrap()
    );
    assert!(!root.starts_with(r"\\?\"), "shown without the verbatim prefix: {root}");
    let Response::Project {
        project: again,
        root: found,
    } = call(fx.repo.path()).await.unwrap()
    else {
        panic!("not a project");
    };
    assert_eq!((again, found), (project, root));
}

#[tokio::test]
async fn paths_outside_a_repository_and_bare_repositories_are_rejected() {
    let fx = fixture(&[("a.txt", "x\n")], &[]).await;
    let client = Client::new(&fx.handler);
    let open = |path: &Path| {
        client.call(Request::OpenProject {
            repo: path.to_str().unwrap().into(),
        })
    };

    let dir = tempfile::tempdir().unwrap();
    let error = open(dir.path()).await.unwrap_err();
    assert_eq!(error, format!("Not a git repository: {}", dir.path().display()));
    let missing = dir.path().join("missing");
    assert!(open(&missing).await.unwrap_err().starts_with("Not a git repository"));

    let bare = tempfile::tempdir().unwrap();
    git(bare.path(), &["init", "-q", "--bare"]);
    assert_eq!(
        open(bare.path()).await.unwrap_err(),
        "Bare repositories are not supported"
    );
}

impl Client {
    async fn set_base(&self, project: ProjectId, base: BaseChoice) -> Result<Base, String> {
        match self.call(Request::SetBase { project, base }).await? {
            Response::Base(base) => Ok(base),
            other => panic!("{other:?}"),
        }
    }
}

/// The commit the base of `version` of the first changeset of a fresh store is held at.
fn base_ref(repo: &Path, version: u32) -> String {
    git(repo, &["rev-parse", &format!("refs/vet/changesets/1/{version}/base")])
}

fn merge_base(reference: &str) -> BaseChoice {
    BaseChoice::MergeBase {
        reference: reference.into(),
    }
}

#[tokio::test]
async fn the_base_can_be_changed_and_review_state_follows() {
    let main_text = text(40, &[]);
    let fx = fixture(&[("a.txt", &main_text)], &[]).await;
    let p = fx.repo.path();
    let main = git(p, &["rev-parse", "HEAD"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    // The first branch commit drops a line inside what the second one edits.
    let dropped = main_text.replace("line 21\n", "");
    write(p, "a.txt", &dropped);
    git(p, &["commit", "-q", "-am", "drop a line"]);
    let one = git(p, &["rev-parse", "HEAD"]);
    let committed = dropped.replace("line 20\n", "edited 20\n");
    write(p, "a.txt", &committed);
    git(p, &["commit", "-q", "-am", "edit a line"]);
    let two = git(p, &["rev-parse", "HEAD"]);
    write(p, "a.txt", &committed.replace("line 30\n", "edited 30\n"));
    let tree = git(p, &["rev-parse", "HEAD^{tree}"]);
    let unrelated = git(p, &["commit-tree", &tree, "-m", "unrelated"]);
    git(p, &["branch", "unrelated", &unrelated]);

    let mut client = Client::new(&fx.handler);
    let project = open(&client, p).await;

    // The current branch's tip: only the dirty edit.
    let m = client.manifest(project).await;
    assert_eq!(m.base.choice, merge_base("feature"));
    assert_eq!(
        (m.base.commit.id.as_str(), m.base.name.as_str()),
        (two.as_str(), "feature")
    );
    assert_eq!(m.base.commit.subject, "edit a line");
    assert_eq!(
        (m.target.name.as_str(), m.target.branch.as_deref()),
        ("Working tree", Some("feature"))
    );
    assert_eq!(m.files[0].hunks.len(), 1);
    assert_eq!(lines(&m.completion), (2, 0));

    // Where feature branched off main: the branch's change shows as well.
    let base = client.set_base(project, merge_base("main")).await.unwrap();
    assert_eq!((base.commit.id.as_str(), base.name.as_str()), (main.as_str(), "main"));
    assert_eq!(client.settled().await, 1);
    assert!(project_refs(p).is_empty(), "browsing holds no refs");
    let m = client.manifest(project).await;
    assert_eq!((m.revision, &m.base), (1, &base));
    assert_eq!(hunk_states(&m), [HunkState::Unseen, HunkState::Unseen]);
    assert_eq!((m.files[0].base_lines, m.files[0].target_lines), (41, 40));

    // Marking the branch's hunk starts the review, which holds the base chosen so far.
    let update = client.toggle_hunk(project, 1, 0).await;
    assert_eq!(update.review.hunks, [HunkState::Seen, HunkState::Unseen]);
    assert_eq!(base_ref(p, 1), main);

    // Against the first branch commit, base lines of the marked hunk have changed.
    let base = client
        .set_base(project, BaseChoice::Commit { id: one[..10].into() })
        .await
        .unwrap();
    assert_eq!(
        (base.commit.id.as_str(), base.name.as_str()),
        (one.as_str(), "feature~1")
    );
    assert_eq!(base.choice, BaseChoice::Commit { id: one.clone() });
    assert_eq!(client.settled().await, 2);
    assert_eq!((base_ref(p, 2), base_ref(p, 1)), (one.clone(), main.clone()));
    let m = client.manifest(project).await;
    assert_eq!(hunk_states(&m), [HunkState::SeenAgainstOldBase, HunkState::Unseen]);
    assert_eq!(m.completion.old_base_lines, 1);
    let Response::FileContents {
        revision, base: shown, ..
    } = client
        .call(Request::FileContents {
            view: vet_protocol::View::Changeset,
            project,
            path: "a.txt".into(),
        })
        .await
        .unwrap()
    else {
        panic!("not file contents");
    };
    assert_eq!((revision, shown), (2, Some(dropped)));

    // Back to the branch's tip; the mark lies outside what is left to review.
    let base = client.set_base(project, merge_base("feature")).await.unwrap();
    assert_eq!((base.commit.id.as_str(), base.name.as_str()), (two.as_str(), "feature"));
    assert_eq!(client.settled().await, 3);
    assert_eq!(base_ref(p, 3), two);
    assert_eq!(hunk_states(&client.manifest(project).await), [HunkState::Unseen]);

    // The earlier base brings the earlier coverage back.
    client.set_base(project, merge_base("main")).await.unwrap();
    client.settled().await;
    assert_eq!(
        hunk_states(&client.manifest(project).await),
        [HunkState::Seen, HunkState::Unseen]
    );

    // Candidates: branches by name and the commits HEAD reaches.
    let Response::BaseCandidates { branches, commits } =
        client.call(Request::ListBaseCandidates { project }).await.unwrap()
    else {
        panic!("not candidates");
    };
    let branches: Vec<_> = branches.iter().map(|b| (b.name.as_str(), b.current)).collect();
    assert_eq!(branches, [("feature", true), ("main", false), ("unrelated", false)]);
    let ids: Vec<_> = commits.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, [two.as_str(), one.as_str(), main.as_str()]);
    assert_eq!(commits[0].short_id, two[..7]);
    assert_eq!(commits[0].subject, "edit a line");

    // Unresolvable bases are refused with a readable message and change nothing.
    let err = client
        .set_base(project, BaseChoice::Commit { id: "nope".into() })
        .await
        .unwrap_err();
    assert_eq!(err, "unknown commit nope");
    let err = client.set_base(project, merge_base("unrelated")).await.unwrap_err();
    assert_eq!(err, "no common ancestor with unrelated");
    assert_eq!(client.manifest(project).await.revision, 4);
    assert!(client.no_event());
}

#[tokio::test]
async fn versions_are_listed_once_a_review_started_and_an_interdiff_needs_two_of_them() {
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[("a.txt", &text(30, &[3]))]).await;
    let mut one = Client::new(&fx.handler);
    let project = open(&one, fx.repo.path()).await;
    async fn versions(client: &Client, project: ProjectId) -> Vec<vet_protocol::VersionInfo> {
        match client.call(Request::Versions { project }).await.unwrap() {
            Response::Versions { versions } => versions,
            other => panic!("{other:?}"),
        }
    }
    assert!(versions(&one, project).await.is_empty(), "browsing records no versions");

    one.toggle_hunk(project, 0, 0).await;

    let listed = versions(&one, project).await;
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].n, listed[0].reviewed_here), (1, true));

    // Each version is served with what it changed against the one before.
    write(fx.repo.path(), "a.txt", &text(30, &[3, 20]));
    write(fx.repo.path(), "b.txt", "new\n");
    assert_eq!(one.settled().await, 1);
    let files = |v: &vet_protocol::VersionInfo| {
        (v.files.iter())
            .map(|f| (f.path.clone(), f.delta.added, f.delta.removed))
            .collect::<Vec<_>>()
    };
    let listed = versions(&one, project).await;
    assert_eq!(files(&listed[0]), [("a.txt".to_string(), 1, 1)]);
    assert_eq!(
        files(&listed[1]),
        [("a.txt".to_string(), 1, 1), ("b.txt".to_string(), 1, 0)]
    );
    let interdiff = Request::Manifest {
        project,
        view: vet_protocol::View::Interdiff { from: 1, to: 3 },
    };
    assert_eq!(one.call(interdiff).await.unwrap_err(), "no such version 3");
}

#[tokio::test]
async fn a_review_starts_with_the_first_mark_and_finishing_returns_to_browsing() {
    let dirty = text(30, &[3]);
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[("a.txt", &dirty)]).await;
    let mut one = Client::new(&fx.handler);
    let mut two = Client::new(&fx.handler);
    let project = open(&one, fx.repo.path()).await;
    open(&two, fx.repo.path()).await;
    assert_eq!(one.manifest(project).await.review, ReviewInfo::Browsing);

    // Looking around starts nothing.
    assert!(one.started().is_empty());

    let update = one.toggle_hunk(project, 0, 0).await;
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    let ReviewInfo::Open { version: 1, updated_at } = one.manifest(project).await.review else {
        panic!("the review did not start");
    };
    assert!(updated_at > 1_600_000_000);
    // Everyone is told, the requester too; a second mark says nothing more.
    let started = ReviewInfo::Open { version: 1, updated_at };
    assert_eq!(one.started(), [started]);
    assert!(matches!(two.event().await, Event::ReviewChanged { .. }));
    assert_eq!(two.started(), [started]);
    one.toggle_hunk(project, 0, 0).await;
    one.toggle_hunk(project, 0, 0).await;
    assert_eq!(one.started().len(), 1);

    // Each settled snapshot is the next version.
    write(fx.repo.path(), "a.txt", &text(30, &[3, 20]));
    assert_eq!(one.settled().await, 1);
    assert!(matches!(
        one.manifest(project).await.review,
        ReviewInfo::Open { version: 2, .. }
    ));

    // Finishing with an unseen hunk left is allowed, and everyone refetches.
    let Response::ReviewFinished { complete } = one.call(Request::FinishReview { project }).await.unwrap() else {
        panic!("not finished");
    };
    assert!(!complete);
    assert_eq!(one.settled().await, 2);
    // The other connection still has the review changes queued before it.
    let revision = loop {
        match two.event().await {
            Event::SnapshotSettled { revision, .. } if revision == 2 => break revision,
            Event::ReviewChanged { .. } | Event::SnapshotSettled { .. } => {}
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(revision, 2);
    let m = one.manifest(project).await;
    assert_eq!(m.review, ReviewInfo::Browsing);
    assert_eq!(hunk_states(&m), [HunkState::Unseen, HunkState::Unseen]);
    assert_eq!(project_refs(fx.repo.path()), "");
    assert!(one.call(Request::FinishReview { project }).await.is_err());

    // The next review action starts a new review at version 1.
    one.toggle_hunk(project, 2, 0).await;
    assert!(matches!(
        one.manifest(project).await.review,
        ReviewInfo::Open { version: 1, .. }
    ));
}

#[tokio::test]
async fn reopening_without_a_started_review_browses_again() {
    let fx = fixture(&[("a.txt", &text(5, &[]))], &[("a.txt", &text(5, &[1]))]).await;
    let first = Client::new(&fx.handler);
    let project = open(&first, fx.repo.path()).await;
    first.call(Request::CloseProject { project }).await.unwrap();

    let second = Client::new(&fx.handler);
    let project = open(&second, fx.repo.path()).await;

    assert_eq!(second.manifest(project).await.review, ReviewInfo::Browsing);
}

#[tokio::test]
async fn a_branch_base_reports_how_far_the_branch_moved_on_until_chosen_again() {
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[("a.txt", &text(10, &[2]))]).await;
    let p = fx.repo.path();
    let first = git(p, &["rev-parse", "HEAD"]);
    let mut client = Client::new(&fx.handler);
    let project = open(&client, p).await;
    let m = client.manifest(project).await;
    assert_eq!((m.base.name.as_str(), m.base.behind), ("main", None));

    // Two commits land on main; the working tree stays as it was.
    git(p, &["commit", "-q", "-am", "edit"]);
    git(p, &["commit", "-q", "--allow-empty", "-m", "empty"]);
    let tip = git(p, &["rev-parse", "HEAD"]);

    let m = client.manifest(project).await;
    assert_eq!(m.base.commit.id, first);
    assert_eq!((m.base.name.as_str(), m.base.behind), ("main~2", Some(2)));
    assert_eq!(m.files[0].hunks.len(), 1, "the commits are still being reviewed");

    let base = client.set_base(project, merge_base("main")).await.unwrap();
    assert_eq!((base.commit.id, base.name.as_str(), base.behind), (tip, "main", None));
    client.settled().await;
    assert!(client.manifest(project).await.files.is_empty());
}

/// `main` gained `c.txt` after `feature` split off and `feature` added `b.txt`. Leaves `main` checked out.
async fn diverged(fx: &Fixture) -> String {
    let p = fx.repo.path();
    let fork = git(p, &["rev-parse", "HEAD"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "b.txt", "from the branch\n");
    git(p, &["add", "b.txt"]);
    git(p, &["commit", "-q", "-m", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    write(p, "c.txt", "from main\n");
    git(p, &["add", "c.txt"]);
    git(p, &["commit", "-q", "-m", "main moves on"]);
    fork
}

fn paths(m: &vet_protocol::Manifest) -> Vec<&str> {
    m.files.iter().map(|f| f.path.as_str()).collect()
}

impl Client {
    async fn open_branch(&mut self, project: ProjectId, name: &str) {
        let changeset = ChangesetKey::Branch { name: name.into() };
        self.call(Request::OpenChangeset { project, changeset }).await.unwrap();
        let Event::ChangesetOpened { .. } = self.event().await else {
            panic!("not told which changeset is on view")
        };
    }
}

#[tokio::test]
async fn a_branch_against_a_target_that_moved_on_shows_only_its_own_changes() {
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[]).await;
    let fork = diverged(&fx).await;
    let mut client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    client.open_branch(project, "feature").await;

    let base = client.set_base(project, merge_base("main")).await.unwrap();

    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(paths(&m), ["b.txt"], "what main gained is not a reverse change");
    assert_eq!(
        (base.commit.id.as_str(), base.behind, base.contained_in),
        (fork.as_str(), Some(1), None)
    );
}

#[tokio::test]
async fn a_changeset_the_target_already_contains_is_empty_and_says_so() {
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[]).await;
    let p = fx.repo.path();
    git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "b.txt", "from the branch\n");
    git(p, &["add", "b.txt"]);
    git(p, &["commit", "-q", "-m", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    let mut client = Client::new(&fx.handler);
    let project = open(&client, p).await;
    client.open_branch(project, "main").await;

    let base = client.set_base(project, merge_base("feature")).await.unwrap();

    client.settled().await;
    assert!(client.manifest(project).await.files.is_empty());
    assert_eq!(base.contained_in.as_deref(), Some("feature"));
    assert_eq!(base.name, "main");
}

#[tokio::test]
async fn the_working_tree_against_a_target_is_the_branch_since_the_fork_plus_uncommitted_changes() {
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[]).await;
    let p = fx.repo.path();
    let fork = diverged(&fx).await;
    git(p, &["checkout", "-q", "feature"]);
    write(p, "a.txt", &text(10, &[2]));
    let mut client = Client::new(&fx.handler);
    let project = open(&client, p).await;
    assert_eq!(
        paths(&client.manifest(project).await),
        ["a.txt"],
        "by default only uncommitted changes"
    );

    let base = client.set_base(project, merge_base("main")).await.unwrap();

    client.settled().await;
    assert_eq!(paths(&client.manifest(project).await), ["a.txt", "b.txt"]);
    assert_eq!(base.commit.id, fork);

    // An explicit commit is the base as chosen, whatever the target is.
    let main_tip = git(p, &["rev-parse", "main"]);
    client
        .set_base(project, BaseChoice::Commit { id: main_tip })
        .await
        .unwrap();
    client.settled().await;
    assert_eq!(paths(&client.manifest(project).await), ["a.txt", "b.txt", "c.txt"]);
}

#[tokio::test]
async fn a_branch_is_listed_opened_as_the_changeset_on_view_and_left_again_for_every_connection() {
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[("a.txt", &text(10, &[2]))]).await;
    let p = fx.repo.path();
    git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "b.txt", "from the branch\n");
    git(p, &["add", "b.txt"]);
    git(p, &["commit", "-q", "-m", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    let mut first = Client::new(&fx.handler);
    let mut second = Client::new(&fx.handler);
    let project = open(&first, p).await;
    open(&second, p).await;

    let Response::Changesets { rows, on_view } = first.call(Request::ListChangesets { project }).await.unwrap() else {
        panic!("not a list")
    };
    assert_eq!(on_view, ChangesetKey::WorkingTree);
    assert_eq!(rows[0].key, ChangesetKey::WorkingTree);
    let feature = rows
        .iter()
        .find(|r| r.key == ChangesetKey::Branch { name: "feature".into() })
        .expect("a branch with a commit of its own is listed");
    assert_eq!(feature.divergence, Some(Divergence { ahead: 1, behind: 0 }));
    assert_eq!(feature.review, RowReview::None);

    let key = feature.key.clone();
    first
        .call(Request::OpenChangeset {
            project,
            changeset: key.clone(),
        })
        .await
        .unwrap();
    for client in [&mut first, &mut second] {
        let Event::ChangesetOpened {
            revision, changeset, ..
        } = client.event().await
        else {
            panic!("not told which changeset is on view")
        };
        assert_eq!(changeset, key);
        let m = client.manifest(project).await;
        assert_eq!(
            (m.revision, m.target.changeset.clone(), m.target.name.as_str()),
            (revision, key.clone(), "feature")
        );
        assert_eq!(m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["b.txt"]);
    }

    first
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::WorkingTree,
        })
        .await
        .unwrap();
    let Event::ChangesetOpened { .. } = second.event().await else {
        panic!("not told")
    };
    let m = second.manifest(project).await;
    assert_eq!(m.target.changeset, ChangesetKey::WorkingTree);
    assert_eq!(m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
}

/// A repository with a branch `feature` that edits `a.txt`, `main` checked out.
async fn fixture_with_feature() -> Fixture {
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[]).await;
    let p = fx.repo.path();
    git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", &text(30, &[3]));
    git(p, &["commit", "-q", "-a", "-m", "feature"]);
    git(p, &["checkout", "-q", "main"]);
    fx
}

/// Commits `content` as `a.txt` to `branch` from a second checkout of the repository, so the
/// one the app watches sees only the branch's ref move.
fn commit_elsewhere(repo: &Path, checkout: &Path, branch: &str, content: &str) {
    if !checkout.exists() {
        git(
            repo,
            &["worktree", "add", "-q", "--detach", checkout.to_str().unwrap(), "main"],
        );
    }
    git(checkout, &["checkout", "-q", branch]);
    write(checkout, "a.txt", content);
    git(checkout, &["commit", "-q", "-a", "-m", "more"]);
    git(checkout, &["checkout", "-q", "--detach", "main"]);
}

async fn version_count(client: &Client, project: ProjectId) -> usize {
    match client.call(Request::Versions { project }).await.unwrap() {
        Response::Versions { versions } => versions.len(),
        other => panic!("{other:?}"),
    }
}

/// Opens `name` as the changeset on view and starts its review with the first mark.
async fn review_branch(client: &mut Client, project: ProjectId, name: &str) {
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::Branch { name: name.into() },
        })
        .await
        .unwrap();
    let Event::ChangesetOpened { revision, .. } = client.event().await else {
        panic!("not told which changeset is on view")
    };
    client.toggle_hunk(project, revision, 0).await;
    // The marker is told of its mark too; a fetch at opening may say what it found.
    loop {
        match client.event().await {
            Event::ReviewChanged { .. } => break,
            Event::BaseChanged { .. } | Event::ChangesetsChanged { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(version_count(client, project).await, 1);
}

#[tokio::test]
async fn a_row_of_the_list_has_its_lines_asked_for_and_the_progress_of_its_review_and_the_author_of_its_tip() {
    let fx = fixture_with_feature().await;
    let mut client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    let feature = ChangesetKey::Branch { name: "feature".into() };
    async fn lines(client: &Client, project: ProjectId, changeset: ChangesetKey) -> Option<LineDelta> {
        match client
            .call(Request::ChangesetLines { project, changeset })
            .await
            .unwrap()
        {
            Response::ChangesetLines { lines } => lines,
            other => panic!("{other:?}"),
        }
    }
    async fn row(client: &Client, project: ProjectId, key: &ChangesetKey) -> vet_protocol::ChangesetRow {
        let Response::Changesets { rows, .. } = client.call(Request::ListChangesets { project }).await.unwrap() else {
            panic!("not a list")
        };
        rows.into_iter().find(|r| r.key == *key).expect("listed")
    }

    assert_eq!(
        lines(&client, project, feature.clone()).await,
        Some(LineDelta { added: 1, removed: 1 })
    );
    assert_eq!(lines(&client, project, ChangesetKey::WorkingTree).await, None);
    let before = row(&client, project, &feature).await;
    assert_eq!((before.review, before.author.as_deref()), (RowReview::None, Some("t")));

    review_branch(&mut client, project, "feature").await;

    let RowReview::Open { progress, .. } = row(&client, project, &feature).await.review else {
        panic!("the review is open")
    };
    let progress = progress.expect("measured with the first mark");
    assert_eq!((progress.seen_percent, progress.old_base_percent), (100.0, 0.0));
}

#[tokio::test]
async fn a_commit_to_the_branch_on_view_is_its_next_version_without_opening_it_again() {
    let fx = fixture_with_feature().await;
    let p = fx.repo.path();
    let mut client = Client::new(&fx.handler);
    let project = open(&client, p).await;
    review_branch(&mut client, project, "feature").await;

    let other = tempfile::tempdir().unwrap();
    commit_elsewhere(p, &other.path().join("wt"), "feature", &text(30, &[3, 20]));

    let revision = client.settled().await;
    assert_eq!(version_count(&client, project).await, 2);
    let m = client.manifest(project).await;
    assert_eq!(m.revision, revision);
    assert_eq!(m.files[0].hunks.len(), 2, "the new commit is in the changeset");
}

#[tokio::test]
async fn a_fetch_that_moves_the_remote_branch_on_view_is_its_next_version() {
    let fx = fixture_with_feature().await;
    let p = fx.repo.path();
    let scratch = tempfile::tempdir().unwrap();
    let remote = scratch.path().join("remote.git");
    git(
        scratch.path(),
        &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()],
    );
    git(p, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(p, &["push", "-q", "origin", "main", "feature"]);
    let clone = scratch.path().join("clone");
    git(
        scratch.path(),
        &["clone", "-q", remote.to_str().unwrap(), clone.to_str().unwrap()],
    );
    let mut client = Client::new(&fx.handler);
    let project = open(&client, p).await;
    review_branch(&mut client, project, "origin/feature").await;

    git(&clone, &["checkout", "-q", "feature"]);
    write(&clone, "a.txt", &text(30, &[3, 20]));
    git(&clone, &["commit", "-q", "-a", "-m", "more"]);
    git(&clone, &["push", "-q", "origin", "feature"]);
    git(p, &["fetch", "-q", "origin"]);

    // The app fetches by itself on opening, and says so with `BaseChanged` and `ChangesetsChanged`.
    loop {
        match client.event().await {
            Event::BaseChanged { .. } | Event::ChangesetsChanged { .. } => {}
            Event::SnapshotSettled { .. } => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(version_count(&client, project).await, 2);
}

#[tokio::test]
async fn a_commit_to_another_branch_leaves_the_branch_on_view_alone() {
    let fx = fixture_with_feature().await;
    let p = fx.repo.path();
    git(p, &["branch", "other", "main"]);
    let mut client = Client::new(&fx.handler);
    let project = open(&client, p).await;
    review_branch(&mut client, project, "feature").await;
    let revision = client.manifest(project).await.revision;

    let scratch = tempfile::tempdir().unwrap();
    commit_elsewhere(p, &scratch.path().join("wt"), "other", &text(30, &[9]));

    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(client.no_event());
    assert_eq!(version_count(&client, project).await, 1);
    assert_eq!(client.manifest(project).await.revision, revision);
}

#[tokio::test]
async fn every_local_branch_is_listed_however_old_and_whatever_its_name() {
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[]).await;
    let p = fx.repo.path();
    for (i, name) in ["feature/rework-docs", "ostafen-master", "patch/1/deep"]
        .iter()
        .enumerate()
    {
        git(p, &["checkout", "-q", "-b", name, "main"]);
        write(p, &format!("b{i}.txt"), "old work\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "--date", "2020-01-01T00:00:00", "-m", "old work"]);
    }
    git(p, &["checkout", "-q", "main"]);
    git(p, &["branch", "merged"]);
    let client = Client::new(&fx.handler);
    let project = open(&client, p).await;

    let Response::Changesets { rows, .. } = client.call(Request::ListChangesets { project }).await.unwrap() else {
        panic!("not a list")
    };

    let mut names: Vec<_> = rows
        .iter()
        .filter_map(|r| match &r.key {
            ChangesetKey::Branch { name } => Some(name.as_str()),
            ChangesetKey::WorkingTree | ChangesetKey::PullRequest { .. } => None,
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "feature/rework-docs",
            "main",
            "merged",
            "ostafen-master",
            "patch/1/deep"
        ]
    );
}

impl Client {
    async fn set_filter(&self, project: ProjectId, filter: &ReviewFilter) -> Result<Response, String> {
        let filter = filter.clone();
        self.call(Request::SetProjectFilter { project, filter }).await
    }
}

#[tokio::test]
async fn the_filter_moves_completion_and_the_excluded_counts() {
    let fx = fixture(
        &[("a.txt", "1\n"), ("docs/b.md", "1\n")],
        &[("a.txt", "2\n"), ("docs/b.md", "2\n")],
    )
    .await;
    let mut client = Client::new(&fx.handler);
    let project = open(&client, fx.repo.path()).await;
    let m = client.manifest(project).await;
    assert_eq!((m.files.len(), m.completion.lines, m.excluded_lines), (2, 4, 0));
    assert_eq!(m.filter, ReviewFilter::default());
    // Each replaced line is one added and one removed; the changeset sums the shown files.
    let delta = |added, removed| LineDelta { added, removed };
    assert_eq!(
        m.files.iter().map(|f| f.delta).collect::<Vec<_>>(),
        [delta(1, 1), delta(1, 1)]
    );
    assert_eq!(m.delta, delta(2, 2));

    let filter = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };
    assert!(matches!(
        client.set_filter(project, &filter).await.unwrap(),
        Response::Done
    ));
    assert_eq!(client.settled().await, 1);
    let m = client.manifest(project).await;
    assert_eq!((m.revision, &m.filter), (1, &filter));
    assert_eq!((m.files.len(), m.completion.lines, m.excluded_lines), (1, 2, 2));
    assert_eq!(m.delta, delta(1, 1), "hidden files are not counted");
    // Marking the one remaining file completes the review.
    let update = client.toggle_hunk(project, 1, 0).await;
    assert_eq!(lines(&update.changeset), (2, 2));
    assert!(matches!(client.event().await, Event::ReviewChanged { .. }));

    let bad = ReviewFilter {
        include: vec!["src/[".into()],
        ..Default::default()
    };
    let err = client.set_filter(project, &bad).await.unwrap_err();
    assert!(err.starts_with("invalid pattern \"src/[\""), "{err}");
    assert_eq!(client.manifest(project).await.revision, 1);
    assert!(client.no_event());
}

#[tokio::test]
async fn the_project_filter_reaches_every_connection_and_outlives_the_project() {
    let fx = fixture(
        &[("a.txt", "1\n"), ("docs/b.md", "1\n")],
        &[("a.txt", "2\n"), ("docs/b.md", "2\n")],
    )
    .await;
    let mut first = Client::new(&fx.handler);
    let mut second = Client::new(&fx.handler);
    let project = open(&first, fx.repo.path()).await;
    open(&second, fx.repo.path()).await;
    let filter = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };

    first.set_filter(project, &filter).await.unwrap();

    assert_eq!(first.settled().await, 1);
    assert_eq!(second.settled().await, 1);
    assert_eq!(second.manifest(project).await.filter, filter);
    first.call(Request::CloseProject { project }).await.unwrap();
    second.call(Request::CloseProject { project }).await.unwrap();

    let restarted = std::sync::Arc::new(Orchestrator::new(
        open_store(fx._data.path()).await.unwrap(),
        Config {
            debounce: Duration::from_millis(150),
            poll: Duration::from_millis(300),
            orphan_grace: Duration::from_secs(60),
            ..Config::default()
        },
    ));
    let third = Client::new(&restarted);
    let project = open(&third, fx.repo.path()).await;
    let m = third.manifest(project).await;
    assert_eq!(
        (&m.filter, m.files.len()),
        (&filter, 1),
        "browsing, with the project's filter"
    );
}
