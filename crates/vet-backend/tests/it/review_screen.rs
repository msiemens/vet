//! Language servers run for the review screen: a changeset that comes on view while the window
//! shows the queue starts none, and the review screen starts them. The server is a script that
//! writes its process id as a line when it is launched and never answers.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use vet_backend::Config;
use vet_protocol::{ChangesetKey, ProjectId, Request};

use crate::common::{Client, config, git, handler_with, repo_with, write};
#[cfg(debug_assertions)]
use crate::common::{background_live, background_started};

const SERVER: &str = "#!/bin/sh\n\
    if [ \"$1\" = \"--version\" ]; then echo fake-server 1.0; exit 0; fi\n\
    echo $$ >> \"@LAUNCHES@\"\n\
    cat > /dev/null\n";

pub(crate) struct Rig {
    pub(crate) client: Client,
    pub(crate) project: ProjectId,
    launches: PathBuf,
    pub(crate) repo: tempfile::TempDir,
    _tools: tempfile::TempDir,
    _data: tempfile::TempDir,
}

impl Rig {
    pub(crate) async fn start() -> Self {
        Self::start_with(config()).await
    }

    pub(crate) async fn start_with(config: Config) -> Self {
        let repo = repo_with(
            &[
                (
                    "Cargo.toml",
                    "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                ),
                ("src/lib.rs", "pub fn f() -> u32 {\n    1\n}\n"),
            ],
            &[],
        );
        let (handler, data) = handler_with(config).await;
        let client = Client::new(&handler);
        let project = client.open(repo.path()).await;

        let tools = tempfile::tempdir().unwrap();
        let launches = tools.path().join("launches");
        let script = tools.path().join("fake-server");
        std::fs::write(&script, SERVER.replace("@LAUNCHES@", launches.to_str().unwrap())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        client
            .call(Request::SetServerPath {
                project,
                language: "rust".into(),
                path: Some(script.to_str().unwrap().into()),
            })
            .await
            .unwrap();
        Self {
            client,
            project,
            launches,
            repo,
            _tools: tools,
            _data: data,
        }
    }

    /// A branch that changes `src/lib.rs` to return `value`.
    pub(crate) fn branch(&self, name: &str, value: u32) {
        let dir: &Path = self.repo.path();
        git(dir, &["checkout", "-q", "-b", name, "main"]);
        write(dir, "src/lib.rs", &format!("pub fn f() -> u32 {{\n    {value}\n}}\n"));
        git(dir, &["commit", "-q", "-am", name]);
        git(dir, &["checkout", "-q", "main"]);
    }

    /// Puts the branch on view, as a row of the queue does.
    pub(crate) async fn open(&self, branch: &str) {
        self.client
            .call(Request::OpenChangeset {
                project: self.project,
                changeset: ChangesetKey::Branch { name: branch.into() },
            })
            .await
            .unwrap();
    }

    pub(crate) async fn show_review(&self, shown: bool) {
        self.client
            .call(Request::SetReviewShown {
                project: self.project,
                shown,
            })
            .await
            .unwrap();
    }

    /// The process ids of the servers launched so far, in order.
    fn pids(&self) -> Vec<u32> {
        std::fs::read_to_string(&self.launches).map_or_else(
            |_| Vec::new(),
            |text| text.lines().map(|pid| pid.parse().unwrap()).collect(),
        )
    }

    fn launched(&self) -> usize {
        self.pids().len()
    }

    async fn trust(&self, trusted: bool) {
        self.client
            .call(Request::SetProjectTrusted {
                project: self.project,
                trusted,
            })
            .await
            .unwrap();
    }

    /// Waits until more servers were launched than `before`.
    pub(crate) async fn launched_beyond(&self, before: usize) {
        for _ in 0..600 {
            if self.launched() > before {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no server was launched beyond the {before} that were");
    }

    /// How many times the backend asked for the changeset on view's servers to start: the decision
    /// itself, where `launched` waits for the process.
    fn servers_started(&self) -> usize {
        background_started(&self.client, self.project, "servers")
    }
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_changeset_put_on_view_from_the_queue_starts_no_server_and_the_review_screen_starts_it() {
    let rig = Rig::start().await;
    rig.branch("a", 2);
    rig.branch("b", 3);

    rig.open("a").await;
    rig.open("b").await;
    // Opening from the queue asks for no server: the review screen is what does.
    assert_eq!(rig.servers_started(), 0, "the queue started a server");

    rig.show_review(true).await;
    assert!(rig.servers_started() > 0, "the review screen starts them");
    rig.launched_beyond(0).await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn servers_follow_the_changeset_on_view_while_the_review_screen_is_shown_and_not_after_it_is_left() {
    let rig = Rig::start().await;
    rig.branch("a", 2);
    rig.branch("b", 3);
    rig.branch("c", 4);

    rig.show_review(true).await;
    rig.open("a").await;
    assert!(rig.servers_started() > 0, "the changeset on view starts servers");
    rig.launched_beyond(0).await;

    let before = rig.servers_started();
    rig.open("b").await;
    assert!(
        rig.servers_started() > before,
        "a changeset opened while the review screen is shown starts servers"
    );

    rig.show_review(false).await;
    let before = rig.servers_started();
    rig.open("c").await;
    assert_eq!(
        rig.servers_started(),
        before,
        "a server started for a changeset opened from the queue"
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn withdrawn_trust_retires_the_executing_servers_and_analyses_again() {
    let rig = Rig::start().await;
    rig.branch("a", 2);
    rig.open("a").await;
    rig.show_review(true).await;
    rig.launched_beyond(0).await;
    let limited = rig.launched();

    // The servers of the trusted project execute project code.
    rig.trust(true).await;
    rig.launched_beyond(limited).await;
    let executing = rig.pids().split_off(limited);
    let started = |kind| background_started(&rig.client, rig.project, kind);
    let (retired, analyses) = (started("retire"), started("symbols"));

    rig.trust(false).await;
    // The retire and the analysis are decided before the reply goes out.
    assert!(started("retire") > retired, "the executing servers were not retired");
    assert!(started("symbols") > analyses, "the analysis did not start again");
    for pid in executing {
        assert!(crate::restart::gone(pid).await, "the executing server {pid} still runs");
    }
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn two_server_starts_never_run_at_once() {
    // A clean working tree has no file to analyse, so only the starts launch servers.
    let rig = Rig::start().await;
    rig.show_review(true).await;
    // The server never answers, so its start goes on.
    rig.launched_beyond(0).await;
    background_live(&rig.client, rig.project, "servers", 1).await;

    // A new policy asks for servers of its own, which wait for the start that goes on instead of
    // launching next to it.
    rig.trust(true).await;
    rig.trust(false).await;
    rig.trust(true).await;
    assert_eq!(rig.servers_started(), 4);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.launched(), 1, "a second start ran while the first went on");
    background_live(&rig.client, rig.project, "servers", 4).await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn a_closed_project_ends_its_server_starts_and_starts_no_more() {
    let rig = Rig::start().await;
    rig.branch("a", 2);
    rig.open("a").await;
    rig.show_review(true).await;
    rig.launched_beyond(0).await;
    rig.trust(true).await;
    let launched = rig.launched();

    rig.client
        .call(Request::CloseProject { project: rig.project })
        .await
        .unwrap();

    background_live(&rig.client, rig.project, "servers", 0).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        rig.launched(),
        launched,
        "a server was launched after the project closed"
    );
}
