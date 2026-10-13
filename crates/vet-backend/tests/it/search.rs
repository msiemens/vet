//! Text search through the protocol, against a real repository.

use crate::common::{Client, handler, repo_with};
use vet_protocol::{
    FileStatus, HunkState, ProjectId, Request, Response, SearchFile, SearchLineKind, SearchPattern, SearchScope,
    SearchSide, View,
};

struct Found {
    files: Vec<SearchFile>,
    hits: u32,
    truncated: bool,
}

fn pattern(text: &str, regex: bool) -> SearchPattern {
    SearchPattern {
        text: text.into(),
        match_case: false,
        whole_word: false,
        regex,
    }
}

struct Ask {
    scope: SearchScope,
    pattern: SearchPattern,
    mask: &'static str,
    only_changes: bool,
    limit: u32,
}

impl Ask {
    fn new(scope: SearchScope, text: &str) -> Self {
        Self {
            scope,
            pattern: pattern(text, false),
            mask: "",
            only_changes: false,
            limit: 1000,
        }
    }
}

async fn search(client: &Client, project: ProjectId, ask: Ask) -> Result<Found, String> {
    let response = client
        .call(Request::Search {
            project,
            scope: ask.scope,
            pattern: ask.pattern,
            mask: ask.mask.into(),
            side: SearchSide::Both,
            only_changes: ask.only_changes,
            not_seen: false,
            limit: ask.limit,
        })
        .await?;
    match response {
        Response::SearchResults {
            files, hits, truncated, ..
        } => Ok(Found { files, hits, truncated }),
        other => panic!("{other:?}"),
    }
}

fn paths(found: &Found) -> Vec<&str> {
    found.files.iter().map(|f| f.path.as_str()).collect()
}

const CHANGES: SearchScope = SearchScope::View { view: View::Changeset };

async fn open() -> (Client, ProjectId, tempfile::TempDir, tempfile::TempDir) {
    let repo = repo_with(
        &[
            ("src/a.rs", "let delay = 1;\nlet old = 2;\n"),
            ("src/same.rs", "// delay here\n"),
            ("notes.md", "nothing\n"),
        ],
        &[
            ("src/a.rs", "let delay = 1;\nlet Delay2 = 2;\n"),
            ("target/out.rs", "delay\n"),
            ("src/new.rs", "delay\n"),
        ],
    );
    let (handler, data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    (client, project, repo, data)
}

#[tokio::test]
async fn a_changeset_search_finds_changed_lines_on_either_side_and_leaves_unchanged_files_out() {
    let (client, project, _repo, _data) = open().await;
    let found = search(&client, project, Ask::new(CHANGES, "old")).await.unwrap();
    assert_eq!(paths(&found), ["src/a.rs"]);
    assert_eq!(found.files[0].lines[0].kind, SearchLineKind::Removed);

    let found = search(&client, project, Ask::new(CHANGES, "delay")).await.unwrap();
    assert_eq!(paths(&found), ["src/a.rs", "src/new.rs"]);
    // The context line, the added one and the new file's line.
    assert_eq!(found.hits, 3);
    assert_eq!(found.files[1].status, Some(FileStatus::Added));
    // A preview draws the diff from the hunks and shows their review state.
    assert_eq!(
        (found.files[0].hunks.len(), &found.files[0].states[..]),
        (1, &[HunkState::Unseen][..])
    );
}

#[tokio::test]
async fn the_repository_search_adds_unchanged_files_but_not_ignored_ones() {
    let (client, project, _repo, _data) = open().await;
    let found = search(&client, project, Ask::new(SearchScope::Repository, "delay"))
        .await
        .unwrap();
    assert_eq!(paths(&found), ["src/a.rs", "src/new.rs", "src/same.rs"]);
    assert_eq!(found.files[2].status, None);

    let ask = Ask {
        only_changes: true,
        ..Ask::new(SearchScope::Repository, "delay")
    };
    assert_eq!(
        paths(&search(&client, project, ask).await.unwrap()),
        ["src/a.rs", "src/new.rs"]
    );
}

#[tokio::test]
async fn the_mask_and_the_limit_narrow_the_search() {
    let (client, project, _repo, _data) = open().await;
    let ask = Ask {
        mask: "src/*.rs, !**/new.rs",
        ..Ask::new(SearchScope::Repository, "delay")
    };
    assert_eq!(
        paths(&search(&client, project, ask).await.unwrap()),
        ["src/a.rs", "src/same.rs"]
    );

    let ask = Ask {
        limit: 1,
        ..Ask::new(SearchScope::Repository, "delay")
    };
    let found = search(&client, project, ask).await.unwrap();
    assert!(found.truncated);
    assert_eq!(found.hits, 1);
}

#[tokio::test]
async fn a_regex_is_rust_syntax_and_an_invalid_one_or_a_bad_mask_fails_with_a_message() {
    let (client, project, _repo, _data) = open().await;
    let ask = Ask {
        pattern: pattern(r"Delay\d", true),
        ..Ask::new(CHANGES, "")
    };
    let found = search(&client, project, ask).await.unwrap();
    assert_eq!(found.files[0].lines[0].hits[0].from, 4);

    let ask = Ask {
        pattern: pattern("a(", true),
        ..Ask::new(CHANGES, "")
    };
    let error = search(&client, project, ask).await.err().unwrap();
    assert!(error.contains("unclosed group") && !error.contains('\n'), "{error}");
    let ask = Ask {
        mask: "src/[a",
        ..Ask::new(CHANGES, "delay")
    };
    assert!(search(&client, project, ask).await.err().unwrap().contains("src/[a"));
}
