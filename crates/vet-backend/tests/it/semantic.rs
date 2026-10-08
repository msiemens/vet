//! Semantic and language server requests through the handler. The Rust test
//! needs a real rust-analyzer and is skipped without one; run it alone
//! (`--test-threads=1`): it starts two servers.

#![allow(clippy::disallowed_methods)]

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::common::{Client, config, find, git, repo_with, write};
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{
    CallFate, Event, Reach, Request, Response, ServerMessage, ServerState, Side, SymbolChange, SymbolId,
    UnavailableKind,
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
";

fn name(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
async fn rust_semantics_through_the_protocol() {
    if Command::new("rust-analyzer").arg("--version").output().is_err() {
        eprintln!("skipped: rust-analyzer is not installed");
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    write(repo.path(), ".gitignore", "target/\n");
    write(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(repo.path(), "src/lib.rs", BASE);
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
        })
        .await
        .unwrap()
    else {
        panic!("not references");
    };
    assert_eq!(refs.locations.len(), 1, "{refs:?}");
    assert_eq!(refs.locations[0].span.start.line, find(DIRTY, "keep()", 1, 0).0);

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
        })
        .await
        .unwrap()
    else {
        panic!("not diagnostics");
    };
    assert_eq!(unavailable, None);
    assert!(
        diagnostics.iter().any(|d| d.message.contains("expected u32")),
        "{diagnostics:?}"
    );

    let Response::ChangedSymbols(changed) = client
        .call(Request::ChangedSymbols { project, revision })
        .await
        .unwrap()
    else {
        panic!("not changed symbols");
    };
    assert_eq!(changed.revision, revision);
    let change_of = |symbol: &str| {
        changed
            .symbols
            .iter()
            .find(|c| c.symbol.id.name == name(&[symbol]))
            .unwrap_or_else(|| panic!("{symbol} in {:?}", changed.symbols))
            .change
    };
    assert_eq!(change_of("gone"), SymbolChange::Removed);
    assert_eq!(change_of("keep"), SymbolChange::Modified);
    assert_eq!(change_of("fresh"), SymbolChange::Added);
    assert_eq!(change_of("main"), SymbolChange::Modified);
    assert!(changed.gaps.is_empty(), "{changed:?}");
    assert_eq!(changed.inputs.analysis, "changed symbols");
    assert!(
        changed
            .inputs
            .primitives
            .iter()
            .any(|p| p.starts_with("document_symbols(base:"))
    );
    assert!(!changed.inputs.hunks.is_empty());

    let id = |symbol: &str| SymbolId {
        path: path.into(),
        name: name(&[symbol]),
    };
    // A removed symbol's signature comes from the base side.
    for (symbol, expected) in [("keep", "fn keep() -> u32"), ("gone", "fn gone() -> u32")] {
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

    let Response::Impact(impact) = client
        .call(Request::Impact {
            project,
            revision,
            symbol: id("keep"),
        })
        .await
        .unwrap()
    else {
        panic!("not an impact");
    };
    assert_eq!(impact.revision, revision);
    assert_eq!(impact.references.len(), 1, "{impact:?}");
    let callers: Vec<_> = impact.callers.iter().map(|c| c.id.name.clone()).collect();
    assert_eq!(callers, [name(&["fresh"])]);
    assert!(impact.gaps.is_empty());
    assert!(
        impact
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
    assert_eq!(removal.call_sites[0].base.span.start.line, find(BASE, "gone()", 1, 0).0);
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
        via.iter().map(|s| s.name[0].as_str()).collect::<Vec<_>>(),
        ["fresh", "main"]
    );
    assert_eq!(reach.inputs.bounds.map(|b| b.max_depth), Some(8));
    assert!(reach.inputs.config.iter().any(|c| c.contains("main")));

    // Kinds are not interchangeable, and stale revisions are refused.
    let wrong = client
        .call(Request::Impact {
            project,
            revision,
            symbol: id("gone"),
        })
        .await;
    assert!(wrong.unwrap_err().contains("removed"));
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
    assert!(unknown.unwrap_err().contains("changed symbol"));
    for request in [
        Request::Hover {
            project,
            revision: revision + 1,
            side: Side::Target,
            path: path.into(),
            line: 0,
            col: 0,
        },
        Request::ChangedSymbols {
            project,
            revision: revision + 1,
        },
        Request::Diagnostics {
            project,
            revision: revision + 1,
            path: path.into(),
        },
    ] {
        let error = client.call(request).await.unwrap_err();
        assert!(error.contains("stale revision"), "{error}");
    }
    client.call(Request::CloseProject { project }).await.unwrap();
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
    let Response::ChangedSymbols(changed) = client
        .call(Request::ChangedSymbols { project, revision })
        .await
        .unwrap()
    else {
        panic!("not changed symbols");
    };
    assert!(changed.symbols.is_empty());
    assert_eq!(changed.gaps.len(), 1, "{changed:?}");
    assert_eq!(changed.gaps[0].path, "a.py");
    assert!(changed.gaps[0].reason.contains("/no/such/pyright-langserver"));

    // The project itself is fine.
    assert_eq!(client.revision(project).await, revision);
}

#[tokio::test]
async fn servers_are_listed_and_custom_paths_persist_and_notify() {
    let repo = repo_with(&[("a.txt", "x\n")], &[]);
    let data = tempfile::tempdir().unwrap();
    let handler = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let mut first = Client::new(&handler);
    let project = first.open(repo.path()).await;

    let listed = first.servers(project).await;
    let languages: Vec<_> = listed.iter().map(|s| s.language.as_str()).collect();
    assert_eq!(languages, ["rust", "typescript", "python", "csharp"]);
    let python = listed.iter().find(|s| s.language == "python").unwrap();
    assert!(python.pinned_version.is_some());
    assert_eq!(python.server, "Pyright");
    assert_eq!(python.custom_path, None);
    // The footer finds the server of a file by its extension.
    assert!(python.extensions.iter().any(|e| e == "py"));
    let typescript = listed.iter().find(|s| s.language == "typescript").unwrap();
    assert!(typescript.extensions.iter().any(|e| e == "tsx"));
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
    let Response::Servers { servers } = second
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
    assert!(unknown.unwrap_err().contains("unknown language"));

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
