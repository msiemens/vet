//! Python semantics through the protocol with the pinned Pyright. It installs
//! Pyright from npm into a temporary data directory, so it is ignored and runs on
//! request: `cargo nextest run -p vet-backend --run-ignored ignored-only python::`.

use crate::common::{Client, find, handler, repo_with, write};
use vet_protocol::{Request, Response, Severity, Side, SymbolChange};

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
#[ignore = "downloads Pyright; needs npm"]
async fn python_semantics_through_the_protocol() {
    let repo = repo_with(
        &[
            ("pyproject.toml", "[project]\nname = \"p\"\nversion = \"0\"\n"),
            ("a.py", BASE),
        ],
        &[("a.py", DIRTY)],
    );
    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client.install(project, "python").await;
    let revision = client.revision(project).await;
    let path = "a.py";

    // Target side: the dirty tree.
    let at = find(DIRTY, "keep()", 1, 1);
    let (text, unavailable) = client.hover(project, revision, Side::Target, path, at).await;
    assert_eq!(unavailable, None);
    assert!(text.unwrap().contains("def keep() -> int"));

    let defs = client.definition(project, revision, Side::Target, path, at).await;
    assert_eq!(defs.locations.len(), 1, "{defs:?}");
    assert_eq!(defs.locations[0].path, path);
    assert_eq!(defs.locations[0].span.start.line, 0);
    assert!(!defs.locations[0].external);

    let refs = client
        .references(project, revision, Side::Target, path, find(DIRTY, "keep", 0, 1))
        .await;
    assert_eq!(refs.rows.len(), 1, "{refs:?}");
    assert_eq!(refs.rows[0].line, find(DIRTY, "keep()", 1, 0).0 + 1);

    let (diagnostics, unavailable) = client.diagnostics(project, revision, path).await;
    assert_eq!(unavailable, None);
    let error = diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error && d.message.contains("not assignable"))
        .unwrap_or_else(|| panic!("{diagnostics:?}"));
    assert_eq!(error.span.start.line, find(DIRTY, "x: int", 0, 0).0);

    // Base side: a different tree, where `gone` still exists.
    let at = find(BASE, "gone()", 1, 1);
    let (text, _) = client.hover(project, revision, Side::Base, path, at).await;
    assert!(text.unwrap().contains("def gone() -> int"));
    let defs = client.definition(project, revision, Side::Base, path, at).await;
    assert_eq!(defs.locations.len(), 1, "{defs:?}");
    assert_eq!(defs.locations[0].span.start.line, 0);
    let refs = client
        .references(project, revision, Side::Base, path, find(BASE, "gone", 0, 1))
        .await;
    assert_eq!(refs.rows.len(), 1, "{refs:?}");
    assert_eq!(refs.rows[0].line, find(BASE, "gone()", 1, 0).0 + 1);

    // Derived results run on both servers.
    let Response::FileSymbols(changed) = client
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
    assert!(changed.gap.is_none(), "{changed:?}");
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
        let now = client.revision(project).await;
        if now != revision {
            break now;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    let (diagnostics, _) = client.diagnostics(project, settled, path).await;
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
