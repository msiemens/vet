//! Acceptance tests for SPEC §12 POC criteria 1 (anchor robustness, I3) and 4
//! (reconciliation), one test per sub-case, against real temporary repositories
//! and real git operations (amend, rebase, mv, rm, update-index).
//!
//! Criterion 1 goes through the protocol like a shell would. Criterion 4 needs to
//! lose watcher events, which the real watcher cannot be told to do, so those
//! tests drive a `LiveSession` over real git with a scripted `Watcher`; the one
//! test that goes through the real watcher is a write storm.

use std::collections::HashSet;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::common::{git, handler, init, repo_with, write};
use tempfile::TempDir;
use tokio::sync::mpsc::{Receiver, UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use vet_app::ports::{Degraded, Dirty, Scanner, Store, WatchEvent, Watcher};
use vet_app::progress::Tasks;
use vet_app::session::{FileStatus as AppFileStatus, LiveSession};
use vet_app::settle::Settling;
use vet_backend::{Orchestrator, open_store};
use vet_core::completion::HunkState as CoreHunk;
use vet_core::content::SnapshotId;
use vet_infra::clock::TokioClock;
use vet_infra::git::GixRepo;
use vet_infra::materialize::Materialize;
use vet_infra::materialize::kept::{KeptTrees, Limits, Referenced};
use vet_infra::store::SqliteStore;
use vet_protocol::{
    AnchorState, AnchorStrategy, ChangesetKey, EntryKind, Event, Failure, FileChange, HunkState, LineLocation,
    Manifest, NonTextKind, ProjectId, Request, Response, ReviewFilter, ServerMessage, Side, StaleReason, Thread,
    ThreadKind, ThreadTarget,
};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::Materializer;
use vet_transport::{Events, Handler};

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

/// Commits `files` on `main` from a second worktree, so the checkout under
/// review stays on its feature branch.
fn advance_main(repo: &Path, files: &[(&str, &str)]) {
    let other = tempfile::tempdir().unwrap();
    let dir = other.path().join("main");
    git(repo, &["worktree", "add", "-q", dir.to_str().unwrap(), "main"]);
    for (name, content) in files {
        write(&dir, name, content);
    }
    git(&dir, &["commit", "-q", "-a", "-m", "main moves on"]);
    git(repo, &["worktree", "remove", "--force", dir.to_str().unwrap()]);
}

struct Fixture {
    repo: TempDir,
    _data: TempDir,
    handler: Arc<Orchestrator>,
}

async fn fixture(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Fixture {
    fixture_of(repo_with(base, dirty)).await
}

async fn fixture_of(repo: TempDir) -> Fixture {
    let (handler, data) = handler().await;
    Fixture {
        repo,
        _data: data,
        handler,
    }
}

struct Client {
    handler: Arc<Orchestrator>,
    events: Events,
    inbox: Receiver<ServerMessage>,
}

impl Client {
    fn new(handler: &Arc<Orchestrator>) -> Self {
        let (events, inbox) = Events::channel();
        Self {
            handler: handler.clone(),
            events,
            inbox,
        }
    }

    async fn call(&self, request: Request) -> Result<Response, Failure> {
        self.handler.handle(request, self.events.clone()).await
    }

    async fn open(&self, repo: &Path) -> ProjectId {
        let request = Request::OpenProject {
            repo: repo.to_str().unwrap().into(),
        };
        match self.call(request).await.unwrap() {
            Response::Project { project, .. } => project,
            other => panic!("{other:?}"),
        }
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

    async fn toggle_hunk(&self, project: ProjectId, path: &str, hunk: u32) {
        let revision = self.manifest(project).await.revision;
        let request = Request::ToggleHunk {
            view: vet_protocol::View::Changeset,
            project,
            revision,
            path: path.into(),
            hunk,
        };
        assert!(matches!(self.call(request).await.unwrap(), Response::Review(_)));
    }

    async fn acknowledge(&self, project: ProjectId, path: &str, kind: NonTextKind) {
        let revision = self.manifest(project).await.revision;
        let request = Request::ToggleItem {
            view: vet_protocol::View::Changeset,
            project,
            revision,
            path: path.into(),
            kind,
        };
        assert!(matches!(self.call(request).await.unwrap(), Response::Review(_)));
    }

    /// Puts the local branch `name` on view: its base follows it as it is rebased.
    async fn open_branch(&self, project: ProjectId, name: &str) {
        let changeset = ChangesetKey::Branch { name: name.into() };
        self.call(Request::OpenChangeset { project, changeset }).await.unwrap();
    }

    async fn set_filter(&self, project: ProjectId, filter: ReviewFilter) {
        self.call(Request::SetProjectFilter { project, filter }).await.unwrap();
    }

    /// A range thread on the target side at the current revision.
    async fn comment(&self, project: ProjectId, path: &str, start: u32, len: u32) -> Thread {
        let revision = self.manifest(project).await.revision;
        let target = ThreadTarget::Range {
            side: Side::Target,
            path: path.into(),
            start,
            len,
        };
        self.create(project, revision, target).await
    }

    async fn create(&self, project: ProjectId, revision: u32, target: ThreadTarget) -> Thread {
        let request = Request::CreateThread {
            view: vet_protocol::View::Changeset,
            project,
            revision,
            target,
            body: "why?".into(),
        };
        match self.call(request).await.unwrap() {
            Response::Thread(thread) => thread,
            other => panic!("{other:?}"),
        }
    }

    async fn threads(&self, project: ProjectId) -> Vec<Thread> {
        match self
            .call(Request::Threads {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .unwrap()
        {
            Response::Threads { threads, .. } => threads,
            other => panic!("{other:?}"),
        }
    }

    async fn thread(&self, project: ProjectId, id: u32) -> Thread {
        self.threads(project).await.into_iter().find(|t| t.id == id).unwrap()
    }

    /// The next settled revision; panics if none arrives.
    async fn settled(&mut self) -> u32 {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), self.inbox.recv())
                .await
                .expect("no event")
                .expect("connection closed");
            if let ServerMessage::Event {
                event: Event::SnapshotSettled { revision, .. },
            } = message
            {
                return revision;
            }
        }
    }

    /// Waits until the project has no background work running or pending, after something outside
    /// the protocol (git, the filesystem) changed the repository, and drains what it sent before it
    /// went quiet.
    async fn quiet(&mut self, project: ProjectId) {
        self.call(Request::WaitForQuiet { project }).await.unwrap();
        while self.inbox.try_recv().is_ok() {}
    }
}

fn file<'a>(m: &'a Manifest, path: &str) -> &'a FileChange {
    m.files.iter().find(|f| f.path == path).unwrap()
}

fn hunks(m: &Manifest, path: &str) -> Vec<HunkState> {
    file(m, path).review.hunks.clone()
}

const EXACT: AnchorState = AnchorState::Exact {
    whitespace_changed: false,
};

fn at(path: &str, start: u32, len: u32) -> Option<LineLocation> {
    Some(LineLocation {
        path: path.into(),
        start,
        len,
    })
}

// ---------------------------------------------------------------------------
// Criterion 1: anchor robustness
// ---------------------------------------------------------------------------

#[tokio::test]
async fn criterion1_amend_keeps_marks_and_comments() {
    let fx = fixture(&[("a.txt", &text(40, &[]))], &[("a.txt", &text(40, &[5]))]).await;
    let p = fx.repo.path();
    // The agent committed once already; that commit is the branch's tip, which
    // the review takes as its base when the first mark starts it.
    git(p, &["commit", "-q", "-a", "-m", "agent work"]);
    let pinned = git(p, &["rev-parse", "HEAD"]);
    write(p, "a.txt", &text(40, &[5, 30]));
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    client.toggle_hunk(project, "a.txt", 0).await;
    let comment = client.comment(project, "a.txt", 30, 1).await;
    assert_eq!(hunks(&client.manifest(project).await, "a.txt"), [HunkState::Seen]);

    // Folding the dirty edit into that commit rewrites it; git may drop the
    // old one for good, the session's base ref keeps it.
    git(p, &["commit", "-q", "-a", "--amend", "--no-edit"]);
    git(p, &["reflog", "expire", "--expire=now", "--all"]);
    git(p, &["gc", "-q", "--prune=now"]);
    client.quiet(project).await;

    assert_ne!(git(p, &["rev-parse", "HEAD"]), pinned);
    let m = client.manifest(project).await;
    assert_eq!(m.base.commit.id, pinned);
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Seen]);
    let thread = client.thread(project, comment.id).await;
    assert_eq!((thread.anchor, thread.location), (Some(EXACT), at("a.txt", 30, 1)));

    write(p, "a.txt", &text(40, &[5, 20, 30]));
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Unseen, HunkState::Seen]);
    let Response::FileContents { base, .. } = client
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
    assert_eq!(base.as_deref(), Some(text(40, &[5]).as_str()));
}

#[tokio::test]
async fn criterion1_rebase_moves_hunks_and_marks_and_comments_follow() {
    let fx = fixture(&[("a.txt", &text(40, &[]))], &[]).await;
    let p = fx.repo.path();
    git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", &text(40, &[20]));
    git(p, &["commit", "-q", "-a", "-m", "feature"]);
    write(p, "a.txt", &text(40, &[20, 30]));
    git(p, &["commit", "-q", "-a", "-m", "feature again"]);
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    client.open_branch(project, "feature").await;
    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Unseen, HunkState::Unseen]);
    client.toggle_hunk(project, "a.txt", 0).await;
    let comment = client.comment(project, "a.txt", 20, 1).await;

    advance_main(p, &[("a.txt", &format!("header\n{}", text(40, &[])))]);
    git(p, &["rebase", "-q", "main"]);
    client.quiet(project).await;

    // The base is where the branch left main now: only the feature's own changes are left, and
    // the marks and the comment moved down by one line.
    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Seen, HunkState::Unseen]);
    let thread = client.thread(project, comment.id).await;
    assert_eq!((thread.anchor, thread.location), (Some(EXACT), at("a.txt", 21, 1)));
}

#[tokio::test]
async fn criterion1_a_rebase_that_changes_the_base_side_marks_seen_against_old_base() {
    let fx = fixture(&[("a.txt", &text(40, &[]))], &[]).await;
    let p = fx.repo.path();
    git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "a.txt", &text(40, &[7, 8, 9]));
    git(p, &["commit", "-q", "-a", "-m", "feature"]);
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    client.open_branch(project, "feature").await;
    client.toggle_hunk(project, "a.txt", 0).await;
    assert_eq!(hunks(&client.manifest(project).await, "a.txt"), [HunkState::Seen]);

    // Main drops a line inside what the feature edited; the rebase keeps the
    // feature's version, so the target side does not change but the base side does.
    advance_main(p, &[("a.txt", &text(40, &[]).replace("line 8\n", ""))]);
    git(p, &["rebase", "-q", "-X", "theirs", "main"]);
    assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), text(40, &[7, 8, 9]));
    client.quiet(project).await;

    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::SeenAgainstOldBase]);
    // The three target lines count as unseen; of the two base lines left, both are seen.
    assert_eq!(
        (m.completion.lines, m.completion.seen_lines, m.completion.old_base_lines),
        (5, 2, 3)
    );

    client.toggle_hunk(project, "a.txt", 0).await;
    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Seen]);
    assert_eq!(m.completion.old_base_lines, 0);
}

#[tokio::test]
async fn criterion1_a_seen_deletion_survives_an_unrelated_rebase() {
    let fx = fixture(&[("a.txt", &text(40, &[]))], &[]).await;
    let p = fx.repo.path();
    git(p, &["checkout", "-q", "-b", "feature"]);
    let without = text(40, &[]).replace("line 10\nline 11\nline 12\n", "");
    write(p, "a.txt", &without);
    git(p, &["commit", "-q", "-a", "-m", "delete three lines"]);
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    client.open_branch(project, "feature").await;
    let m = client.manifest(project).await;
    assert_eq!(
        (
            file(&m, "a.txt").hunks[0].base_len,
            file(&m, "a.txt").hunks[0].target_len
        ),
        (3, 0)
    );
    client.toggle_hunk(project, "a.txt", 0).await;

    // Main changes the file elsewhere: a header on top and an edit far below.
    advance_main(p, &[("a.txt", &format!("header\n{}", text(40, &[30])))]);
    git(p, &["rebase", "-q", "main"]);
    client.quiet(project).await;

    // The base is where the branch left main now: the deletion stays seen.
    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Seen]);
    assert_eq!((m.completion.lines, m.completion.seen_lines), (3, 3));
}

#[tokio::test]
async fn criterion1_a_crlf_only_change_is_whitespace_exact_for_comments_and_unseen_for_marks() {
    let dirty = text(10, &[3]);
    let fx = fixture(&[("a.txt", &text(10, &[]))], &[("a.txt", &dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let project = client.open(fx.repo.path()).await;
    client.toggle_hunk(project, "a.txt", 0).await;
    let comment = client.comment(project, "a.txt", 2, 3).await;

    write(fx.repo.path(), "a.txt", &dirty.replace('\n', "\r\n"));
    client.settled().await;

    let thread = client.thread(project, comment.id).await;
    assert_eq!(
        thread.anchor,
        Some(AnchorState::Exact {
            whitespace_changed: true
        })
    );
    assert_eq!(thread.location, at("a.txt", 2, 3));
    // A mark covers identical content only: the line-ending change is new content.
    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Unseen]);
    // Only the base-side line, whose text did not change, is still seen.
    assert_eq!(m.completion.seen_lines, 1);
}

#[tokio::test]
async fn criterion1_a_filter_change_leaves_review_state_intact_and_counts_the_excluded() {
    let fx = fixture(
        &[("a.txt", &text(10, &[])), ("docs/b.md", "1\n")],
        &[("a.txt", &text(10, &[2])), ("docs/b.md", "2\n")],
    )
    .await;
    let mut client = Client::new(&fx.handler);
    let project = client.open(fx.repo.path()).await;
    client.toggle_hunk(project, "a.txt", 0).await;
    let comment = client.comment(project, "a.txt", 2, 1).await;
    let m = client.manifest(project).await;
    assert_eq!(
        (m.completion.lines, m.completion.seen_lines, m.excluded_lines),
        (4, 2, 0)
    );

    let all = ReviewFilter {
        exclude: vec!["a.txt".into(), "docs/**".into()],
        ..Default::default()
    };
    client.set_filter(project, all).await;
    client.settled().await;
    let m = client.manifest(project).await;
    assert!(m.files.is_empty());
    assert_eq!((m.completion.lines, m.excluded_lines), (0, 4));

    client.set_filter(project, ReviewFilter::default()).await;
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Seen]);
    assert_eq!(hunks(&m, "docs/b.md"), [HunkState::Unseen]);
    assert_eq!(
        (m.completion.lines, m.completion.seen_lines, m.excluded_lines),
        (4, 2, 0)
    );

    // Excluding only what is unseen completes the review, and says what it hides.
    let docs = ReviewFilter {
        exclude: vec!["docs/**".into()],
        ..Default::default()
    };
    client.set_filter(project, docs).await;
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(
        (m.completion.lines, m.completion.seen_lines, m.excluded_lines),
        (2, 2, 2)
    );
    let thread = client.thread(project, comment.id).await;
    assert_eq!((thread.anchor, thread.location), (Some(EXACT), at("a.txt", 2, 1)));
}

#[tokio::test]
async fn criterion1_a_seen_line_that_appears_twice_is_unseen_once_its_context_is_rewritten() {
    let base = "fn a() {\n}\nfn b() {\n}\n";
    let dirty = "fn a() {\n    return 0;\n}\nfn b() {\n    return 0;\n}\n";
    let fx = fixture(&[("a.rs", base)], &[("a.rs", dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let project = client.open(fx.repo.path()).await;
    client.toggle_hunk(project, "a.rs", 0).await;
    assert_eq!(
        hunks(&client.manifest(project).await, "a.rs"),
        [HunkState::Seen, HunkState::Unseen]
    );

    // Unrelated edit of the other copy: the mark stays.
    write(
        fx.repo.path(),
        "a.rs",
        "fn a() {\n    return 0;\n}\nfn b() {\n    return 1;\n}\n",
    );
    client.settled().await;
    assert_eq!(
        hunks(&client.manifest(project).await, "a.rs"),
        [HunkState::Seen, HunkState::Unseen]
    );

    // The context of both copies is rewritten: which `return 0` was seen is no
    // longer knowable, so none is.
    write(
        fx.repo.path(),
        "a.rs",
        "fn c() {\n    return 0;\n}\nfn d() {\n    return 0;\n}\n",
    );
    client.settled().await;
    let m = client.manifest(project).await;
    assert!(hunks(&m, "a.rs").iter().all(|h| *h == HunkState::Unseen));
    assert_eq!(m.completion.seen_lines, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn criterion1_a_mode_only_changeset_is_not_complete_until_acknowledged() {
    let fx = fixture(&[("run.sh", "echo\n"), ("a.txt", &text(5, &[]))], &[]).await;
    let p = fx.repo.path();
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    let complete = |m: &Manifest| {
        m.completion.seen_lines == m.completion.lines && m.completion.acknowledged_items == m.completion.items
    };
    assert!(complete(&client.manifest(project).await), "an empty changeset");

    std::fs::set_permissions(p.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(m.files.len(), 1);
    assert_eq!((m.completion.lines, m.completion.items), (0, 1));
    assert!(!complete(&m));

    client.acknowledge(project, "run.sh", NonTextKind::Mode).await;
    let m = client.manifest(project).await;
    assert!(complete(&m));
    assert_eq!(file(&m, "run.sh").review.acknowledged, [NonTextKind::Mode]);

    // The acknowledgement survives an unrelated settle; that edit is what is left to review.
    write(p, "a.txt", &text(5, &[1]));
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(file(&m, "run.sh").review.acknowledged, [NonTextKind::Mode]);
    assert!(!complete(&m));

    // Changing the mode back leaves no change at all.
    std::fs::set_permissions(p.join("run.sh"), std::fs::Permissions::from_mode(0o644)).unwrap();
    client.settled().await;
    let m = client.manifest(project).await;
    assert!(m.files.iter().all(|f| f.path != "run.sh"));
    assert_eq!(m.completion.items, 0);
}

#[tokio::test]
async fn criterion1_windows_filesystem_mode_comes_from_the_index_when_filemode_is_off() {
    let fx = fixture(&[("run.sh", "echo\n"), ("plain.txt", "x\n")], &[]).await;
    let p = fx.repo.path();
    git(p, &["config", "core.filemode", "false"]);
    git(p, &["update-index", "--chmod=+x", "run.sh"]);
    git(p, &["commit", "-q", "-m", "run.sh is executable"]);
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;

    // On disk the mode bits are whatever the filesystem says; git is told to ignore them.
    #[cfg(unix)]
    {
        let set = |name: &str, mode| std::fs::set_permissions(p.join(name), std::fs::Permissions::from_mode(mode));
        set("run.sh", 0o644).unwrap();
        set("plain.txt", 0o755).unwrap();
    }
    client.quiet(project).await;
    assert!(client.manifest(project).await.files.is_empty());

    git(p, &["update-index", "--chmod=-x", "run.sh"]);
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(m.files.len(), 1);
    let item = &file(&m, "run.sh").non_text[0];
    assert_eq!(item.kind, NonTextKind::Mode);
    assert_eq!(
        (item.before.as_ref().unwrap().mode, item.after.as_ref().unwrap().mode),
        (0o100755, 0o100644)
    );

    // An untracked file is a plain file, whatever the filesystem says.
    write(p, "new.sh", "echo\n");
    #[cfg(unix)]
    {
        std::fs::set_permissions(p.join("new.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    client.settled().await;
    let m = client.manifest(project).await;
    let new = file(&m, "new.sh");
    assert_eq!(new.status, vet_protocol::FileStatus::Added);
    assert!(new.non_text.is_empty());
}

#[tokio::test]
async fn criterion1_windows_filesystem_a_symlink_stays_a_symlink_when_symlinks_are_off() {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    init(p);
    git(p, &["config", "core.symlinks", "false"]);
    write(p, "target.txt", "x\n");
    write(p, "other.txt", "y\n");
    // What a checkout leaves with core.symlinks=false: a plain file holding the
    // link's target. The index entry still says `120000`.
    write(p, "link", "target.txt");
    git(p, &["add", "target.txt", "other.txt"]);
    let blob = git(p, &["hash-object", "-w", "link"]);
    git(
        p,
        &["update-index", "--add", "--cacheinfo", &format!("120000,{blob},link")],
    );
    git(p, &["commit", "-q", "-m", "base"]);
    let fx = fixture_of(repo).await;
    let p = fx.repo.path();
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    assert!(client.manifest(project).await.files.is_empty(), "no change");

    write(p, "link", "other.txt");
    client.settled().await;
    let m = client.manifest(project).await;
    assert_eq!(m.files.len(), 1);
    let link = file(&m, "link");
    assert!(link.hunks.is_empty());
    assert_eq!(link.non_text.len(), 1);
    assert_eq!(link.non_text[0].kind, NonTextKind::Symlink);
    let (before, after) = (link.non_text[0].before.as_ref(), link.non_text[0].after.as_ref());
    assert_eq!(before.unwrap().kind, EntryKind::Symlink);
    assert_eq!(after.unwrap().kind, EntryKind::Symlink);
    assert_eq!(after.unwrap().target.as_deref(), Some("other.txt"));
}

/// A repository whose `main` commit holds `Foo.txt` and `foo.txt`, made with
/// plumbing so that it works on a case-insensitive checkout too.
fn colliding_repo() -> TempDir {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    init(p);
    let scratch = tempfile::tempdir().unwrap();
    for (name, content) in [("one", "1\n"), ("two", "2\n")] {
        write(scratch.path(), name, content);
    }
    for (path, scratch_name) in [("Foo.txt", "one"), ("foo.txt", "two")] {
        let blob = git(
            p,
            &["hash-object", "-w", scratch.path().join(scratch_name).to_str().unwrap()],
        );
        git(
            p,
            &["update-index", "--add", "--cacheinfo", &format!("100644,{blob},{path}")],
        );
    }
    git(p, &["commit", "-q", "-m", "base"]);
    repo
}

/// A session whose review has started, so that its snapshots are stored.
async fn collision_session(repo: &Path) -> (LiveSession<GixRepo, SqliteStore>, SqliteStore, TempDir) {
    let data = tempfile::tempdir().unwrap();
    let store = open_store(data.path()).await.unwrap();
    let session = LiveSession::open(GixRepo::for_test(repo), store.clone(), "/repo".into(), || 0)
        .await
        .unwrap();
    session
        .create_thread(
            0,
            vet_app::session::View::Changeset,
            vet_app::session::NewThread::Changeset,
            "start".into(),
            0,
        )
        .await
        .unwrap();
    (session, store, data)
}

/// Windows and macOS keep case-insensitive filesystems, where two such files cannot exist.
#[cfg(not(any(windows, target_os = "macos")))]
fn is_case_sensitive(dir: &Path) -> bool {
    write(dir, "probe.txt", "1\n");
    write(dir, "PROBE.txt", "2\n");
    let names = std::fs::read_dir(dir).unwrap().count();
    std::fs::remove_file(dir.join("probe.txt")).unwrap();
    std::fs::remove_file(dir.join("PROBE.txt")).ok();
    names == 2
}

const POLICY: AnalysisPolicy = AnalysisPolicy::LIMITED;

fn kept_trees(data: &Path) -> Arc<KeptTrees> {
    let none: Arc<dyn Referenced> = Arc::new(HashSet::<SnapshotId>::new);
    Arc::new(KeptTrees::for_test(data.join("materialized"), Limits::default(), none))
}

/// Only where files can differ in case alone; `..._in_a_commit_...` covers the other filesystems.
#[cfg(not(any(windows, target_os = "macos")))]
#[tokio::test]
async fn criterion1_case_only_paths_in_the_worktree_are_flagged_and_materialized_on_a_case_sensitive_filesystem() {
    let repo = repo_with(&[("a.txt", "a\n")], &[]);
    let p = repo.path();
    assert!(
        is_case_sensitive(p),
        "the temporary directory's filesystem ignores case"
    );
    write(p, "Foo.txt", "1\n");
    write(p, "foo.txt", "2\n");
    let (session, store, data) = collision_session(p).await;
    let sides = session.view().semantic_sides(0).await.unwrap();

    let (_, status) = store.snapshot(sides.target).await.unwrap().unwrap();
    let flagged: Vec<_> = status.path_collisions.iter().map(|p| p.0.clone()).collect();
    assert_eq!(flagged, [b"Foo.txt".to_vec(), b"foo.txt".to_vec()]);
    // The tree is only refused where the filesystem would merge the two.
    let result = Materialize::new(session.acquisition(), kept_trees(data.path()))
        .materialize(sides.target, POLICY)
        .await;
    assert!(result.is_ok(), "{:?}", result.err());
}

#[tokio::test]
async fn criterion1_windows_filesystem_case_only_paths_in_a_commit_are_flagged_and_not_materialized_where_they_merge() {
    let repo = colliding_repo();
    let (session, store, data) = collision_session(repo.path()).await;
    let sides = session.view().semantic_sides(0).await.unwrap();

    let (_, status) = store.snapshot(sides.base).await.unwrap().unwrap();
    let flagged: Vec<_> = status.path_collisions.iter().map(|p| p.0.clone()).collect();
    assert_eq!(flagged, [b"Foo.txt".to_vec(), b"foo.txt".to_vec()]);
    let result = Materialize::new(session.acquisition(), kept_trees(data.path()))
        .materialize(sides.base, POLICY)
        .await;
    assert_eq!(result.is_err(), cfg!(any(windows, target_os = "macos")));
}

#[tokio::test]
async fn criterion1_a_rename_keeps_marks_comments_and_file_threads() {
    let fx = fixture(&[("a.txt", &text(40, &[]))], &[("a.txt", &text(40, &[5]))]).await;
    let p = fx.repo.path();
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    client.toggle_hunk(project, "a.txt", 0).await;
    let range = client.comment(project, "a.txt", 5, 1).await;
    let whole_file = client
        .create(
            project,
            0,
            ThreadTarget::File {
                side: Side::Target,
                path: "a.txt".into(),
            },
        )
        .await;

    git(p, &["mv", "a.txt", "b.txt"]);
    client.settled().await;

    let m = client.manifest(project).await;
    assert_eq!(m.files.len(), 1);
    let renamed = file(&m, "b.txt");
    assert_eq!(
        (renamed.status, renamed.old_path.as_deref()),
        (vet_protocol::FileStatus::Renamed, Some("a.txt"))
    );
    assert_eq!(hunks(&m, "b.txt"), [HunkState::Seen]);
    let range = client.thread(project, range.id).await;
    assert_eq!((range.anchor, range.location), (Some(EXACT), at("b.txt", 5, 1)));
    let whole_file = client.thread(project, whole_file.id).await;
    assert_eq!(
        (whole_file.kind, whole_file.anchor, whole_file.path.as_deref()),
        (ThreadKind::File, Some(EXACT), Some("b.txt"))
    );
}

#[tokio::test]
async fn criterion1_an_edit_inside_the_anchor_range_is_inferred_until_confirmed() {
    let dirty = text(30, &[3]);
    let fx = fixture(&[("a.txt", &text(30, &[]))], &[("a.txt", &dirty)]).await;
    let mut client = Client::new(&fx.handler);
    let project = client.open(fx.repo.path()).await;
    let comment = client.comment(project, "a.txt", 10, 3).await;

    write(fx.repo.path(), "a.txt", &text(30, &[3, 11]));
    let revision = client.settled().await;

    let thread = client.thread(project, comment.id).await;
    assert!(
        matches!(
            thread.anchor,
            Some(AnchorState::Inferred {
                strategy: AnchorStrategy::ContextWindow,
                ..
            })
        ),
        "{:?}",
        thread.anchor
    );
    assert_eq!(
        (thread.location, thread.original),
        (at("a.txt", 10, 3), at("a.txt", 10, 3))
    );

    let request = Request::ConfirmAnchor {
        project,
        revision,
        thread: comment.id,
    };
    let Response::Thread(confirmed) = client.call(request).await.unwrap() else {
        panic!("not a thread");
    };
    assert_eq!(confirmed.anchor, Some(EXACT));
}

#[tokio::test]
async fn criterion1_duplicate_content_is_ambiguous_until_one_candidate_is_attached() {
    let block = "dup A\ndup B\ndup C\n";
    let before = format!("p1\np2\np3\n{block}q1\nq2\nq3\n");
    let fx = fixture(&[("a.txt", "x\n")], &[("a.txt", &before)]).await;
    let mut client = Client::new(&fx.handler);
    let project = client.open(fx.repo.path()).await;
    let comment = client.comment(project, "a.txt", 3, 3).await;

    let after = format!("x1\nx2\nx3\n{block}x4\nx5\nx6\nx7\nx8\n{block}y1\ny2\ny3\n");
    write(fx.repo.path(), "a.txt", &after);
    let revision = client.settled().await;

    let thread = client.thread(project, comment.id).await;
    assert_eq!(thread.location, None);
    assert_eq!(
        thread.anchor,
        Some(AnchorState::Ambiguous {
            candidates: vec![at("a.txt", 3, 3).unwrap(), at("a.txt", 11, 3).unwrap()]
        })
    );
    let request = Request::AttachAnchor {
        project,
        revision,
        thread: comment.id,
        candidate: 1,
    };
    let Response::Thread(attached) = client.call(request).await.unwrap() else {
        panic!("not a thread");
    };
    assert_eq!((attached.anchor, attached.location), (Some(EXACT), at("a.txt", 11, 3)));
}

#[tokio::test]
async fn criterion1_moved_content_in_another_changed_file_is_inferred_across_files() {
    let a = text(10, &[]);
    let b: String = (0..10).map(|i| format!("b {i}\n")).collect();
    let block = "m1\nm2\nm3\n";
    let insert = |source: &str, before: &str| {
        let at = source.find(before).unwrap();
        format!("{}{block}{}", &source[..at], &source[at..])
    };
    let fx = fixture(&[("a.txt", &a), ("b.txt", &b)], &[("a.txt", &insert(&a, "line 5"))]).await;
    let mut client = Client::new(&fx.handler);
    let project = client.open(fx.repo.path()).await;
    let comment = client.comment(project, "a.txt", 5, 3).await;

    write(fx.repo.path(), "a.txt", &a);
    write(fx.repo.path(), "b.txt", &insert(&b, "b 7"));
    client.quiet(project).await;

    let thread = client.thread(project, comment.id).await;
    assert!(
        matches!(
            thread.anchor,
            Some(AnchorState::Inferred {
                strategy: AnchorStrategy::CrossFileMove,
                ..
            })
        ),
        "{:?}",
        thread.anchor
    );
    assert_eq!(
        (thread.path.as_deref(), thread.location),
        (Some("b.txt"), at("b.txt", 7, 3))
    );
}

#[tokio::test]
async fn criterion1_deleting_a_file_detaches_its_threads_but_not_the_changeset_thread() {
    let fx = fixture(
        &[("a.txt", &text(10, &[])), ("b.txt", "b\n")],
        &[("a.txt", &text(10, &[1]))],
    )
    .await;
    let p = fx.repo.path();
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    client.toggle_hunk(project, "a.txt", 0).await;
    let range = client.comment(project, "a.txt", 2, 3).await;
    let whole_file = client
        .create(
            project,
            0,
            ThreadTarget::File {
                side: Side::Target,
                path: "a.txt".into(),
            },
        )
        .await;
    let changeset = client.create(project, 0, ThreadTarget::Changeset).await;

    git(p, &["rm", "-q", "-f", "a.txt"]);
    client.settled().await;

    let gone = Some(AnchorState::Stale {
        reason: StaleReason::FileDeleted,
    });
    let range = client.thread(project, range.id).await;
    assert_eq!((range.anchor, range.location), (gone.clone(), None));
    assert_eq!(client.thread(project, whole_file.id).await.anchor, gone);
    let changeset = client.thread(project, changeset.id).await;
    assert_eq!(
        (changeset.anchor, changeset.path, changeset.location),
        (None, None, None)
    );
    // The deletion is a change to review. Only the base line the reviewer saw is
    // still covered; the mark on the edited target line is not carried onto it.
    let m = client.manifest(project).await;
    assert_eq!(file(&m, "a.txt").status, vet_protocol::FileStatus::Deleted);
    assert_eq!((m.completion.lines, m.completion.seen_lines), (10, 1));
}

#[tokio::test]
async fn criterion1_a_rewritten_file_leaves_its_thread_stale_for_changed_context() {
    let own: String = (0..30).map(|i| format!("rewritten line {i}\n")).collect();
    let fx = fixture(&[("rewritten.txt", &own)], &[]).await;
    let mut client = Client::new(&fx.handler);
    let project = client.open(fx.repo.path()).await;
    let comment = client.comment(project, "rewritten.txt", 10, 3).await;

    write(fx.repo.path(), "rewritten.txt", "completely\ndifferent\n");
    client.quiet(project).await;

    let thread = client.thread(project, comment.id).await;
    assert_eq!(
        (thread.anchor, thread.location),
        (
            Some(AnchorState::Stale {
                reason: StaleReason::ContextChanged
            }),
            None
        )
    );
}

// ---------------------------------------------------------------------------
// Criterion 4: reconciliation
// ---------------------------------------------------------------------------

/// A watcher whose events the test writes, so events can be lost, doubled or
/// replaced by overflow signals. Platform signals (inotify `IN_Q_OVERFLOW`,
/// FSEvents `MustScanSubDirs`/`UserDropped`/`KernelDropped`, a zero-byte
/// `ReadDirectoryChangesW` result, a handle error) all reach settling as
/// `WatchEvent::MustRescan`; the backends' own tests cover that translation
/// (`vet-infra/src/watch/notify_backend.rs` on macOS and Linux,
/// `windows_backend.rs` on Windows).
struct Scripted {
    events: UnboundedReceiver<WatchEvent>,
    degraded: Option<Degraded>,
    /// Whether the settle loop has taken an event and not asked for the next one yet.
    processing: Arc<AtomicBool>,
    /// Events sent to the loop but not taken yet.
    queued: Arc<AtomicUsize>,
}

impl Watcher for Scripted {
    fn degraded(&self) -> Option<Degraded> {
        self.degraded
    }

    async fn next(&mut self) -> WatchEvent {
        // Asking for an event means the one before it is done.
        self.processing.store(false, Ordering::SeqCst);
        match self.events.recv().await {
            Some(event) => {
                self.queued.fetch_sub(1, Ordering::SeqCst);
                if let WatchEvent::Degraded(reason) = &event {
                    self.degraded = Some(*reason);
                }
                self.processing.store(true, Ordering::SeqCst);
                event
            }
            // A watcher that died silently: nothing more is ever reported.
            None => std::future::pending().await,
        }
    }
}

type Session = LiveSession<GixRepo, SqliteStore>;

struct Live {
    repo: TempDir,
    _data: TempDir,
    session: Arc<Session>,
    watch: UnboundedSender<WatchEvent>,
    revisions: UnboundedReceiver<u32>,
    follower: JoinHandle<()>,
    /// The scans and diffs the session reports.
    tasks: Tasks,
    /// Whether an edit waits to settle.
    pending: Arc<AtomicBool>,
    /// Whether the settle loop has taken an event and not asked for the next one yet.
    processing: Arc<AtomicBool>,
    /// Events sent to the settle loop but not taken yet.
    queued: Arc<AtomicUsize>,
}

impl Live {
    async fn open(base: &[(&str, &str)], dirty: &[(&str, &str)]) -> Self {
        let repo = repo_with(base, dirty);
        let data = tempfile::tempdir().unwrap();
        let store = open_store(data.path()).await.unwrap();
        let tasks = Tasks::default();
        let session = Arc::new(
            Session::open_reporting(
                GixRepo::for_test(repo.path()),
                store,
                "/repo".into(),
                || 0,
                tasks.clone(),
            )
            .await
            .unwrap(),
        );
        let (watch, events) = unbounded_channel();
        let (revisions_tx, revisions) = unbounded_channel();
        let pending = Arc::new(AtomicBool::new(false));
        let processing = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicUsize::new(0));
        let follower = Self::follow(
            &session,
            events,
            revisions_tx,
            pending.clone(),
            processing.clone(),
            queued.clone(),
        )
        .await;
        Self {
            repo,
            _data: data,
            session,
            watch,
            revisions,
            follower,
            tasks,
            pending,
            processing,
            queued,
        }
    }

    async fn follow(
        session: &Arc<Session>,
        events: UnboundedReceiver<WatchEvent>,
        revisions: UnboundedSender<u32>,
        pending: Arc<AtomicBool>,
        processing: Arc<AtomicBool>,
        queued: Arc<AtomicUsize>,
    ) -> JoinHandle<()> {
        let watcher = Scripted {
            events,
            degraded: None,
            processing,
            queued,
        };
        let settling =
            Settling::new(watcher, TokioClock::new(), 100, Some(session.settled_snapshot().await)).reporting_edits();
        let session = session.clone();
        tokio::spawn(async move {
            session
                .follow(
                    settling,
                    |revision| {
                        let _ = revisions.send(revision);
                    },
                    || {},
                    |waiting| pending.store(waiting, Ordering::SeqCst),
                )
                .await;
        })
    }

    fn path(&self) -> &Path {
        self.repo.path()
    }

    fn send(&self, event: WatchEvent) {
        self.queued.fetch_add(1, Ordering::SeqCst);
        self.watch.send(event).unwrap();
    }

    /// The watcher dies without a word and is started again later: a new one
    /// with its own event stream, on the same session.
    async fn kill_watcher(&mut self) {
        self.follower.abort();
        let _ = (&mut self.follower).await;
        self.processing.store(false, Ordering::SeqCst);
    }

    async fn restart_watcher(&mut self) {
        let (watch, events) = unbounded_channel();
        let (revisions_tx, revisions) = unbounded_channel();
        self.watch = watch;
        self.revisions = revisions;
        self.queued.store(0, Ordering::SeqCst);
        self.processing.store(false, Ordering::SeqCst);
        self.follower = Self::follow(
            &self.session,
            events,
            revisions_tx,
            self.pending.clone(),
            self.processing.clone(),
            self.queued.clone(),
        )
        .await;
    }

    async fn revision(&mut self) -> u32 {
        tokio::time::timeout(Duration::from_secs(10), self.revisions.recv())
            .await
            .expect("nothing settled")
            .expect("follower stopped")
    }

    /// Waits until the settle loop has no work pending: nothing queued to it, nothing it is
    /// processing, no edit waiting to settle, and no scan or diff running. The barrier that a
    /// change was reconciled is over, where a sleep would only hope that it is.
    async fn quiet(&self) {
        for _ in 0..6000 {
            if !self.processing.load(Ordering::SeqCst)
                && self.queued.load(Ordering::SeqCst) == 0
                && !self.pending.load(Ordering::SeqCst)
                && !self.tasks.any_running()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the session never went quiet");
    }

    /// Settled revisions until nothing more arrives; the last one.
    async fn settle_all(&mut self) -> Option<u32> {
        let mut last = None;
        while let Ok(Some(revision)) = tokio::time::timeout(Duration::from_millis(900), self.revisions.recv()).await {
            last = Some(revision);
        }
        last
    }

    /// What a from-scratch scan of the tree says the snapshot is.
    async fn oracle(&self) -> vet_core::content::SnapshotId {
        GixRepo::for_test(self.path())
            .scan(Dirty::All)
            .await
            .unwrap()
            .snapshot
            .id
    }

    async fn assert_correct(&self) {
        assert_eq!(self.session.target_snapshot().await.id, self.oracle().await);
    }

    async fn hunks(&self, path: &str) -> Vec<CoreHunk> {
        let m = self.session.manifest(vet_app::session::View::Changeset).await.unwrap();
        m.files
            .iter()
            .find(|f| f.path.0 == path.as_bytes())
            .unwrap_or_else(|| panic!("{path} is not in the manifest"))
            .review
            .hunks
            .clone()
    }

    async fn files(&self) -> Vec<(String, AppFileStatus)> {
        let m = self.session.manifest(vet_app::session::View::Changeset).await.unwrap();
        let mut files: Vec<_> = m
            .files
            .iter()
            .map(|f| (String::from_utf8(f.path.0.clone()).unwrap(), f.status))
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }
}

#[tokio::test]
async fn criterion4_dropped_events_are_caught_up_by_the_next_rescan() {
    let mut live = Live::open(
        &[("a.txt", &text(40, &[])), ("gone.txt", "x\n")],
        &[("a.txt", &text(40, &[5, 30]))],
    )
    .await;
    live.session
        .toggle_hunk(0, vet_app::session::View::Changeset, "a.txt", 0)
        .await
        .unwrap();
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Seen, CoreHunk::Unseen]);

    // The agent rewrites the marked line, adds a file and deletes one; the
    // watcher reports none of it.
    write(live.path(), "a.txt", &text(40, &[30]).replace("line 5\n", "agent 5\n"));
    write(live.path(), "new.txt", "new\n");
    std::fs::remove_file(live.path().join("gone.txt")).unwrap();
    live.quiet().await;
    // Late, not wrong: the old revision still describes the old snapshot.
    assert_eq!(live.session.revision().await, 0);
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Seen, CoreHunk::Unseen]);

    live.send(WatchEvent::MustRescan);
    assert_eq!(live.revision().await, 1);
    live.assert_correct().await;
    assert_eq!(
        live.files().await,
        [
            ("a.txt".into(), AppFileStatus::Modified),
            ("gone.txt".into(), AppFileStatus::Deleted),
            ("new.txt".into(), AppFileStatus::Added),
        ]
    );
    // The mark was on content that is gone, so it is gone too.
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Unseen, CoreHunk::Unseen]);
    live.quiet().await;
    assert_eq!(live.session.revision().await, 1, "settled once");
}

#[tokio::test]
async fn criterion4_an_incomplete_event_stream_still_yields_the_whole_tree() {
    let mut live = Live::open(
        &[("a.txt", &text(10, &[])), ("b.txt", &text(10, &[]))],
        &[("a.txt", &text(10, &[1]))],
    )
    .await;
    live.session
        .toggle_hunk(0, vet_app::session::View::Changeset, "a.txt", 0)
        .await
        .unwrap();

    write(live.path(), "a.txt", &text(10, &[1, 6]));
    write(live.path(), "b.txt", &text(10, &[2]));
    live.send(WatchEvent::Dirty(vec![vet_core::content::RepoPath(b"a.txt".to_vec())]));
    assert_eq!(live.revision().await, 1);

    live.assert_correct().await;
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Seen, CoreHunk::Unseen]);
    assert_eq!(live.hunks("b.txt").await, [CoreHunk::Unseen]);
}

#[tokio::test]
async fn criterion4_a_killed_and_restarted_watcher_catches_up_in_one_rescan() {
    let mut live = Live::open(&[("a.txt", &text(40, &[]))], &[("a.txt", &text(40, &[5, 30]))]).await;
    live.session
        .toggle_hunk(0, vet_app::session::View::Changeset, "a.txt", 0)
        .await
        .unwrap();
    live.session
        .toggle_hunk(0, vet_app::session::View::Changeset, "a.txt", 1)
        .await
        .unwrap();

    live.kill_watcher().await;
    write(live.path(), "a.txt", &text(40, &[5]));
    write(live.path(), "while-dead.txt", "x\n");
    // `kill_watcher` joined the follower, so the watcher is over and cannot have settled the
    // writes: no wait is needed to be sure of it.
    assert_eq!(live.session.revision().await, 0);

    // A new watcher starts; a restart reconciles before it trusts any event.
    live.restart_watcher().await;
    live.send(WatchEvent::MustRescan);
    assert_eq!(live.revision().await, 1);
    live.assert_correct().await;
    // Line 30 reverted to base: that hunk is gone; line 5 is still as it was marked.
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Seen]);
    assert_eq!(
        live.files().await,
        [
            ("a.txt".into(), AppFileStatus::Modified),
            ("while-dead.txt".into(), AppFileStatus::Added)
        ]
    );

    write(live.path(), "a.txt", &text(40, &[5, 20]));
    live.send(WatchEvent::Dirty(vec![vet_core::content::RepoPath(b"a.txt".to_vec())]));
    assert_eq!(live.revision().await, 2);
    live.assert_correct().await;
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Seen, CoreHunk::Unseen]);
}

#[tokio::test]
async fn criterion4_an_overflow_in_the_middle_of_a_write_burst_settles_on_the_final_tree() {
    let mut live = Live::open(&[("a.txt", &text(40, &[]))], &[("a.txt", &text(40, &[5]))]).await;
    live.session
        .toggle_hunk(0, vet_app::session::View::Changeset, "a.txt", 0)
        .await
        .unwrap();

    // An edit stream with an overflow in the middle and a dropped tail.
    let path = vet_core::content::RepoPath(b"a.txt".to_vec());
    write(live.path(), "a.txt", &text(40, &[5, 10]));
    live.send(WatchEvent::Dirty(vec![path]));
    tokio::time::sleep(Duration::from_millis(30)).await;
    write(live.path(), "a.txt", &text(40, &[5, 10, 20]));
    live.send(WatchEvent::MustRescan);
    tokio::time::sleep(Duration::from_millis(30)).await;
    write(live.path(), "a.txt", &text(40, &[5, 10, 20, 30]));

    let last = live.settle_all().await.expect("something settled");
    assert!(last >= 1);
    live.assert_correct().await;
    let text_now = live
        .session
        .file_text(vet_app::session::View::Changeset, "a.txt")
        .await
        .unwrap();
    assert_eq!(text_now.target.as_deref(), Some(text(40, &[5, 10, 20, 30]).as_str()));
    assert_eq!(
        live.hunks("a.txt").await,
        [CoreHunk::Seen, CoreHunk::Unseen, CoreHunk::Unseen, CoreHunk::Unseen]
    );
}

#[tokio::test]
async fn criterion4_a_watch_limit_fallback_rescans_and_is_reported() {
    let mut live = Live::open(&[("a.txt", &text(10, &[]))], &[("a.txt", &text(10, &[1]))]).await;
    live.session
        .toggle_hunk(0, vet_app::session::View::Changeset, "a.txt", 0)
        .await
        .unwrap();
    assert_eq!(
        live.session
            .manifest(vet_app::session::View::Changeset)
            .await
            .unwrap()
            .degraded,
        None
    );

    write(live.path(), "a.txt", &text(10, &[1, 7]));
    live.send(WatchEvent::Degraded(Degraded::WatchLimit));
    assert_eq!(live.revision().await, 1);

    live.assert_correct().await;
    let m = live.session.manifest(vet_app::session::View::Changeset).await.unwrap();
    assert_eq!(m.degraded, Some(Degraded::WatchLimit));
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Seen, CoreHunk::Unseen]);
}

#[tokio::test]
async fn criterion4_a_git_state_change_without_worktree_events_is_reconciled() {
    // `git commit` moves HEAD and the index but touches no worktree file; the
    // snapshot must stay the same and the review state must not move.
    let live = Live::open(&[("a.txt", &text(10, &[]))], &[("a.txt", &text(10, &[1]))]).await;
    live.session
        .toggle_hunk(0, vet_app::session::View::Changeset, "a.txt", 0)
        .await
        .unwrap();
    git(live.path(), &["commit", "-q", "-a", "-m", "agent commits"]);

    live.send(WatchEvent::MustRescan);
    live.quiet().await;
    // Same tree, same snapshot: the commit moved HEAD, not the files.
    assert_eq!(live.session.revision().await, 0);
    live.assert_correct().await;
    assert_eq!(live.hunks("a.txt").await, [CoreHunk::Seen]);
}

#[tokio::test]
async fn criterion4_a_write_storm_through_the_real_watcher_settles_on_the_final_tree() {
    let fx = fixture(
        &[("a.txt", &text(40, &[])), ("old/x.txt", "x\n")],
        &[("a.txt", &text(40, &[5, 30]))],
    )
    .await;
    let p = fx.repo.path();
    let mut client = Client::new(&fx.handler);
    let project = client.open(p).await;
    client.toggle_hunk(project, "a.txt", 0).await;

    // Hundreds of files across many folders in one burst, with the marked line
    // rewritten and the other mark's hunk left alone.
    let mut expected = vec![("a.txt".to_string(), vet_protocol::FileStatus::Modified)];
    for dir in 0..20 {
        for i in 0..25 {
            let name = format!("storm/d{dir}/f{i}.txt");
            write(p, &name, &format!("{dir} {i}\n"));
            if i % 5 == 0 {
                std::fs::remove_file(p.join(&name)).unwrap();
            } else {
                expected.push((name, vet_protocol::FileStatus::Added));
            }
        }
    }
    std::fs::remove_dir_all(p.join("old")).unwrap();
    expected.push(("old/x.txt".into(), vet_protocol::FileStatus::Deleted));
    write(p, "a.txt", &text(40, &[30]).replace("line 5\n", "agent 5\n"));

    // The kernel may deliver the burst in pieces, so a snapshot can settle
    // part-way; the tree must end up at the final state, and stay there.
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let m = loop {
        client.quiet(project).await;
        let m = client.manifest(project).await;
        let mut got: Vec<_> = m.files.iter().map(|f| (f.path.clone(), f.status)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        if got == expected || tokio::time::Instant::now() > deadline {
            assert_eq!(got, expected);
            break m;
        }
    };
    assert_eq!(hunks(&m, "a.txt"), [HunkState::Unseen, HunkState::Unseen]);
}
