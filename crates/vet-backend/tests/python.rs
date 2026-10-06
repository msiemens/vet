//! Python semantics through the protocol with the pinned Pyright. It installs
//! Pyright from npm into a temporary data directory, so it runs only when
//! `VET_TEST_INSTALL` lists `python`. Run it alone (`--test-threads=1`).

mod common;

use common::{Client, find, handler, repo_with, write};
use vet_protocol::{Request, Response, Severity, Side, SymbolChange};

fn wanted() -> bool {
    let listed = std::env::var("VET_TEST_INSTALL").is_ok_and(|l| l.split(',').any(|x| x.trim() == "python"));
    if !listed {
        eprintln!("skipped: set VET_TEST_INSTALL=python to download and run Pyright");
    }
    listed
}

const BASE: &str = "\
def gone() -> int:
    return 1


def keep() -> int:
    return 2


def caller() -> int:
    return gone()


def main() -> int:
    return keep()
";

const DIRTY: &str = "\
def keep() -> int:
    return 3


def fresh() -> int:
    return keep()


def broken() -> int:
    x: int = \"x\"
    return x


def caller() -> int:
    return gone()


def main() -> int:
    return fresh()
";

#[tokio::test]
async fn python_semantics_through_the_protocol() {
    if !wanted() {
        return;
    }
    let repo = repo_with(
        &[
            ("pyproject.toml", "[project]\nname = \"p\"\nversion = \"0\"\n"),
            ("a.py", BASE),
        ],
        &[("a.py", DIRTY)],
    );
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let session = client.open(repo.path()).await;
    client.install(session, "python").await;
    let revision = client.revision(session).await;
    let path = "a.py";

    // Target side: the dirty tree.
    let at = find(DIRTY, "keep()", 1, 1);
    let (text, unavailable) = client.hover(session, revision, Side::Target, path, at).await;
    assert_eq!(unavailable, None);
    assert!(text.unwrap().contains("def keep() -> int"));

    let defs = client.definition(session, revision, Side::Target, path, at).await;
    assert_eq!(defs.locations.len(), 1, "{defs:?}");
    assert_eq!(defs.locations[0].path, path);
    assert_eq!(defs.locations[0].span.start.line, 0);
    assert!(!defs.locations[0].external);

    let refs = client
        .references(session, revision, Side::Target, path, find(DIRTY, "keep", 0, 1))
        .await;
    assert_eq!(refs.locations.len(), 1, "{refs:?}");
    assert_eq!(refs.locations[0].span.start.line, find(DIRTY, "keep()", 1, 0).0);

    let (diagnostics, unavailable) = client.diagnostics(session, revision, path).await;
    assert_eq!(unavailable, None);
    let error = diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error && d.message.contains("not assignable"))
        .unwrap_or_else(|| panic!("{diagnostics:?}"));
    assert_eq!(error.span.start.line, find(DIRTY, "x: int", 0, 0).0);

    // Base side: a different tree, where `gone` still exists.
    let at = find(BASE, "gone()", 1, 1);
    let (text, _) = client.hover(session, revision, Side::Base, path, at).await;
    assert!(text.unwrap().contains("def gone() -> int"));
    let defs = client.definition(session, revision, Side::Base, path, at).await;
    assert_eq!(defs.locations.len(), 1, "{defs:?}");
    assert_eq!(defs.locations[0].span.start.line, 0);
    let refs = client
        .references(session, revision, Side::Base, path, find(BASE, "gone", 0, 1))
        .await;
    assert_eq!(refs.locations.len(), 1, "{refs:?}");
    assert_eq!(refs.locations[0].span.start.line, find(BASE, "gone()", 1, 0).0);

    // Derived results run on both servers.
    let Response::ChangedSymbols(changed) = client
        .call(Request::ChangedSymbols { session, revision })
        .await
        .unwrap()
    else {
        panic!("not changed symbols");
    };
    assert!(changed.gaps.is_empty() && changed.unsupported.is_empty(), "{changed:?}");
    let change_of = |symbol: &str| {
        changed
            .symbols
            .iter()
            .find(|c| c.symbol.id.name == [symbol])
            .unwrap_or_else(|| panic!("{symbol} in {:?}", changed.symbols))
            .change
    };
    assert_eq!(change_of("fresh"), SymbolChange::Added);
    assert_eq!(change_of("keep"), SymbolChange::Modified);

    // Edits to the dirty tree reach the server.
    write(repo.path(), "a.py", &DIRTY.replace("x: int = \"x\"", "x: int = 1"));
    let settled = loop {
        let now = client.revision(session).await;
        if now != revision {
            break now;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let (diagnostics, _) = client.diagnostics(session, settled, path).await;
    // `gone` is still undefined there, but the type error is fixed.
    assert!(
        diagnostics.iter().all(|d| !d.message.contains("not assignable")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("\"gone\" is not defined"))
    );
}
