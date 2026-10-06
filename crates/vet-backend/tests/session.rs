//! The handler against a real temporary repository: open, manifest, mark,
//! edit, settle, remap, revert (SPEC §12 criterion 1's split and revert cases,
//! and criterion 4's settle path). `git` is used only to build the fixture.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use vet_backend::{Config, Orchestrator, open_store};
use vet_protocol::{
    AnchorState, Base, BaseChoice, Completion, DirEntry, DirEntryKind, Event, FileStatus, HunkState, LineLocation,
    Manifest, NoTextReason, NonTextKind, Request, Response, ReviewUpdate, ServerMessage, SessionId, Side, TextContent,
    Thread, ThreadStatus, ThreadTarget,
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
}

impl Client {
    fn new(handler: &std::sync::Arc<Orchestrator>) -> Self {
        let (events, inbox) = Events::channel();
        Self {
            handler: handler.clone(),
            events,
            inbox,
        }
    }

    async fn call(&self, request: Request) -> Result<Response, String> {
        self.handler.handle(request, self.events.clone()).await
    }

    async fn manifest(&self, session: SessionId) -> Manifest {
        match self.call(Request::Manifest { session }).await.unwrap() {
            Response::Manifest(m) => m,
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

    /// The next event; panics if none arrives.
    async fn event(&mut self) -> Event {
        let message = tokio::time::timeout(Duration::from_secs(10), self.inbox.recv())
            .await
            .expect("no event")
            .expect("connection closed");
        match message {
            ServerMessage::Event { event } => event,
            other => panic!("{other:?}"),
        }
    }

    async fn settled(&mut self) -> u32 {
        match self.event().await {
            Event::SnapshotSettled { revision, .. } => revision,
            other => panic!("{other:?}"),
        }
    }

    fn no_event(&mut self) -> bool {
        self.inbox.try_recv().is_err()
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
async fn an_orphaned_session_is_closed_after_the_grace_period() {
    let fx = fixture(&[("a.txt", &text(5, &[]))], &[("a.txt", &text(5, &[1]))]).await;
    let first = Client::new(&fx.handler);
    let session = open(&first, fx.repo.path()).await;
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
    assert_eq!(session_refs(fx.repo.path()), "");
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

#[tokio::test]
async fn hover_is_not_implemented() {
    let fx = fixture(&[("a.txt", "x\n")], &[]).await;
    let client = Client::new(&fx.handler);
    assert!(
        client
            .call(Request::Hover {
                session: 1,
                path: "a".into(),
                line: 0,
                col: 0
            })
            .await
            .is_err()
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

    // The pin: only the dirty edit.
    let m = client.manifest(session).await;
    assert_eq!(m.base.choice, BaseChoice::Pin);
    assert_eq!((m.base.commit.as_str(), m.base.label.as_str()), (two.as_str(), "pin"));
    assert_eq!(m.files[0].hunks.len(), 1);
    assert_eq!(lines(&m.completion), (2, 0));

    // The merge base with main: the branch's change shows as well.
    let base = client.set_base(session, merge_base("main")).await.unwrap();
    assert_eq!(
        (base.commit.as_str(), base.label.as_str()),
        (main.as_str(), "merge-base main")
    );
    assert_eq!(client.settled().await, 1);
    assert_eq!(session_base_ref(p, session), main);
    let m = client.manifest(session).await;
    assert_eq!((m.revision, &m.base), (1, &base));
    assert_eq!(hunk_states(&m), [HunkState::Unseen, HunkState::Unseen]);
    assert_eq!((m.files[0].base_lines, m.files[0].target_lines), (40, 39));

    // Mark the branch's hunk.
    let update = client.set_seen(session, 1, &[0], true).await;
    assert_eq!(update.review.hunks, [HunkState::Seen, HunkState::Unseen]);

    // Against the first branch commit, base lines of the marked hunk have changed.
    let base = client
        .set_base(session, BaseChoice::Commit { id: one[..10].into() })
        .await
        .unwrap();
    assert_eq!((base.commit.as_str(), base.label.as_str()), (one.as_str(), "commit"));
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

    // Back to the pin; the mark lies outside what is left to review.
    let base = client.set_base(session, BaseChoice::Pin).await.unwrap();
    assert_eq!((base.commit.as_str(), base.label.as_str()), (two.as_str(), "pin"));
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
