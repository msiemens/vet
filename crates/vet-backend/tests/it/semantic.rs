//! Semantic and language server requests through the handler. The Rust test
//! needs a real rust-analyzer: it is ignored and run by `ci/tool-tests.sh`.

#![allow(clippy::disallowed_methods)]

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{Client, config, find, git, repo_with, write};
use vet_backend::{Config, Orchestrator, open_store};
use vet_protocol::{
    CallFate, ChangesetKey, Event, LocationChange, ProjectId, Reach, Request, Response, ServerMessage, ServerState,
    Serving, Side, SymbolChange, SymbolId, SymbolKind, SymbolName, UnavailableKind, View,
};

const BASE: &str = "\
pub fn gone() -> u32 {
    1
}

pub fn keep() -> u32 {
    2
}

pub fn caller() -> u32 {
    gone()
}

pub fn main() -> u32 {
    keep()
}

pub fn trouble() -> u32 {
    let y: u32 = \"y\";
    y
}
";

const DIRTY: &str = "\
pub fn keep() -> u32 {
    3
}

pub fn fresh() -> u32 {
    keep()
}

pub fn broken() -> u32 {
    let x: u32 = \"x\";
    x
}

pub fn caller() -> u32 {
    gone()
}

pub fn main() -> u32 {
    fresh()
}

pub fn trouble() -> u32 {
    let y: u32 = \"y\";
    y
}
";

fn name(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn rust_semantics_through_the_protocol() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    write(repo.path(), ".gitignore", "target/\n");
    write(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(repo.path(), "src/lib.rs", BASE);
    write(repo.path(), "src/other.rs", "pub fn steady() {}\n");
    let lock = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    write(repo.path(), "src/lib.rs", DIRTY);

    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    let revision = client.revision(project).await;
    let path = "src/lib.rs";

    let (line, col) = find(DIRTY, "keep()", 1, 1);
    let hover = client
        .call(Request::Hover {
            project,
            revision,
            side: Side::Target,
            path: path.into(),
            line,
            col,
        })
        .await
        .unwrap();
    let Response::Hover {
        revision: answered,
        side,
        text,
        code_highlights,
        unavailable,
    } = hover
    else {
        panic!("{hover:?}");
    };
    assert_eq!((answered, side, unavailable), (revision, Side::Target, None));
    assert!(text.unwrap().contains("fn keep"));
    // The signature is the last of the fenced Rust blocks: `pub fn keep`.
    let signature = code_highlights.last().unwrap();
    assert!(
        signature.starts_with(&[0, 3, 0, 1, 2, 0, 1, 4, 5]),
        "{code_highlights:?}"
    );

    let Response::Definition(defs) = client
        .call(Request::Definition {
            project,
            revision,
            side: Side::Target,
            path: path.into(),
            line,
            col,
        })
        .await
        .unwrap()
    else {
        panic!("not a definition");
    };
    assert_eq!((defs.revision, defs.side), (revision, Side::Target));
    assert_eq!(defs.locations.len(), 1);
    assert_eq!(defs.locations[0].path, path);
    assert_eq!(defs.locations[0].span.start.line, 0);
    assert!(!defs.locations[0].external);

    let (line, col) = find(DIRTY, "keep", 0, 1);
    let Response::References(refs) = client
        .call(Request::References {
            project,
            revision,
            side: Side::Target,
            path: path.into(),
            line,
            col,
            limit: None,
        })
        .await
        .unwrap()
    else {
        panic!("not references");
    };
    assert_eq!(refs.rows.len(), 1, "{refs:?}");
    assert_eq!(refs.rows[0].line, find(DIRTY, "keep()", 1, 0).0 + 1);
    let row = &refs.rows[0];
    assert_eq!(row.text, "keep()");
    assert_eq!(&row.text[row.match_start as usize..row.match_end as usize], "keep");
    // The call's name is a function, coloured as the line is in the file (here indented).
    assert!(row.highlights.starts_with(&[0, 4, 5]), "{:?}", row.highlights);
    assert_eq!(
        row.within,
        Some(SymbolName {
            id: SymbolId {
                path: path.into(),
                name: name(&["fresh"])
            },
            modules: 0,
            kind: SymbolKind::Function
        })
    );
    assert_eq!(row.change, LocationChange::Hunk, "{refs:?}");
    assert_eq!(row.hunk, Some(0), "{refs:?}");
    assert!(!row.external);
    let cut = client
        .references_up_to(project, revision, Side::Target, path, (line, col), Some(0))
        .await;
    assert_eq!(
        (cut.rows.len(), cut.total),
        (0, 1),
        "a limit cuts the rows and keeps the total"
    );

    // A server that ran says what it offers; one that has not started does not.
    let Response::Servers { servers, .. } = client.call(Request::ListServers { project }).await.unwrap() else {
        panic!("not servers");
    };
    let offers = |language: &str| {
        let server = servers.iter().find(|s| s.language == language).unwrap();
        server.features.map(|f| f.implementations)
    };
    assert_eq!(offers("rust"), Some(true));
    assert_eq!(offers("python"), None);

    // The base side is a different tree: `gone` only exists there.
    let (line, col) = find(BASE, "gone()", 1, 1);
    let Response::Hover { text, side, .. } = client
        .call(Request::Hover {
            project,
            revision,
            side: Side::Base,
            path: path.into(),
            line,
            col,
        })
        .await
        .unwrap()
    else {
        panic!("not a hover");
    };
    assert_eq!(side, Side::Base);
    assert!(text.unwrap().contains("fn gone"));

    let Response::Diagnostics {
        diagnostics,
        unavailable,
        ..
    } = client
        .call(Request::Diagnostics {
            project,
            revision,
            path: path.into(),
            against_base: true,
        })
        .await
        .unwrap()
    else {
        panic!("not diagnostics");
    };
    assert_eq!(unavailable, None);
    // `broken` is the change's; `trouble` is the same error in the base, moved down with the diff.
    let origin_at = |line: u32| {
        let at = diagnostics
            .iter()
            .filter(|d| d.message.contains("expected u32") && d.span.start.line == line)
            .collect::<Vec<_>>();
        assert_eq!(at.len(), 1, "{diagnostics:?}");
        at[0].origin
    };
    assert_eq!(
        origin_at(find(DIRTY, "let x: u32", 0, 0).0),
        vet_protocol::DiagnosticOrigin::New
    );
    assert_eq!(
        origin_at(find(DIRTY, "let y: u32", 0, 0).0),
        vet_protocol::DiagnosticOrigin::Existing
    );

    let Response::FileSymbols(file) = client
        .call(Request::FileSymbols {
            project,
            revision,
            path: path.into(),
        })
        .await
        .unwrap()
    else {
        panic!("not file symbols");
    };
    assert_eq!(file.revision, revision);
    let change_of = |symbol: &str| {
        file.symbols
            .iter()
            .find(|c| c.symbol.id.name == name(&[symbol]))
            .unwrap_or_else(|| panic!("{symbol} in {:?}", file.symbols))
            .change
    };
    assert_eq!(change_of("gone"), SymbolChange::Removed);
    assert_eq!(change_of("keep"), SymbolChange::Modified);
    assert_eq!(change_of("fresh"), SymbolChange::Added);
    assert_eq!(change_of("main"), SymbolChange::Modified);
    // Every symbol is at the top level, so none has a holder.
    assert!(file.context.is_empty() && file.gap.is_none(), "{file:?}");
    assert_eq!(file.inputs.analysis, "changed symbols");
    assert!(
        file.inputs
            .primitives
            .iter()
            .any(|p| p.starts_with("document_symbols(base:"))
    );
    assert!(!file.inputs.hunks.is_empty());
    assert!(
        client
            .call(Request::FileSymbols {
                project,
                revision,
                path: "Cargo.toml".into()
            })
            .await
            .is_err(),
        "not a changed file"
    );

    // The whole outline: the unchanged symbols too, and the removed one where it stood.
    let outline_of = |view, path: &str| {
        let client = &client;
        let request = Request::FileOutline {
            project,
            view,
            revision,
            path: path.into(),
        };
        async move {
            let Response::FileOutline(outline) = client.call(request).await.unwrap() else {
                panic!("not an outline");
            };
            outline
        }
    };
    let outline = outline_of(View::Changeset, path).await;
    assert_eq!(outline.unavailable, None);
    assert_eq!(
        (outline.symbols.iter())
            .map(|o| (o.symbol.id.name[0].as_str(), o.change))
            .collect::<Vec<_>>(),
        [
            ("gone", Some(SymbolChange::Removed)),
            ("keep", Some(SymbolChange::Modified)),
            ("fresh", Some(SymbolChange::Added)),
            ("broken", Some(SymbolChange::Added)),
            ("caller", None),
            ("main", Some(SymbolChange::Modified)),
            ("trouble", None),
        ]
    );
    // A file the changeset leaves alone has an outline too.
    let other = outline_of(View::Changeset, "src/other.rs").await;
    assert_eq!(
        (other.symbols.iter())
            .map(|o| (o.symbol.id.name[0].as_str(), o.change))
            .collect::<Vec<_>>(),
        [("steady", None)]
    );

    // The symbols of the project by name: the Rust server answers, and no other language is asked.
    let Response::WorkspaceSymbols(found) = client
        .call(Request::WorkspaceSymbols {
            project,
            revision,
            query: "fresh".into(),
        })
        .await
        .unwrap()
    else {
        panic!("not workspace symbols");
    };
    assert_eq!(found.revision, revision);
    let [rust] = &found.languages[..] else {
        panic!("{:?}", found.languages)
    };
    assert_eq!((rust.language.as_str(), rust.failure.is_none()), ("rust", true));
    assert!(
        rust.symbols
            .iter()
            .any(|s| s.id.path == path && s.id.name == name(&["fresh"])),
        "{:?}",
        rust.symbols
    );

    // The symbol under the cursor: changed or not, on either side.
    let symbol_at = |side, (line, col): (u32, u32)| {
        let client = &client;
        async move {
            let Response::SymbolAt(at) = client
                .call(Request::SymbolAt {
                    project,
                    revision,
                    side,
                    path: path.into(),
                    line,
                    col,
                })
                .await
                .unwrap()
            else {
                panic!("not a symbol");
            };
            (at.symbol.map(|s| s.id.name), at.change)
        }
    };
    assert_eq!(
        symbol_at(Side::Target, find(DIRTY, "    3", 0, 4)).await,
        (Some(name(&["keep"])), Some(SymbolChange::Modified))
    );
    assert_eq!(
        symbol_at(Side::Target, find(DIRTY, "    fresh()", 0, 4)).await,
        (Some(name(&["main"])), Some(SymbolChange::Modified))
    );
    assert_eq!(
        symbol_at(Side::Target, find(DIRTY, "pub fn caller", 0, 8)).await,
        (Some(name(&["caller"])), None)
    );
    // Between two declarations.
    let (fresh, _) = find(DIRTY, "pub fn fresh", 0, 0);
    assert_eq!(symbol_at(Side::Target, (fresh - 1, 0)).await, (None, None));
    assert_eq!(
        symbol_at(Side::Base, find(BASE, "    1", 0, 4)).await,
        (Some(name(&["gone"])), Some(SymbolChange::Removed))
    );

    let id = |symbol: &str| SymbolId {
        path: path.into(),
        name: name(&[symbol]),
    };
    // A removed symbol's signature comes from the base side.
    for (symbol, expected) in [
        ("keep", "fn keep() -> u32"),
        ("gone", "fn gone() -> u32"),
        ("caller", "fn caller() -> u32"),
    ] {
        let Response::Signature {
            signature, unavailable, ..
        } = client
            .call(Request::Signature {
                project,
                revision,
                symbol: id(symbol),
            })
            .await
            .unwrap()
        else {
            panic!("not a signature");
        };
        let signature = signature.unwrap_or_else(|| panic!("no signature for {symbol}: {unavailable:?}"));
        assert!(signature.code.contains(expected), "{signature:?}");
        assert!(!signature.highlights.is_empty(), "{signature:?}");
    }

    let calls = |symbol| {
        let client = &client;
        async move {
            let Response::Calls(calls) = client
                .call(Request::Calls {
                    project,
                    revision,
                    symbol,
                })
                .await
                .unwrap()
            else {
                panic!("not calls");
            };
            calls
        }
    };
    // `main` calls `fresh` and `fresh` calls `keep`: both changed, so the walk goes through them.
    let around_keep = calls(id("keep")).await;
    let callers: Vec<_> = (around_keep.callers.iter())
        .map(|c| (c.symbol.id.name.clone(), c.change, c.depth))
        .collect();
    assert_eq!(
        callers,
        [
            (name(&["fresh"]), SymbolChange::Added, 0),
            (name(&["main"]), SymbolChange::Modified, 1)
        ]
    );
    assert!(around_keep.callees.is_empty() && around_keep.affected.is_empty());
    let around_fresh = calls(id("fresh")).await;
    let callees: Vec<_> = (around_fresh.callees.iter())
        .map(|c| (c.symbol.id.name.clone(), c.change))
        .collect();
    assert_eq!(callees, [(name(&["keep"]), SymbolChange::Modified)]);
    assert!(around_fresh.gaps.is_empty(), "{around_fresh:?}");

    let Response::Usages(usages) = client
        .call(Request::Usages {
            project,
            revision,
            symbol: id("keep"),
        })
        .await
        .unwrap()
    else {
        panic!("not usages");
    };
    let within = usages.references[0].within.as_ref().map(|w| w.id.name.clone());
    assert_eq!((usages.references.len(), within), (1, Some(name(&["fresh"]))));
    assert_eq!(usages.gap, None);
    assert!(
        usages
            .inputs
            .primitives
            .iter()
            .any(|p| p.starts_with("references(target:"))
    );

    let Response::RemovalImpact(removal) = client
        .call(Request::RemovalImpact {
            project,
            revision,
            symbol: id("gone"),
        })
        .await
        .unwrap()
    else {
        panic!("not a removal impact");
    };
    assert_eq!(removal.gap, None);
    assert_eq!(removal.call_sites.len(), 1, "{removal:?}");
    assert_eq!(removal.call_sites[0].fate, CallFate::LikelyBroken);
    assert_eq!(
        removal.call_sites[0].base.location.span.start.line,
        find(BASE, "gone()", 1, 0).0
    );
    let within = removal.call_sites[0].base.within.as_ref().map(|w| w.id.name.clone());
    assert_eq!(within, Some(name(&["caller"])), "{removal:?}");
    assert!(
        removal
            .inputs
            .primitives
            .iter()
            .any(|p| p.starts_with("references(base:"))
    );

    let Response::Reachability(reach) = client
        .call(Request::Reachability {
            project,
            revision,
            symbol: id("fresh"),
        })
        .await
        .unwrap()
    else {
        panic!("not a reachability");
    };
    let Reach::Reached { path: via } = &reach.reach else {
        panic!("{reach:?}");
    };
    assert_eq!(
        via.iter().map(|s| s.id.name[0].as_str()).collect::<Vec<_>>(),
        ["fresh", "main"]
    );
    assert_eq!(reach.inputs.bounds.map(|b| b.max_depth), Some(8));
    assert!(reach.inputs.config.iter().any(|c| c.contains("main")));

    // Kinds are not interchangeable, and stale revisions are refused.
    let wrong = client
        .call(Request::RemovalImpact {
            project,
            revision,
            symbol: id("keep"),
        })
        .await;
    assert!(wrong.is_err());
    let unknown = client
        .call(Request::Reachability {
            project,
            revision,
            symbol: id("nothing"),
        })
        .await;
    let unknown = unknown.unwrap_err();
    assert!(unknown.message.contains("no such symbol"));
    assert_eq!(unknown.kind, vet_protocol::ErrorKind::NotFound);
    let removed = client
        .call(Request::Calls {
            project,
            revision,
            symbol: id("gone"),
        })
        .await;
    assert!(removed.unwrap_err().message.contains("removed"));
    for request in [
        Request::Hover {
            project,
            revision: revision + 1,
            side: Side::Target,
            path: path.into(),
            line: 0,
            col: 0,
        },
        Request::FileSymbols {
            project,
            revision: revision + 1,
            path: path.into(),
        },
        Request::FileOutline {
            project,
            view: View::Changeset,
            revision: revision + 1,
            path: path.into(),
        },
        Request::Diagnostics {
            project,
            revision: revision + 1,
            path: path.into(),
            against_base: true,
        },
    ] {
        let error = client.call(request).await.unwrap_err();
        assert_eq!(error.kind, vet_protocol::ErrorKind::Outdated, "{error}");
    }
    client.call(Request::CloseProject { project }).await.unwrap();
}

#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn the_outline_at_a_commit_is_that_commits_text_without_marks() {
    let repo = tempfile::tempdir().unwrap();
    let p = repo.path();
    git(p, &["init", "-q", "-b", "main"]);
    write(p, ".gitignore", "target/\n");
    write(
        p,
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(p, "src/lib.rs", "pub fn old() {}\n");
    let lock = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(p)
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "base"]);
    git(p, &["checkout", "-q", "-b", "feature"]);
    write(p, "src/lib.rs", "pub fn old() {}\n\npub fn in_commit() {}\n");
    git(p, &["commit", "-q", "-am", "add in_commit"]);
    write(
        p,
        "src/lib.rs",
        "pub fn old() {}\n\npub fn in_commit() {}\n\npub fn later() {}\n",
    );
    git(p, &["commit", "-q", "-am", "add later"]);

    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Client::new(&handler);
    let project = client.open(p).await;
    client
        .call(Request::OpenChangeset {
            project,
            changeset: ChangesetKey::Branch { name: "feature".into() },
        })
        .await
        .unwrap();
    let revision = client.revision(project).await;
    let Response::Commits { commits, .. } = client.call(Request::Commits { project }).await.unwrap() else {
        panic!("not commits");
    };
    assert_eq!(commits.len(), 2, "{commits:?}");

    let outline = |view, path: &str| {
        let request = Request::FileOutline {
            project,
            view,
            revision,
            path: path.into(),
        };
        let client = &client;
        async move {
            let Response::FileOutline(outline) = client.call(request).await.unwrap() else {
                panic!("not an outline");
            };
            outline
        }
    };
    let names = |outline: &vet_protocol::FileOutline| {
        (outline.symbols.iter())
            .map(|o| (o.symbol.id.name[0].clone(), o.change))
            .collect::<Vec<_>>()
    };

    let at_commit = outline(View::Commit { index: 0 }, "src/lib.rs").await;
    assert_eq!(at_commit.unavailable, None);
    assert_eq!(
        names(&at_commit),
        [("old".to_owned(), None), ("in_commit".to_owned(), None)]
    );

    let whole = outline(View::Changeset, "src/lib.rs").await;
    assert_eq!(
        names(&whole),
        [
            ("old".to_owned(), None),
            ("in_commit".to_owned(), Some(SymbolChange::Added)),
            ("later".to_owned(), Some(SymbolChange::Added)),
        ]
    );

    // Not in the commit: no symbols, and nothing is wrong with the file's language.
    let absent = outline(View::Commit { index: 0 }, "src/missing.rs").await;
    assert!(absent.symbols.is_empty() && absent.unavailable.is_none(), "{absent:?}");
}

#[tokio::test]
async fn a_language_without_a_server_degrades_and_never_fails_the_project() {
    let repo = repo_with(
        &[("a.py", "def f():\n    return 1\n"), ("notes.md", "x\n")],
        &[("a.py", "def f():\n    return 2\n"), ("notes.md", "y\n")],
    );
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    let revision = client.revision(project).await;

    // A path that is not a file: the custom path wins over everything, so the server is missing.
    client
        .call(Request::SetServerPath {
            project,
            language: "python".into(),
            path: Some("/no/such/pyright-langserver".into()),
        })
        .await
        .unwrap();
    let hover = |path: &str| Request::Hover {
        project,
        revision,
        side: Side::Target,
        path: path.into(),
        line: 1,
        col: 4,
    };
    let Response::Hover { text, unavailable, .. } = client.call(hover("a.py")).await.unwrap() else {
        panic!("not a hover");
    };
    let unavailable = unavailable.expect("degraded");
    assert_eq!(text, None);
    assert_eq!(unavailable.kind, UnavailableKind::ServerMissing);
    assert_eq!(unavailable.label, "No language server");
    assert_eq!(unavailable.language.as_deref(), Some("python"));
    assert!(
        unavailable.message.contains("/no/such/pyright-langserver"),
        "{}",
        unavailable.message
    );

    let Response::Hover { unavailable, .. } = client.call(hover("notes.md")).await.unwrap() else {
        panic!("not a hover");
    };
    let unavailable = unavailable.unwrap();
    assert_eq!(
        (unavailable.kind, unavailable.label.as_str(), unavailable.language),
        (UnavailableKind::Unsupported, "No language support", None)
    );

    // Derived results report the failed file as a gap instead of failing; the changed
    // notes.md has no language and is no gap.
    let file_symbols = |path: &str| {
        let request = Request::FileSymbols {
            project,
            revision,
            path: path.into(),
        };
        let client = &client;
        async move {
            let Response::FileSymbols(file) = client.call(request).await.unwrap() else {
                panic!("not file symbols");
            };
            file
        }
    };
    let failed = file_symbols("a.py").await;
    assert!(failed.symbols.is_empty());
    assert!(
        failed
            .gap
            .as_ref()
            .is_some_and(|gap| gap.message.contains("/no/such/pyright-langserver")),
        "{failed:?}"
    );
    let notes = file_symbols("notes.md").await;
    assert!(notes.symbols.is_empty() && notes.gap.is_none(), "{notes:?}");

    // An outline says why there is none, in every view.
    for view in [View::Changeset, View::Commit { index: 0 }] {
        let outline = |path: &str| {
            let request = Request::FileOutline {
                project,
                view,
                revision,
                path: path.into(),
            };
            let client = &client;
            async move {
                let Response::FileOutline(outline) = client.call(request).await.unwrap() else {
                    panic!("not an outline");
                };
                outline
            }
        };
        let notes = outline("notes.md").await;
        assert!(notes.symbols.is_empty(), "{notes:?}");
        assert_eq!(notes.unavailable.unwrap().kind, UnavailableKind::Unsupported);
    }
    let python = Request::FileOutline {
        project,
        view: View::Changeset,
        revision,
        path: "a.py".into(),
    };
    let Response::FileOutline(python) = client.call(python).await.unwrap() else {
        panic!("not an outline");
    };
    assert!(python.symbols.is_empty());
    assert_eq!(python.unavailable.unwrap().kind, UnavailableKind::ServerMissing);

    // The project itself is fine.
    assert_eq!(client.revision(project).await, revision);
}

/// The languages that serve `paths`, as the backend names them.
async fn served_by(client: &Client, project: ProjectId, paths: &[&str]) -> Vec<Option<String>> {
    let paths = paths.iter().map(|p| (*p).to_owned()).collect();
    match client.call(Request::ServedBy { project, paths }).await.unwrap() {
        Response::ServedBy { serving } => serving
            .into_iter()
            .map(|s| match s {
                Serving::Language { language } => Some(language),
                Serving::Unsupported { why } => {
                    assert_eq!(why.kind, UnavailableKind::Unsupported);
                    None
                }
            })
            .collect(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_html_file_is_served_by_angular_only_in_a_project_that_has_an_angular_json() {
    let plain = repo_with(&[("page.html", "<p>x</p>\n"), ("src/a.rs", "fn a() {}\n")], &[]);
    let angular = repo_with(&[("angular.json", "{}\n"), ("src/app/page.html", "<p>x</p>\n")], &[]);
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Client::new(&handler);

    let project = client.open(plain.path()).await;
    client.revision(project).await;
    assert_eq!(
        served_by(&client, project, &["page.html", "src/a.rs", "notes.unknown"]).await,
        [None, Some("rust".to_owned()), None]
    );

    let project = client.open(angular.path()).await;
    client.revision(project).await;
    assert_eq!(
        served_by(&client, project, &["src/app/page.html"]).await,
        [Some("angular".to_owned())]
    );
}

#[tokio::test]
async fn servers_are_listed_and_custom_paths_persist_and_notify() {
    let repo = repo_with(&[("a.txt", "x\n")], &[]);
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let mut first = Client::new(&handler);
    let project = first.open(repo.path()).await;

    let listed = first.servers(project).await;
    let python = listed.iter().find(|s| s.language == "python").unwrap();
    assert_eq!(python.custom_path, None);
    assert!(
        listed
            .iter()
            .find(|s| s.language == "rust")
            .unwrap()
            .pinned_version
            .is_none()
    );

    // A second connection sets a path; the first is told, the setter is not.
    let second = Client::new(&handler);
    assert_eq!(second.open(repo.path()).await, project);
    let tool = data.path().join("my-pyright");
    std::fs::write(&tool, "").unwrap();
    let path = tool.to_str().unwrap().to_owned();
    let Response::Servers { servers, .. } = second
        .call(Request::SetServerPath {
            project,
            language: "python".into(),
            path: Some(path.clone()),
        })
        .await
        .unwrap()
    else {
        panic!("not servers");
    };
    let python = servers.iter().find(|s| s.language == "python").unwrap();
    assert_eq!(python.custom_path.as_deref(), Some(path.as_str()));
    let ServerState::Found { path: found, .. } = &python.state else {
        panic!("{python:?}");
    };
    assert_eq!(found, &path);
    let message = loop {
        let message = tokio::time::timeout(Duration::from_secs(5), first.inbox.recv())
            .await
            .unwrap()
            .unwrap();
        if !matches!(message, ServerMessage::Event { event: Event::Task(_) }) {
            break message;
        }
    };
    assert!(matches!(
        message,
        ServerMessage::Event {
            event: Event::ServersChanged { .. }
        }
    ));

    let unknown = second
        .call(Request::SetServerPath {
            project,
            language: "cobol".into(),
            path: None,
        })
        .await;
    let unknown = unknown.unwrap_err();
    assert!(unknown.message.contains("unknown language"));
    assert_eq!(unknown.kind, vet_protocol::ErrorKind::NotFound);

    // A new backend on the same data directory still has the path.
    drop((first, second));
    let again = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Client::new(&again);
    let project = client.open(repo.path()).await;
    let python = client
        .servers(project)
        .await
        .into_iter()
        .find(|s| s.language == "python")
        .unwrap();
    assert_eq!(python.custom_path.as_deref(), Some(path.as_str()));

    client
        .call(Request::SetServerPath {
            project,
            language: "python".into(),
            path: None,
        })
        .await
        .unwrap();
    let python = client
        .servers(project)
        .await
        .into_iter()
        .find(|s| s.language == "python")
        .unwrap();
    assert_eq!(python.custom_path, None);
}

/// Answers keep the positions of the revision on view (D97): its text is what the
/// worktree's server is fed, so a file whose lines have shifted on disk since the
/// snapshot settled is still asked about where the revision has it. The settles
/// that would follow the shift are held off for as long as the test runs, and the
/// tree is held quiet, so no newer revision can intervene and no other copy of
/// the content can answer for the query: what the revision read is what the
/// query keeps. The snapshot's text is an uncommitted edit, the case the worktree
/// usually is; the position the worktree has the file at answers differently,
/// which is how the query tells the revision's text from the disk's.
#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn rust_analyzer_is_asked_about_the_revision_even_after_the_worktree_lines_shifted() {
    const SNAPSHOT: &str = "\
pub fn gone() -> u32 {
    1
}

pub fn keep() -> u32 {
    2
}

pub fn main() -> u32 {
    keep()
}
";
    const SHIFTED: &str = "// moved\n// further\n// again\n";
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    write(repo.path(), ".gitignore", "target/\n");
    write(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(repo.path(), "src/lib.rs", SNAPSHOT);
    let lock = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    // The edit stays uncommitted: no git object holds its text (§6.1).
    git(repo.path(), &["add", ".gitignore", "Cargo.toml", "Cargo.lock"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);

    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(
        open_store(data.path()).await.unwrap(),
        Config {
            debounce: Duration::from_secs(60),
            quiet: Duration::from_secs(3600),
            ..config()
        },
    ));
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    let path = "src/lib.rs";
    let revision = client.revision(project).await;

    // The lines of the file shift on disk, and nothing settles a newer revision first.
    write(repo.path(), path, &(SHIFTED.to_owned() + SNAPSHOT));
    assert_eq!(client.revision(project).await, revision, "the revision did not move");

    // Hover is answered from the revision's text, not the disk's.
    let (at, col) = find(SNAPSHOT, "keep()", 1, 1);
    let (text, unavailable) = client.hover(project, revision, Side::Target, path, (at, col)).await;
    assert_eq!(unavailable, None);
    let text = text.expect("the hover of the call on the revision's text");
    assert!(text.contains("fn keep()"), "{text}");

    // Its definition is where the revision has it, not where the disk has moved it to.
    let defs = client
        .definition(project, revision, Side::Target, path, (at, col))
        .await;
    assert_eq!(defs.locations.len(), 1, "{defs:?}");
    assert_eq!(defs.locations[0].path, path);
    assert_eq!(defs.locations[0].span.start.line, 4);
    assert!(!defs.locations[0].external);
}
