//! The LSP client against a real rust-analyzer on a tiny cargo project.
//! Skipped (passes) when the binary is not installed.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;
use vet_core::content::RepoPath;
use vet_infra::lsp::{BinaryLocator, LspLauncher, NoRestore, config};
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
        ..LspLauncher::new(BinaryLocator, NoRestore)
    };
    let server = launcher
        .start(
            dir.path(),
            Tree::Live,
            &config::rust_analyzer(),
            AnalysisPolicy { allow_executing: true },
        )
        .await
        .expect("rust-analyzer starts and has every required capability")
        .server;
    let lib = path("src/lib.rs");

    let symbols = server.document_symbols(&lib).await.unwrap();
    let names: Vec<String> = symbols.iter().map(|s| s.sym.name.join("::")).collect();
    assert!(names.contains(&"Counter".to_string()), "{names:?}");
    assert!(names.contains(&"impl Counter::bump".to_string()), "{names:?}");
    assert!(names.contains(&"entry".to_string()), "{names:?}");
    // What holds code is told from the fields and locals the outline lists as well.
    let container = |name: &str| {
        symbols
            .iter()
            .find(|s| s.sym.name.join("::") == name)
            .map(|s| s.container)
    };
    assert_eq!(container("Counter"), Some(true));
    assert_eq!(container("impl Counter"), Some(true));
    assert_eq!(container("impl Counter::bump"), Some(true));
    assert_eq!(container("entry"), Some(true));
    assert_eq!(container("Counter::n"), Some(false));

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

    // The symbols of other text under the file's name, as for a base side.
    let base = "pub fn gone() -> u32 {\n    1\n}\n\npub fn entry() -> u32 {\n    gone()\n}\n";
    let names = symbol_names(server.text_symbols(&lib, base).await.unwrap());
    assert_eq!(names, ["entry", "gone"]);
    let names = symbol_names(server.document_symbols(&lib).await.unwrap());
    assert!(names.contains(&"added".to_string()) && !names.contains(&"gone".to_string()));
    assert_eq!(std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(), edited);
    assert!(!dir.path().join("src").read_dir().unwrap().any(|e| {
        let name = e.unwrap().file_name();
        name != "lib.rs"
    }));

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
    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            dir.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy { allow_executing: false },
        )
        .await
        .expect("typescript-language-server starts and has every required capability")
        .server;
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

    // Other text under the file's name, as for a base side. A script declaring
    // a global the real file also declares must not disturb the real file.
    let base = "function helper() { return 2; }\nfunction removed() { return 3; }\n";
    let found = symbol_names(server.text_symbols(&file, base).await.unwrap());
    assert_eq!(found, ["helper", "removed"]);
    assert_eq!(
        symbol_names(server.document_symbols(&file).await.unwrap()),
        ["entry", "helper"]
    );
    assert!(server.diagnostics(&file).await.unwrap().is_empty());
    server.shutdown().await;
}

/// Roslyn has its projects loaded when it is ready, so opening another file of
/// them is no reason to wait for it to go quiet: a hover on a file nobody has
/// opened yet answers at once, on either kind of tree.
#[tokio::test]
async fn roslyn_answers_a_hover_on_a_newly_opened_file_without_waiting_for_quiet() {
    if !installed("roslyn-language-server") || !installed("dotnet") {
        eprintln!("skipped: roslyn-language-server or dotnet is not installed");
        return;
    }
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("app.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>",
    )
    .unwrap();
    for i in 0..6 {
        let class = format!("namespace App;\npublic class C{i}\n{{\n    public int Run(int x) => x + {i};\n}}\n");
        std::fs::write(dir.path().join(format!("C{i}.cs")), class).unwrap();
    }
    let restored = Command::new("dotnet")
        .arg("restore")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "{}",
        String::from_utf8_lossy(&restored.stdout)
    );

    for tree in [Tree::Immutable, Tree::Live] {
        let server = LspLauncher::new(BinaryLocator, NoRestore)
            .start(
                dir.path(),
                tree,
                &config::roslyn(),
                AnalysisPolicy { allow_executing: true },
            )
            .await
            .expect("roslyn starts")
            .server;
        let hover = |i: usize| At {
            file: path(&format!("C{i}.cs")),
            pos: Pos { line: 3, col: 15 },
        };
        // The first answer pays for the server's own warm-up.
        assert!(server.hover(&hover(0)).await.unwrap().is_some());
        let started = std::time::Instant::now();
        for i in 1..5 {
            let text = server.hover(&hover(i)).await.unwrap().expect("a hover on `Run`");
            assert!(text.contains("Run"), "{text}");
        }
        let (a, b) = vet_semantic::join::join(server.hover(&hover(5)), server.references(&hover(0))).await;
        a.unwrap();
        b.unwrap();
        // Waiting for quiet costs three quarters of a second per file opened.
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        server.shutdown().await;
    }
}

/// The server publishes under its own spelling of the file's URI, which differs
/// from Vet's when the path has characters it encodes (parentheses, brackets, `+`, `@`).
#[tokio::test]
async fn pushed_diagnostics_are_found_for_a_path_the_server_spells_differently() {
    if !installed("typescript-language-server") {
        eprintln!("skipped: typescript-language-server is not installed");
        return;
    }
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("my proj (1)");
    std::fs::create_dir_all(project.join("[id]+x")).unwrap();
    std::fs::write(project.join("tsconfig.json"), "{\"compilerOptions\":{\"strict\":true}}").unwrap();
    std::fs::write(project.join("[id]+x/é@a.ts"), "const x: number = \"a\";\n").unwrap();
    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            &project,
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy { allow_executing: false },
        )
        .await
        .expect("typescript-language-server starts")
        .server;
    let started = std::time::Instant::now();
    let diagnostics = server.diagnostics(&path("[id]+x/é@a.ts")).await.unwrap();
    assert!(
        diagnostics.iter().any(|d| d.message.contains("not assignable")),
        "{diagnostics:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(15));
    server.shutdown().await;
}

/// Files opened together share one wait for the server to go quiet, instead of
/// each waiting after the one before.
#[tokio::test]
async fn diagnostics_of_files_opened_together_share_one_quiet_wait() {
    if !installed("typescript-language-server") {
        eprintln!("skipped: typescript-language-server is not installed");
        return;
    }
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true}}",
    )
    .unwrap();
    for i in 0..6 {
        std::fs::write(
            dir.path().join(format!("a{i}.ts")),
            format!("export const x{i}: number = {i};\n"),
        )
        .unwrap();
    }
    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            dir.path(),
            Tree::Live,
            &config::typescript(),
            AnalysisPolicy { allow_executing: false },
        )
        .await
        .expect("typescript-language-server starts")
        .server;
    let started = std::time::Instant::now();
    let files: Vec<_> = (0..6).map(|i| path(&format!("a{i}.ts"))).collect();
    let answers = vet_semantic::join::bounded(6, &files, |file| server.diagnostics(file)).await;
    assert!(answers.iter().all(Result::is_ok));
    // One after another, each file would add its own wait of three quarters of a second.
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    server.shutdown().await;
}

fn symbol_names(mut symbols: Vec<vet_semantic::ports::SymbolInfo>) -> Vec<String> {
    symbols.sort_by_key(|s| s.span.start.line);
    let mut names: Vec<String> = symbols.iter().map(|s| s.sym.name.join("::")).collect();
    names.sort();
    names
}
