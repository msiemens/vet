//! The LSP client against a real rust-analyzer on a tiny cargo project.
//! Skipped (passes) when the binary is not installed.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;
use vet_core::content::RepoPath;
use vet_infra::lsp::{LspLauncher, config};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{At, LanguageServer, Pos, ServerLauncher, Tree};

const LIB: &str = "\
pub struct Counter {
    n: u32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { n: 0 }
    }

    pub fn bump(&mut self) -> u32 {
        self.n += 1;
        self.n
    }
}

pub fn helper() -> u32 {
    let mut c = Counter::new();
    c.bump()
}

pub fn entry() -> u32 {
    helper()
}
";

fn installed(binary: &str) -> bool {
    Command::new(binary).arg("--version").output().is_ok()
}

fn path(p: &str) -> RepoPath {
    RepoPath(p.as_bytes().to_vec())
}

/// The position of the `nth` (0-based) occurrence of `needle`, offset by `inside` columns.
fn find(text: &str, needle: &str, nth: usize, inside: u32) -> Pos {
    let (line, col) = text
        .lines()
        .enumerate()
        .flat_map(|(l, s)| s.match_indices(needle).map(move |(c, _)| (l, c)))
        .nth(nth)
        .expect("needle present");
    Pos {
        line: line as u32,
        col: col as u32 + inside,
    }
}

fn project(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), LIB).unwrap();
}

#[tokio::test]
async fn rust_analyzer_answers_the_primitive_queries() {
    if !installed("rust-analyzer") {
        eprintln!("skipped: rust-analyzer is not installed");
        return;
    }
    let dir = TempDir::new().unwrap();
    project(dir.path());
    let launcher = LspLauncher {
        ready_timeout: Duration::from_secs(120),
    };
    let server = launcher
        .start(
            dir.path(),
            Tree::Live,
            &config::rust_analyzer(),
            AnalysisPolicy { allow_executing: true },
        )
        .await
        .expect("rust-analyzer starts and has every required capability");
    let lib = path("src/lib.rs");

    let symbols = server.document_symbols(&lib).await.unwrap();
    let names: Vec<String> = symbols.iter().map(|s| s.sym.name.join("::")).collect();
    assert!(names.contains(&"Counter".to_string()), "{names:?}");
    assert!(names.contains(&"impl Counter::bump".to_string()), "{names:?}");
    assert!(names.contains(&"entry".to_string()), "{names:?}");

    let call = At {
        file: lib.clone(),
        pos: find(LIB, "helper()", 1, 1),
    };
    let hover = server.hover(&call).await.unwrap().expect("hover on a function");
    assert!(hover.contains("fn helper"), "{hover}");

    let defs = server.definition(&call).await.unwrap();
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].file, lib);
    assert!(!defs[0].external);
    assert_eq!(defs[0].span.start.line, find(LIB, "fn helper", 0, 0).line);

    let helper = At {
        file: lib.clone(),
        pos: find(LIB, "helper", 0, 1),
    };
    let refs = server.references(&helper).await.unwrap();
    assert_eq!(refs.len(), 1, "{refs:?}");
    assert_eq!(refs[0].span.start.line, call.pos.line);

    let callers = server.incoming_calls(&helper).await.unwrap();
    let callers: Vec<String> = callers.iter().map(|s| s.sym.name.join("::")).collect();
    assert_eq!(callers, ["entry"]);
    let callees = server.outgoing_calls(&helper).await.unwrap();
    let callees: Vec<String> = callees.iter().map(|s| s.sym.name.join("::")).collect();
    assert!(callees.contains(&"impl Counter::bump".to_string()), "{callees:?}");

    let found = server.workspace_symbols("entry").await.unwrap();
    assert!(
        found
            .iter()
            .any(|s| s.sym.name.last().map(String::as_str) == Some("entry"))
    );

    assert!(server.diagnostics(&lib).await.unwrap().is_empty());

    // An edit on disk is seen by the next query (the live worktree case).
    let edited = format!("{LIB}\npub fn added() -> u32 {{\n    let unused: u32 = \"x\";\n    1\n}}\n");
    std::fs::write(dir.path().join("src/lib.rs"), &edited).unwrap();
    let names: Vec<String> = server
        .document_symbols(&lib)
        .await
        .unwrap()
        .iter()
        .map(|s| s.sym.name.join("::"))
        .collect();
    assert!(names.contains(&"added".to_string()), "{names:?}");
    let diagnostics = server.diagnostics(&lib).await.unwrap();
    assert!(
        diagnostics.iter().any(|d| d.message.contains("expected u32")),
        "{diagnostics:?}"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn typescript_language_server_answers_the_primitive_queries() {
    if !installed("typescript-language-server") {
        eprintln!("skipped: typescript-language-server is not installed");
        return;
    }
    let dir = TempDir::new().unwrap();
    let src = "export function helper(): number {\n  return 1;\n}\nexport function entry(): number {\n  return helper();\n}\n";
    std::fs::write(
        dir.path().join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true}}",
    )
    .unwrap();
    std::fs::write(dir.path().join("a.ts"), src).unwrap();
    let server = LspLauncher::default()
        .start(
            dir.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy { allow_executing: false },
        )
        .await
        .expect("typescript-language-server starts and has every required capability");
    let file = path("a.ts");
    let mut names: Vec<String> = server
        .document_symbols(&file)
        .await
        .unwrap()
        .iter()
        .map(|s| s.sym.name.join("::"))
        .collect();
    names.sort();
    assert_eq!(names, ["entry", "helper"]);

    // Snapshot files are immutable, so an open document is not re-read.
    std::fs::write(dir.path().join("a.ts"), format!("{src}export function late() {{}}\n")).unwrap();
    assert_eq!(server.document_symbols(&file).await.unwrap().len(), 2);
    let call = At {
        file: file.clone(),
        pos: find(src, "helper()", 0, 1),
    };
    assert!(server.hover(&call).await.unwrap().is_some());
    assert_eq!(server.definition(&call).await.unwrap().len(), 1);
    let decl = At {
        file: file.clone(),
        pos: find(src, "helper", 0, 1),
    };
    assert_eq!(server.references(&decl).await.unwrap().len(), 1);
    assert_eq!(server.incoming_calls(&decl).await.unwrap().len(), 1);
    assert!(server.diagnostics(&file).await.unwrap().is_empty());
    server.shutdown().await;
}
