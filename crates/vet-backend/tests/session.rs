//! The handler against a real temporary repository: open, manifest, mark,
//! edit, settle, remap, revert (SPEC §12 criterion 1's split and revert cases,
//! and criterion 4's settle path). `git` is used only to build the fixture.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use vet_backend::{Config, OUTPUT_CAP, Orchestrator, open_store};
use vet_protocol::{
    AnchorState, ApprovalChange, Base, BaseChoice, BlobContent, Completion, DirEntry, DirEntryKind, EnvVar, Event,
    FileStatus, HunkState, LineLocation, Manifest, NoTextReason, NonTextKind, Request, Response, ReviewFilter,
    ReviewInfo, ReviewUpdate, RunChange, RunStream, RunTarget, ServerMessage, SessionId, Side, TextContent, Thread,
    ThreadStatus, ThreadTarget,
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

    async fn manifest(&self, session: SessionId) -> Manifest {
        match self.call(Request::Manifest { session }).await.unwrap() {
            Response::Manifest(m) => *m,
            other => panic!("{other:?}"),
        }
    }

    async fn set_seen(&self, session: SessionId, revision: u32, hunks: &[u32], seen: bool) -> ReviewUpdate {
        let request = Request::SetSeen {
            session,
            revision,
            path: "a.txt".into(),
            hunks: hunks.to_vec(),
            seen,
        };
        match self.call(request).await.unwrap() {
            Response::Review(update) => update,
            other => panic!("{other:?}"),
        }
    }

    /// The next event other than `ReviewStarted`; panics if none arrives.
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

    async fn settled(&mut self) -> u32 {
        match self.event().await {
            Event::SnapshotSettled { revision, .. } => revision,
            other => panic!("{other:?}"),
        }
    }

    /// Whether no event other than `ReviewStarted` is waiting.
    fn no_event(&mut self) -> bool {
        while let Ok(message) = self.inbox.try_recv() {
            match message {
                ServerMessage::Event {
                    event: Event::ReviewStarted { review, .. },
                } => self.started.push(review),
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
    };
    Fixture {
        repo,
        _data: data,
        handler: std::sync::Arc::new(Orchestrator::new(store, config)),
    }
}

async fn open(client: &Client, repo: &Path) -> SessionId {
    let request = Request::OpenSession {
        repo: repo.to_str().unwrap().into(),
    };
    match client.call(request).await.unwrap() {
        Response::Session { session, .. } => session,
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

    async fn threads(&self, session: SessionId) -> (u32, Vec<Thread>) {
        match self.call(Request::Threads { session }).await.unwrap() {
            Response::Threads { revision, threads } => (revision, threads),
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn threads_follow_edits_and_notify_other_connections() {
    let dirty = text(30, &[3]);
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[("a.txt", &dirty)]).await;
    let mut one = Client::new(&fx.handler);
    let mut two = Client::new(&fx.handler);
    let session = open(&one, fx.repo.path()).await;
    assert_eq!(open(&two, fx.repo.path()).await, session);

    let created = one
        .thread(Request::CreateThread {
            session,
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
    assert!(one.no_event(), "the requester is not told about its own change");

    // An edit above shifts the lines; the next fetch resolves them at the new revision.
    write(fx.repo.path(), "a.txt", &format!("new 1\nnew 2\n{dirty}"));
    assert_eq!(one.settled().await, 1);
    assert_eq!(two.settled().await, 1);
    let (revision, threads) = one.threads(session).await;
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
    let (_, threads) = one.threads(session).await;
    assert!(
        matches!(threads[0].anchor, Some(AnchorState::Inferred { .. })),
        "{:?}",
        threads[0].anchor
    );
    let id = threads[0].id;
    let confirmed = one
        .thread(Request::ConfirmAnchor {
            session,
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
            session,
            thread: id,
            body: "because".into(),
        })
        .await;
    assert_eq!(replied.messages.len(), 2);
    let resolved = one
        .thread(Request::SetThreadStatus {
            session,
            thread: id,
            status: ThreadStatus::Resolved,
        })
        .await;
    assert_eq!(resolved.status, ThreadStatus::Resolved);
    one.call(Request::DeleteThread { session, thread: id }).await.unwrap();
    for _ in 0..3 {
        assert!(matches!(two.event().await, Event::ThreadsChanged { .. }));
    }
    assert!(one.threads(session).await.1.is_empty());
}

#[tokio::test]
async fn mark_edit_settle_remap_and_revert() {
    let base = text(40, &[]);
    let dirty = text(40, &[5]);
    let fx = fixture(&[("a.txt", &base)], &[("a.txt", &dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let session = open(&client, fx.repo.path()).await;

    let m = client.manifest(session).await;
    assert_eq!((m.revision, m.files.len()), (0, 1));
    assert_eq!(m.files[0].status, FileStatus::Modified);
    assert_eq!(hunk_states(&m), [HunkState::Unseen]);
    assert_eq!(lines(&m.completion), (2, 0));
    assert!(m.degraded.is_none());

    // Mark the hunk seen.
    let update = client.set_seen(session, 0, &[0], true).await;
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    assert_eq!(lines(&update.changeset), (2, 2));

    // The agent edits elsewhere: the mark survives, the new hunk is unseen.
    write(fx.repo.path(), "a.txt", &text(40, &[5, 30]));
    assert_eq!(client.settled().await, 1);
    let m = client.manifest(session).await;
    assert_eq!(m.revision, 1);
    assert_eq!(hunk_states(&m), [HunkState::Seen, HunkState::Unseen]);
    assert_eq!(lines(&m.completion), (4, 2));

    // A request made against revision 0 is rejected, and changes nothing.
    let stale = Request::SetSeen {
        session,
        revision: 0,
        path: "a.txt".into(),
        hunks: vec![1],
        seen: true,
    };
    assert!(client.call(stale).await.unwrap_err().contains("stale"));

    // Editing the marked line uncovers it; restoring it, byte for byte, covers it again.
    write(
        fx.repo.path(),
        "a.txt",
        &text(40, &[30]).replace("line 5\n", "something else\n"),
    );
    client.settled().await;
    let m = client.manifest(session).await;
    assert_eq!(hunk_states(&m), [HunkState::Unseen, HunkState::Unseen]);
    write(fx.repo.path(), "a.txt", &text(40, &[5, 30]));
    client.settled().await;
    let m = client.manifest(session).await;
    assert_eq!(hunk_states(&m), [HunkState::Seen, HunkState::Unseen]);

    // Reverting the second edit leaves the original state, and the file text is current.
    write(fx.repo.path(), "a.txt", &dirty);
    let revision = client.settled().await;
    let m = client.manifest(session).await;
    assert_eq!(hunk_states(&m), [HunkState::Seen]);
    assert_eq!(lines(&m.completion), (2, 2));
    let Response::FileContents {
        revision: at,
        base: b,
        target: t,
        ..
    } = client
        .call(Request::FileContents {
            session,
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
    let update = client.set_seen(session, revision, &[0], false).await;
    assert_eq!(update.review.hunks, [HunkState::Unseen]);
    assert_eq!(lines(&update.changeset), (2, 0));

    client.call(Request::CloseSession { session }).await.unwrap();
    assert!(client.call(Request::Manifest { session }).await.is_err());
}

#[tokio::test]
async fn a_seen_region_with_an_edit_in_the_middle_is_split() {
    let region: Vec<usize> = (5..25).collect();
    let dirty = text(40, &region);
    let fx = fixture(&[("a.txt", &text(40, &[]))], &[("a.txt", &dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let session = open(&client, fx.repo.path()).await;
    client.set_seen(session, 0, &[0], true).await;

    let middle = dirty
        .replace("edited 14", "mid a")
        .replace("edited 15", "mid b")
        .replace("edited 16", "mid c");
    write(fx.repo.path(), "a.txt", &middle);
    client.settled().await;

    let m = client.manifest(session).await;
    assert_eq!(hunk_states(&m), [HunkState::Unseen]);
    // All 20 base lines are still seen, and 17 of the 20 target lines.
    assert_eq!(lines(&m.completion), (40, 37));
}

#[tokio::test]
async fn non_text_changes_are_acknowledged_and_the_other_connection_is_told() {
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
    let session = open(&first, fx.repo.path()).await;
    assert_eq!(open(&second, fx.repo.path()).await, session, "same repo, same session");

    let m = first.manifest(session).await;
    let run = m.files.iter().find(|f| f.path == "run.sh").unwrap();
    assert_eq!(run.non_text.len(), 1);
    assert_eq!(run.non_text[0].kind, NonTextKind::Mode);
    assert_eq!(run.non_text[0].after.as_ref().unwrap().mode, 0o100755);
    assert_eq!((m.completion.items, m.completion.acknowledged_items), (1, 0));

    let request = Request::Acknowledge {
        session,
        revision: 0,
        path: "run.sh".into(),
        kind: NonTextKind::Mode,
        acknowledged: true,
    };
    let Response::Review(update) = first.call(request).await.unwrap() else {
        panic!("not a review update");
    };
    assert_eq!(update.review.acknowledged, [NonTextKind::Mode]);

    match second.event().await {
        Event::ReviewChanged {
            revision, update: u, ..
        } => assert_eq!((revision, u), (0, update)),
        other => panic!("{other:?}"),
    }
    assert!(first.no_event(), "the requester gets the response, not an event");

    // Marking a hunk is reported the same way.
    first.set_seen(session, 0, &[0], true).await;
    assert!(matches!(second.event().await, Event::ReviewChanged { .. }));

    // The first connection leaves; the session lives on for the second.
    first.call(Request::CloseSession { session }).await.unwrap();
    assert_eq!(second.manifest(session).await.completion.acknowledged_items, 1);
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
    let session = open(&client, fx.repo.path()).await;

    let m = client.manifest(session).await;
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

    let request = Request::Acknowledge {
        session,
        revision: 0,
        path: "new.txt".into(),
        kind: NonTextKind::Empty,
        acknowledged: true,
    };
    let Response::Review(update) = client.call(request).await.unwrap() else {
        panic!("not a review update");
    };
    assert_eq!(update.review.acknowledged, [NonTextKind::Empty]);
    assert_eq!(client.manifest(session).await.completion.acknowledged_items, 1);
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
    let session = open(&client, fx.repo.path()).await;
    let blob = |revision, path: &str, side| {
        let request = Request::ItemBlob {
            session,
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
    let session = open(&client, fx.repo.path()).await;

    write(fx.repo.path(), "target/debug/out", "build output\n");
    write(fx.repo.path(), "target/debug/more", "more\n");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(client.no_event(), "ignored writes must not produce snapshots");

    write(fx.repo.path(), "a.txt", &text(5, &[1]));
    assert_eq!(client.settled().await, 1);
    let m = client.manifest(session).await;
    assert_eq!(m.files.len(), 1);
}

impl Client {
    async fn list(&self, session: SessionId, path: &str) -> Result<(u32, Vec<DirEntry>), String> {
        match self
            .call(Request::ListDirectory {
                session,
                path: path.into(),
            })
            .await?
        {
            Response::Directory { revision, entries } => Ok((revision, entries)),
            other => panic!("{other:?}"),
        }
    }

    async fn file_text(&self, session: SessionId, path: &str) -> Result<TextContent, String> {
        match self
            .call(Request::FileText {
                session,
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
    let session = open(&client, fx.repo.path()).await;

    let (revision, root) = client.list(session, "").await.unwrap();
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
    let (_, src) = client.list(session, "src").await.unwrap();
    assert_eq!(
        names(&src),
        [("a", Dir, true), ("lib.rs", File, false), ("new.rs", File, true)]
    );
    let (_, deep) = client.list(session, "src/a/deep").await.unwrap();
    assert_eq!(names(&deep), [("f.rs", File, true)]);
    assert!(client.list(session, "target").await.is_err());
    assert!(client.list(session, ".pnpm-store").await.is_err());

    let text = |text: &str| TextContent::Text { text: text.into() };
    assert_eq!(client.file_text(session, "src/lib.rs").await.unwrap(), text("// lib\n"));
    assert_eq!(client.file_text(session, "src/new.rs").await.unwrap(), text("// new\n"));
    assert_eq!(
        client.file_text(session, "blob.bin").await.unwrap(),
        TextContent::NoText {
            reason: NoTextReason::Binary
        }
    );
    assert!(client.file_text(session, "target/debug/out").await.is_err());
}

fn session_refs(repo: &Path) -> String {
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
    let session = open(&first, fx.repo.path()).await;
    first.set_seen(session, 0, &[0], true).await;
    first
        .thread(Request::CreateThread {
            session,
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
    first.call(Request::CloseSession { session }).await.unwrap();

    // The app quits, and a file is edited while it is closed: two lines go in at the top.
    let restarted = std::sync::Arc::new(Orchestrator::new(
        open_store(fx._data.path()).await.unwrap(),
        Config {
            debounce: Duration::from_millis(150),
            poll: Duration::from_millis(300),
            orphan_grace: Duration::from_secs(60),
        },
    ));
    write(fx.repo.path(), "a.txt", &format!("new 1\nnew 2\n{dirty}"));
    let second = Client::new(&restarted);
    let session = open(&second, fx.repo.path()).await;

    let m = second.manifest(session).await;
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
    let (_, threads) = second.threads(session).await;
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].messages[0].body, "why?");
    assert_eq!(threads[0].location.as_ref().map(|l| l.start), Some(22));
}

#[tokio::test]
async fn an_orphaned_session_is_released_but_stays_open_for_the_next_open() {
    let fx = fixture(&[("a.txt", &text(5, &[]))], &[("a.txt", &text(5, &[1]))]).await;
    let first = Client::new(&fx.handler);
    let session = open(&first, fx.repo.path()).await;
    assert_eq!(session_refs(fx.repo.path()), "", "browsing holds nothing");
    first.set_seen(session, 0, &[0], true).await;
    assert_eq!(
        session_refs(fx.repo.path()),
        format!("refs/vet/sessions/{session}/base\nrefs/vet/sessions/{session}/target\n")
    );

    // The connection goes away without closing; a reload within the grace
    // period finds the session again.
    drop(first);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = Client::new(&fx.handler);
    assert_eq!(open(&second, fx.repo.path()).await, session);

    drop(second);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let third = Client::new(&fx.handler);
    assert!(third.call(Request::Manifest { session }).await.is_err());
    // Not closed: its refs stay, and opening the repository again resumes it.
    assert_eq!(
        session_refs(fx.repo.path()),
        format!("refs/vet/sessions/{session}/base\nrefs/vet/sessions/{session}/target\n")
    );
    let resumed = open(&third, fx.repo.path()).await;
    assert!(matches!(
        third.manifest(resumed).await.review,
        ReviewInfo::Open { version: 1, .. }
    ));
}

#[tokio::test]
async fn a_subfolder_opens_the_repository_root_and_both_paths_share_one_session() {
    let fx = fixture(&[("sub/dir/a.txt", "x\n")], &[]).await;
    let client = Client::new(&fx.handler);
    let root = fx.repo.path().canonicalize().unwrap();
    let call = |path: &Path| {
        client.call(Request::OpenSession {
            repo: path.to_str().unwrap().into(),
        })
    };

    let Response::Session { session, root: found } = call(&fx.repo.path().join("sub/dir")).await.unwrap() else {
        panic!("not a session");
    };
    assert_eq!(found, root.to_str().unwrap());
    let Response::Session {
        session: again,
        root: found,
    } = call(fx.repo.path()).await.unwrap()
    else {
        panic!("not a session");
    };
    assert_eq!((again, found.as_str()), (session, root.to_str().unwrap()));
}

#[tokio::test]
async fn paths_outside_a_repository_and_bare_repositories_are_rejected() {
    let fx = fixture(&[("a.txt", "x\n")], &[]).await;
    let client = Client::new(&fx.handler);
    let open = |path: &Path| {
        client.call(Request::OpenSession {
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
    async fn set_base(&self, session: SessionId, base: BaseChoice) -> Result<Base, String> {
        match self.call(Request::SetBase { session, base }).await? {
            Response::Base(base) => Ok(base),
            other => panic!("{other:?}"),
        }
    }
}

fn session_base_ref(repo: &Path, session: SessionId) -> String {
    git(repo, &["rev-parse", &format!("refs/vet/sessions/{session}/base")])
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
    let session = open(&client, p).await;

    // The current branch's tip: only the dirty edit.
    let m = client.manifest(session).await;
    assert_eq!(m.base.choice, BaseChoice::Branch { name: "feature".into() });
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
    let base = client.set_base(session, merge_base("main")).await.unwrap();
    assert_eq!((base.commit.id.as_str(), base.name.as_str()), (main.as_str(), "main"));
    assert_eq!(client.settled().await, 1);
    assert!(session_refs(p).is_empty(), "browsing holds no refs");
    let m = client.manifest(session).await;
    assert_eq!((m.revision, &m.base), (1, &base));
    assert_eq!(hunk_states(&m), [HunkState::Unseen, HunkState::Unseen]);
    assert_eq!((m.files[0].base_lines, m.files[0].target_lines), (40, 39));

    // Marking the branch's hunk starts the review, which holds the base chosen so far.
    let update = client.set_seen(session, 1, &[0], true).await;
    assert_eq!(update.review.hunks, [HunkState::Seen, HunkState::Unseen]);
    assert_eq!(session_base_ref(p, session), main);

    // Against the first branch commit, base lines of the marked hunk have changed.
    let base = client
        .set_base(session, BaseChoice::Commit { id: one[..10].into() })
        .await
        .unwrap();
    assert_eq!(
        (base.commit.id.as_str(), base.name.as_str()),
        (one.as_str(), "feature~1")
    );
    assert_eq!(base.choice, BaseChoice::Commit { id: one.clone() });
    assert_eq!(client.settled().await, 2);
    assert_eq!(session_base_ref(p, session), one);
    let m = client.manifest(session).await;
    assert_eq!(hunk_states(&m), [HunkState::SeenAgainstOldBase, HunkState::Unseen]);
    assert_eq!(m.completion.old_base_lines, 1);
    let Response::FileContents {
        revision, base: shown, ..
    } = client
        .call(Request::FileContents {
            session,
            path: "a.txt".into(),
        })
        .await
        .unwrap()
    else {
        panic!("not file contents");
    };
    assert_eq!((revision, shown), (2, Some(dropped)));

    // Back to the branch's tip; the mark lies outside what is left to review.
    let feature = BaseChoice::Branch { name: "feature".into() };
    let base = client.set_base(session, feature).await.unwrap();
    assert_eq!((base.commit.id.as_str(), base.name.as_str()), (two.as_str(), "feature"));
    assert_eq!(client.settled().await, 3);
    assert_eq!(session_base_ref(p, session), two);
    assert_eq!(hunk_states(&client.manifest(session).await), [HunkState::Unseen]);

    // The earlier base brings the earlier coverage back.
    client.set_base(session, merge_base("main")).await.unwrap();
    client.settled().await;
    assert_eq!(
        hunk_states(&client.manifest(session).await),
        [HunkState::Seen, HunkState::Unseen]
    );

    // Candidates: branches by name and the commits HEAD reaches.
    let Response::BaseCandidates { branches, commits } =
        client.call(Request::ListBaseCandidates { session }).await.unwrap()
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
        .set_base(session, BaseChoice::Commit { id: "nope".into() })
        .await
        .unwrap_err();
    assert_eq!(err, "unknown commit nope");
    let err = client.set_base(session, merge_base("unrelated")).await.unwrap_err();
    assert_eq!(err, "no common ancestor with unrelated");
    assert_eq!(client.manifest(session).await.revision, 4);
    assert!(client.no_event());
}

#[tokio::test]
async fn a_review_starts_with_the_first_mark_and_finishing_returns_to_browsing() {
    let dirty = text(30, &[3]);
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[("a.txt", &dirty)]).await;
    let mut one = Client::new(&fx.handler);
    let mut two = Client::new(&fx.handler);
    let session = open(&one, fx.repo.path()).await;
    open(&two, fx.repo.path()).await;
    assert_eq!(one.manifest(session).await.review, ReviewInfo::Browsing);

    // Looking around and un-marking start nothing.
    one.set_seen(session, 0, &[0], false).await;
    assert!(one.started().is_empty());

    let update = one.set_seen(session, 0, &[0], true).await;
    assert_eq!(update.review.hunks, [HunkState::Seen]);
    let ReviewInfo::Open { version: 1, updated_at } = one.manifest(session).await.review else {
        panic!("the review did not start");
    };
    assert!(updated_at > 1_600_000_000);
    // Everyone is told, the requester too; a second mark says nothing more.
    let started = ReviewInfo::Open { version: 1, updated_at };
    assert_eq!(one.started(), [started]);
    assert!(matches!(two.event().await, Event::ReviewChanged { .. }));
    assert_eq!(two.started(), [started]);
    one.set_seen(session, 0, &[0], false).await;
    one.set_seen(session, 0, &[0], true).await;
    assert_eq!(one.started().len(), 1);

    // Each settled snapshot is the next version.
    write(fx.repo.path(), "a.txt", &text(30, &[3, 20]));
    assert_eq!(one.settled().await, 1);
    assert!(matches!(
        one.manifest(session).await.review,
        ReviewInfo::Open { version: 2, .. }
    ));

    // Finishing with an unseen hunk left is allowed, and everyone refetches.
    let Response::ReviewFinished { complete } = one.call(Request::FinishReview { session }).await.unwrap() else {
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
    let m = one.manifest(session).await;
    assert_eq!(m.review, ReviewInfo::Browsing);
    assert_eq!(hunk_states(&m), [HunkState::Unseen, HunkState::Unseen]);
    assert_eq!(session_refs(fx.repo.path()), "");
    assert!(one.call(Request::FinishReview { session }).await.is_err());

    // The next review action starts a new review at version 1.
    one.set_seen(session, 2, &[0], true).await;
    assert!(matches!(
        one.manifest(session).await.review,
        ReviewInfo::Open { version: 1, .. }
    ));
}

#[tokio::test]
async fn reopening_without_a_started_review_browses_again() {
    let fx = fixture(&[("a.txt", &text(5, &[]))], &[("a.txt", &text(5, &[1]))]).await;
    let first = Client::new(&fx.handler);
    let session = open(&first, fx.repo.path()).await;
    first.call(Request::CloseSession { session }).await.unwrap();

    let second = Client::new(&fx.handler);
    let session = open(&second, fx.repo.path()).await;

    assert_eq!(second.manifest(session).await.review, ReviewInfo::Browsing);
}

#[tokio::test]
async fn a_branch_base_reports_how_far_the_branch_moved_on_until_chosen_again() {
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[("a.txt", &text(10, &[2]))]).await;
    let p = fx.repo.path();
    let first = git(p, &["rev-parse", "HEAD"]);
    let mut client = Client::new(&fx.handler);
    let session = open(&client, p).await;
    let m = client.manifest(session).await;
    assert_eq!((m.base.name.as_str(), m.base.behind), ("main", None));

    // Two commits land on main; the working tree stays as it was.
    git(p, &["commit", "-q", "-am", "edit"]);
    git(p, &["commit", "-q", "--allow-empty", "-m", "empty"]);
    let tip = git(p, &["rev-parse", "HEAD"]);

    let m = client.manifest(session).await;
    assert_eq!(m.base.commit.id, first);
    assert_eq!((m.base.name.as_str(), m.base.behind), ("main~2", Some(2)));
    assert_eq!(m.files[0].hunks.len(), 1, "the commits are still being reviewed");

    let base = client
        .set_base(session, BaseChoice::Branch { name: "main".into() })
        .await
        .unwrap();
    assert_eq!((base.commit.id, base.name.as_str(), base.behind), (tip, "main", None));
    client.settled().await;
    assert!(client.manifest(session).await.files.is_empty());
}

impl Client {
    async fn set_filter(&self, session: SessionId, filter: &ReviewFilter) -> Result<Response, String> {
        let filter = filter.clone();
        self.call(Request::SetFilter { session, filter }).await
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
    let session = open(&client, fx.repo.path()).await;
    let m = client.manifest(session).await;
    assert_eq!((m.files.len(), m.completion.lines, m.excluded_lines), (2, 4, 0));
    assert_eq!(m.filter, ReviewFilter::default());

    let filter = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };
    assert!(matches!(
        client.set_filter(session, &filter).await.unwrap(),
        Response::Done
    ));
    assert_eq!(client.settled().await, 1);
    let m = client.manifest(session).await;
    assert_eq!((m.revision, &m.filter), (1, &filter));
    assert_eq!((m.files.len(), m.completion.lines, m.excluded_lines), (1, 2, 2));
    // Marking the one remaining file completes the review.
    let update = client.set_seen(session, 1, &[0], true).await;
    assert_eq!(lines(&update.changeset), (2, 2));

    let bad = ReviewFilter {
        include: vec!["src/[".into()],
        ..Default::default()
    };
    let err = client.set_filter(session, &bad).await.unwrap_err();
    assert!(err.starts_with("invalid pattern \"src/[\""), "{err}");
    assert_eq!(client.manifest(session).await.revision, 1);
    assert!(client.no_event());
}

const PACKAGE: &str = r#"{"scripts":{"hi":"echo hi"}}"#;

fn sh_target(id: &str, script: &str) -> RunTarget {
    RunTarget {
        id: id.into(),
        source: None,
        label: id.into(),
        program: "sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: String::new(),
        env: vec![EnvVar {
            name: "GREETING".into(),
            value: "hi".into(),
        }],
    }
}

impl Client {
    async fn run_targets(&self, session: SessionId) -> Vec<(String, bool)> {
        match self.call(Request::ListRunTargets { session }).await.unwrap() {
            Response::RunTargets { targets } => targets.into_iter().map(|t| (t.target.id, t.stored)).collect(),
            other => panic!("{other:?}"),
        }
    }

    async fn save_target(&self, session: SessionId, target: RunTarget) -> Result<Response, String> {
        self.call(Request::SaveRunTarget { session, target }).await
    }

    async fn run(&self, session: SessionId, target: &str, approved: Option<&str>) -> Response {
        let request = Request::Run {
            session,
            target_id: target.into(),
            approved_hash: approved.map(String::from),
        };
        self.call(request).await.unwrap()
    }

    /// Output until the exit, whatever else is pushed meanwhile.
    async fn output(&mut self, run: u32) -> (String, String, Option<i32>) {
        let (out, err, code, dropped) = self.output_bounded(run).await;
        assert_eq!(dropped, 0);
        (String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap(), code)
    }

    /// Like `output`, with raw bytes and the dropped count.
    async fn output_bounded(&mut self, run: u32) -> (Vec<u8>, Vec<u8>, Option<i32>, u32) {
        use base64::Engine;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        loop {
            match self.event().await {
                Event::RunOutput {
                    run_id, stream, bytes, ..
                } => {
                    assert_eq!(run_id, run);
                    let bytes = base64::engine::general_purpose::STANDARD.decode(bytes).unwrap();
                    match stream {
                        RunStream::Stdout => out.extend(bytes),
                        RunStream::Stderr => err.extend(bytes),
                    }
                }
                Event::RunExited {
                    run_id,
                    code,
                    dropped_bytes,
                    ..
                } => {
                    assert_eq!(run_id, run);
                    return (out, err, code, dropped_bytes);
                }
                Event::SnapshotSettled { .. } => {}
                other => panic!("{other:?}"),
            }
        }
    }
}

fn needs_approval(response: Response) -> (String, bool, Vec<ApprovalChange>) {
    match response {
        Response::RunNeedsApproval {
            hash,
            first_run,
            changed,
        } => (hash, first_run, changed),
        other => panic!("{other:?}"),
    }
}

fn started(response: Response) -> (u32, u32) {
    match response {
        Response::RunStarted {
            run_id,
            changed_since_review,
            ..
        } => (run_id, changed_since_review),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_run_needs_approval_once_then_streams_output_and_the_exit_code() {
    let fx = fixture(&[("package.json", PACKAGE)], &[("a.txt", "dirty\n")]).await;
    let mut client = Client::new(&fx.handler);
    let session = open(&client, fx.repo.path()).await;

    // The script is discovered; saving a target of the same id edits it.
    assert_eq!(client.run_targets(session).await, [("script::hi".to_string(), false)]);
    let no_command = RunTarget {
        program: " ".into(),
        ..sh_target("t", "true")
    };
    for bad in [sh_target("", "true"), no_command] {
        assert!(client.save_target(session, bad).await.is_err());
    }
    let mut edited = sh_target("script::hi", "echo $GREETING; echo oops >&2; exit 3");
    edited.label = "hi".into();
    client.save_target(session, edited).await.unwrap();
    assert_eq!(client.run_targets(session).await, [("script::hi".to_string(), true)]);

    // The first run asks, with the build file that has never been approved.
    let (hash, first_run, changed) = needs_approval(client.run(session, "script::hi", None).await);
    assert!(first_run);
    assert_eq!(
        changed,
        [ApprovalChange {
            path: "package.json".into(),
            change: RunChange::Added
        }]
    );
    let Response::RunApprovalDiff { files } = client
        .call(Request::RunApprovalDiff {
            session,
            hash: hash.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("not a diff");
    };
    assert_eq!(files.len(), 1);
    assert_eq!(
        (files[0].base.as_deref(), files[0].target.as_deref()),
        (None, Some(PACKAGE))
    );
    let unknown = Request::RunApprovalDiff {
        session,
        hash: "0".repeat(64),
    };
    assert!(client.call(unknown).await.is_err());

    // A wrong hash does not approve; the right one runs.
    assert!(matches!(
        client.run(session, "script::hi", Some(&"0".repeat(64))).await,
        Response::RunNeedsApproval { .. }
    ));
    let (run, changed_since_review) = started(client.run(session, "script::hi", Some(&hash)).await);
    assert_eq!(changed_since_review, 0);
    assert_eq!(client.output(run).await, ("hi\n".into(), "oops\n".into(), Some(3)));

    // Approved: it runs without asking, until the build file changes.
    let (second, _) = started(client.run(session, "script::hi", None).await);
    assert_ne!(second, run);
    client.output(second).await;
    let changed = r#"{"scripts":{"hi":"echo changed"}}"#;
    write(fx.repo.path(), "package.json", changed);
    let (_, first_run, changed_files) = needs_approval(client.run(session, "script::hi", None).await);
    assert!(!first_run);
    assert_eq!(changed_files[0].change, RunChange::Modified);

    // Resetting drops the edit; an unknown target is refused.
    client
        .call(Request::ResetRunTarget {
            session,
            id: "script::hi".into(),
        })
        .await
        .unwrap();
    assert_eq!(client.run_targets(session).await, [("script::hi".to_string(), false)]);
    let unknown = Request::Run {
        session,
        target_id: "nope".into(),
        approved_hash: None,
    };
    assert_eq!(client.call(unknown).await.unwrap_err(), "no run target nope");
}

#[tokio::test]
async fn the_approval_diff_pairs_renamed_build_files() {
    let make = "build:\n\techo one\n\techo two\n\techo three\n\techo four\n";
    let just = "default:\n\techo a\n\techo b\n\techo c\n\techo d\n";
    let fx = fixture(
        &[("package.json", PACKAGE), ("Makefile", make), ("justfile", just)],
        &[],
    )
    .await;
    let client = Client::new(&fx.handler);
    let session = open(&client, fx.repo.path()).await;
    client.save_target(session, sh_target("t", "true")).await.unwrap();
    let (hash, ..) = needs_approval(client.run(session, "t", None).await);
    started(client.run(session, "t", Some(&hash)).await);

    std::fs::remove_file(fx.repo.path().join("Makefile")).unwrap();
    std::fs::remove_file(fx.repo.path().join("justfile")).unwrap();
    let edited = make.replace("four", "five");
    write(fx.repo.path(), "sub/Makefile", &edited);
    write(fx.repo.path(), "sub/justfile", just);
    let (hash, _, changed) = needs_approval(client.run(session, "t", None).await);
    assert_eq!(changed.len(), 4);
    let Response::RunApprovalDiff { files } = client.call(Request::RunApprovalDiff { session, hash }).await.unwrap()
    else {
        panic!("not a diff");
    };
    let summary: Vec<_> = files
        .iter()
        .map(|f| {
            (
                f.path.as_str(),
                f.from.as_deref(),
                f.base.as_deref(),
                f.target.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("sub/Makefile", Some("Makefile"), Some(make), Some(edited.as_str())),
            ("sub/justfile", Some("justfile"), Some(just), Some(just)),
        ]
    );
    assert_eq!(files[0].hunks.len(), 1);
    assert!(files[1].hunks.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn run_output_is_bounded_and_the_process_is_not_blocked() {
    let fx = fixture(&[("package.json", PACKAGE)], &[]).await;
    let mut client = Client::new(&fx.handler);
    let session = open(&client, fx.repo.path()).await;
    let total = OUTPUT_CAP + 1_000_000;
    let script = format!("head -c {total} /dev/zero; echo done >&2; exit 7");
    client.save_target(session, sh_target("loud", &script)).await.unwrap();
    let (hash, ..) = needs_approval(client.run(session, "loud", None).await);

    let (run, _) = started(client.run(session, "loud", Some(&hash)).await);
    let (out, err, code, dropped) = client.output_bounded(run).await;
    assert_eq!(code, Some(7));
    assert_eq!(out.len() + err.len(), OUTPUT_CAP);
    assert_eq!(dropped as usize, total + "done\n".len() - OUTPUT_CAP);
}

fn pid_is_gone(pid: &str) -> bool {
    !Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[cfg(unix)]
#[tokio::test]
async fn a_run_can_be_stopped_and_dies_with_its_session() {
    let fx = fixture(&[("package.json", PACKAGE)], &[]).await;
    let mut client = Client::new(&fx.handler);
    let session = open(&client, fx.repo.path()).await;
    let pids = tempfile::tempdir().unwrap();
    let pid_file = pids.path().join("pid");
    let long = format!("echo $$ > {}; exec sleep 30", pid_file.display());
    client.save_target(session, sh_target("long", &long)).await.unwrap();
    let (hash, ..) = needs_approval(client.run(session, "long", None).await);

    let (run, _) = started(client.run(session, "long", Some(&hash)).await);
    client.call(Request::StopRun { session, run_id: run }).await.unwrap();
    assert_eq!(client.output(run).await, (String::new(), String::new(), None));
    client.call(Request::StopRun { session, run_id: run }).await.unwrap();

    // Closing the session kills what still runs.
    std::fs::remove_file(&pid_file).ok();
    let (run, _) = started(client.run(session, "long", None).await);
    let pid = loop {
        match std::fs::read_to_string(&pid_file) {
            Ok(pid) if pid.ends_with('\n') => break pid.trim().to_string(),
            _ => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    };
    assert!(!pid_is_gone(&pid));
    client.call(Request::CloseSession { session }).await.unwrap();
    let mut waited = 0;
    while !pid_is_gone(&pid) {
        assert!(waited < 250, "run {run} outlived its session");
        tokio::time::sleep(Duration::from_millis(20)).await;
        waited += 1;
    }
}
