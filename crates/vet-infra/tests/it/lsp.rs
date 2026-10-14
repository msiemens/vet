//! The LSP client against the real rust-analyzer, typescript-language-server,
//! roslyn-language-server and the servers of Svelte, Angular, YAML, JSON, shell scripts and
//! Dockerfiles on tiny projects. Each test is ignored with the tool it needs
//! as its reason; `ci/tool-tests.sh` runs those whose tool is installed.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;
use vet_core::content::RepoPath;
use vet_infra::exec::dotnet_env;
use vet_infra::lsp::{BinaryLocator, LspLauncher, NoRestore, config};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{At, Backend, Feature, LanguageServer, Pos, ServerLauncher, Tree, Unwatched};

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

pub trait Shape {
    fn area(&self) -> u32;
}

pub struct Square(u32);

impl Shape for Square {
    fn area(&self) -> u32 {
        self.0 * self.0
    }
}
";

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
#[ignore = "needs rust-analyzer"]
async fn rust_analyzer_answers_the_primitive_queries() {
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
            AnalysisPolicy::EXECUTING,
            &Unwatched,
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

    let trait_method = At {
        file: lib.clone(),
        pos: find(LIB, "fn area", 0, 3),
    };
    let implementations = server.implementations(&trait_method).await.unwrap();
    assert_eq!(implementations.len(), 1, "{implementations:?}");
    assert_eq!(implementations[0].file, lib);
    assert_eq!(implementations[0].span.start.line, find(LIB, "fn area", 1, 0).line);

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
#[ignore = "needs typescript-language-server"]
async fn typescript_language_server_answers_the_primitive_queries() {
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
            AnalysisPolicy::LIMITED,
            &Unwatched,
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

/// typescript-language-server lists every callback, function expression and local constant in
/// its outline; none of them is a symbol, and a call in one is a call of the function around it.
/// A call from top-level code is reported as a call of a module item named after the file; that
/// is no symbol either.
#[tokio::test]
#[ignore = "needs typescript-language-server"]
async fn typescript_closures_belong_to_the_function_around_them() {
    let dir = TempDir::new().unwrap();
    let src = "\
export function helper(): number {
  return 1;
}
export const arrow = (a: number) => a + helper();
(() => {
  helper();
})();
helper();
export class Widget {
  field = 1;
  method(items: number[]) {
    const local = items.map((i) => i + helper());
    const inner = function () { return 1; };
    function named() { return 2; }
    items.forEach(function (i) { console.log(i); });
    return local;
  }
}
";
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
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("typescript-language-server starts and has every required capability")
        .server;
    let file = path("a.ts");
    assert_eq!(
        symbol_names(server.document_symbols(&file).await.unwrap()),
        [
            "Widget",
            "Widget::field",
            "Widget::method",
            "Widget::method::named",
            "arrow",
            "helper"
        ]
    );

    let helper = At {
        file: file.clone(),
        pos: find(src, "helper", 0, 1),
    };
    assert_eq!(
        symbol_names(server.incoming_calls(&helper).await.unwrap()),
        ["Widget::method", "arrow"]
    );
    server.shutdown().await;
}

/// rust-analyzer lists a closure and a local in no outline and a nested function in its function's;
/// a call in a closure is a call of the function around it.
#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn rust_analyzer_closures_belong_to_the_function_around_them() {
    let dir = TempDir::new().unwrap();
    let src = "\
pub fn helper() -> u32 {
    1
}

pub struct S {
    pub n: u32,
}

impl S {
    pub fn go(&self, extra: u32) -> u32 {
        let local = helper();
        let closure = |x: u32| x + helper();
        fn inner() -> u32 {
            helper()
        }
        closure(extra) + inner() + local
    }
}

pub static VALUE: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| helper());
";
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), src).unwrap();
    let launcher = LspLauncher {
        ready_timeout: Duration::from_secs(120),
        ..LspLauncher::new(BinaryLocator, NoRestore)
    };
    let server = launcher
        .start(
            dir.path(),
            Tree::Live,
            &config::rust_analyzer(),
            AnalysisPolicy::EXECUTING,
            &Unwatched,
        )
        .await
        .expect("rust-analyzer starts and has every required capability")
        .server;
    let lib = path("src/lib.rs");
    assert_eq!(
        symbol_names(server.document_symbols(&lib).await.unwrap()),
        [
            "S",
            "S::n",
            "VALUE",
            "helper",
            "impl S",
            "impl S::go",
            "impl S::go::inner"
        ]
    );
    let helper = At {
        file: lib,
        pos: find(src, "helper", 0, 1),
    };
    assert_eq!(
        symbol_names(server.incoming_calls(&helper).await.unwrap()),
        ["impl S::go", "impl S::go::inner"]
    );
    server.shutdown().await;
}

/// Pyright lists the parameters and locals of a function in its outline, and reports a lambda as
/// `(lambda)` and the code outside every declaration as a module item.
#[tokio::test]
#[ignore = "needs pyright-langserver"]
async fn pyright_lambdas_and_locals_belong_to_the_function_around_them() {
    let dir = TempDir::new().unwrap();
    let src = "\
def helper():
    return 1


helper()

VALUE = helper()


class K:
    attr = helper()

    def method(self, p):
        loc = helper()
        f = lambda: helper()
        def inner():
            return helper()
        return [helper() for _ in range(2)]


handler = lambda: helper()
";
    std::fs::write(dir.path().join("m.py"), src).unwrap();
    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            dir.path(),
            Tree::Immutable,
            &config::pyright(),
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("pyright starts and has every required capability")
        .server;
    let file = path("m.py");
    assert_eq!(
        symbol_names(server.document_symbols(&file).await.unwrap()),
        [
            "K",
            "K::attr",
            "K::method",
            "K::method::inner",
            "VALUE",
            "handler",
            "helper"
        ]
    );
    let helper = At {
        file: file.clone(),
        pos: find(src, "helper", 0, 1),
    };
    assert_eq!(
        symbol_names(server.incoming_calls(&helper).await.unwrap()),
        ["K::method", "K::method::inner"]
    );
    server.shutdown().await;
}

/// Restores the project in `dir` as a restore step of Vet does, and shows everything dotnet said
/// if that fails.
fn dotnet_restore(dir: &Path) {
    let restored = Command::new("dotnet")
        .arg("restore")
        .envs(dotnet_env())
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "dotnet restore: {}\nstdout:\n{}\nstderr:\n{}",
        restored.status,
        String::from_utf8_lossy(&restored.stdout),
        String::from_utf8_lossy(&restored.stderr)
    );
}

/// roslyn-language-server lists no lambda, anonymous method, local variable or parameter in its
/// outline, a local function in its method's; a call in a lambda is a call of the method around
/// it, and top-level statements are the call of a generated entry point, which is no symbol.
#[tokio::test]
#[ignore = "needs roslyn-language-server and dotnet"]
async fn roslyn_lambdas_and_locals_belong_to_the_method_around_them() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("app.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework><OutputType>Library</OutputType></PropertyGroup></Project>",
    )
    .unwrap();
    let src = "\
using System;
using System.Linq;

namespace App;

public class W
{
    public static int Helper() => 1;

    public Func<int> Cb = () => Helper();

    public int Method(int[] items, int p)
    {
        int loc = Helper();
        Func<int> f = () => Helper();
        Func<int> g = delegate () { return Helper(); };
        int Local(int x) { return Helper() + x; }
        var q = items.Select(i => i + Helper()).ToList();
        return loc + f() + g() + Local(1) + q.Count;
    }
}
";
    std::fs::write(dir.path().join("W.cs"), src).unwrap();
    std::fs::write(dir.path().join("Program.cs"), "using App;\n\nvar top = W.Helper();\n").unwrap();
    dotnet_restore(dir.path());
    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            dir.path(),
            Tree::Immutable,
            &config::roslyn(),
            AnalysisPolicy::EXECUTING,
            &Unwatched,
        )
        .await
        .expect("roslyn starts")
        .server;
    let file = path("W.cs");
    // The namespace is the one module, whatever its depth.
    assert_eq!(
        module_counts(server.document_symbols(&file).await.unwrap()),
        counts(&[
            ("App", 1),
            ("App::W", 1),
            ("App::W::Cb : Func<int>", 1),
            ("App::W::Helper() : int", 1),
            ("App::W::Method(int[], int) : int", 1),
            ("App::W::Method(int[], int) : int::Local(int) : int", 1),
        ])
    );
    let helper = At {
        file,
        pos: find(src, "Helper", 0, 1),
    };
    assert_eq!(
        symbol_names(server.incoming_calls(&helper).await.unwrap()),
        [
            "App::W::Method(int[], int) : int",
            "App::W::Method(int[], int) : int::Local(int) : int",
        ]
    );
    server.shutdown().await;
}

/// Roslyn has its projects loaded when it is ready, so a hover on a file nobody has
/// opened yet already resolves across the project, on either kind of tree. (That the
/// client does not wait for such a server to go quiet is tested against a fake, in
/// `lsp::server`.)
#[tokio::test]
#[ignore = "needs roslyn-language-server and dotnet"]
async fn roslyn_answers_a_hover_on_a_newly_opened_file() {
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
    dotnet_restore(dir.path());

    for tree in [Tree::Immutable, Tree::Live] {
        let server = LspLauncher::new(BinaryLocator, NoRestore)
            .start(
                dir.path(),
                tree,
                &config::roslyn(),
                AnalysisPolicy::EXECUTING,
                &Unwatched,
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
        for i in 1..5 {
            let text = server.hover(&hover(i)).await.unwrap().expect("a hover on `Run`");
            assert!(text.contains("Run"), "{text}");
        }
        let (a, b) = vet_semantic::join::join(server.hover(&hover(5)), server.references(&hover(0))).await;
        a.unwrap();
        b.unwrap();
        server.shutdown().await;
    }
}

/// The server publishes under its own spelling of the file's URI, which differs
/// from Vet's when the path has characters it encodes (parentheses, brackets, `+`, `@`).
#[tokio::test]
#[ignore = "needs typescript-language-server"]
async fn typescript_pushed_diagnostics_are_found_for_a_path_the_server_spells_differently() {
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
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("typescript-language-server starts")
        .server;
    // A spelling the client does not match would leave the answer to the server's timeout,
    // and an empty one.
    let diagnostics = server.diagnostics(&path("[id]+x/é@a.ts")).await.unwrap();
    assert!(
        diagnostics.iter().any(|d| d.message.contains("not assignable")),
        "{diagnostics:?}"
    );
    server.shutdown().await;
}

/// Each symbol's name with how many of its leading names are modules or namespaces.
fn module_counts(symbols: Vec<vet_semantic::ports::SymbolInfo>) -> Vec<(String, usize)> {
    let mut counts: Vec<_> = symbols.iter().map(|s| (s.sym.name.join("::"), s.modules)).collect();
    counts.sort();
    counts
}

fn counts(expected: &[(&str, usize)]) -> Vec<(String, usize)> {
    expected.iter().map(|(name, n)| ((*name).to_owned(), *n)).collect()
}

/// A TypeScript namespace is one module item named with its dots, whatever its depth.
#[tokio::test]
#[ignore = "needs typescript-language-server"]
async fn typescript_namespaces_are_counted_as_modules() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true}}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("a.ts"),
        "export namespace Acme.Billing {\n  export class Invoice { merge() {} }\n}\nexport class Box { create() {} }\n",
    )
    .unwrap();
    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            dir.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("typescript-language-server starts and has every required capability")
        .server;
    assert_eq!(
        module_counts(server.document_symbols(&path("a.ts")).await.unwrap()),
        counts(&[
            ("Acme.Billing", 1),
            ("Acme.Billing::Invoice", 1),
            ("Acme.Billing::Invoice::merge", 1),
            ("Box", 0),
            ("Box::create", 0),
        ])
    );
    server.shutdown().await;
}

/// rust-analyzer lists `mod`s as modules, and an `impl` inside them in their chain.
#[tokio::test]
#[ignore = "needs rust-analyzer"]
async fn rust_analyzer_modules_are_counted_as_modules() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub mod billing {\n    pub mod core {\n        pub struct Invoice;\n        impl Invoice {\n            pub fn merge(&self) {}\n        }\n    }\n}\n",
    )
    .unwrap();
    let launcher = LspLauncher {
        ready_timeout: Duration::from_secs(120),
        ..LspLauncher::new(BinaryLocator, NoRestore)
    };
    let server = launcher
        .start(
            dir.path(),
            Tree::Live,
            &config::rust_analyzer(),
            AnalysisPolicy::EXECUTING,
            &Unwatched,
        )
        .await
        .expect("rust-analyzer starts and has every required capability")
        .server;
    assert_eq!(
        module_counts(server.document_symbols(&path("src/lib.rs")).await.unwrap()),
        counts(&[
            ("billing", 1),
            ("billing::core", 2),
            ("billing::core::Invoice", 2),
            ("billing::core::impl Invoice", 2),
            ("billing::core::impl Invoice::merge", 2),
        ])
    );
    server.shutdown().await;
}

fn symbol_names(mut symbols: Vec<vet_semantic::ports::SymbolInfo>) -> Vec<String> {
    symbols.sort_by_key(|s| s.span.start.line);
    let mut names: Vec<String> = symbols.iter().map(|s| s.sym.name.join("::")).collect();
    names.sort();
    names
}

/// A server of `backend` on `dir` for the language servers without dependencies to restore.
async fn start_server(dir: &Path, backend: &Backend, policy: AnalysisPolicy) -> vet_infra::lsp::LspServer {
    LspLauncher::new(BinaryLocator, NoRestore)
        .start(dir, Tree::Immutable, backend, policy, &Unwatched)
        .await
        .unwrap_or_else(|e| panic!("{} starts: {e}", backend.config().binary))
        .server
}

fn write(dir: &Path, name: &str, text: &str) {
    let file = dir.join(name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

fn messages(diagnostics: &[vet_semantic::ports::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

const WORKFLOW: &str = "\
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        bogus: 1
";

/// yaml-language-server offers no references, call hierarchy, workspace symbols or implementations:
/// the launch accepts it and says which it lacks. While the project is not trusted the workflow is
/// checked against the bundled schema, and an address a file names is refused by Vet, not fetched.
#[tokio::test]
#[ignore = "needs yaml-language-server"]
async fn yaml_checks_against_the_bundled_schemas_and_refuses_the_address_a_file_names() {
    // A directory above the project whose name starts with a dot, as the ones under `~/.local` do.
    let dir = TempDir::with_prefix(".vet-yaml-").unwrap();
    write(dir.path(), ".github/workflows/ci.yml", WORKFLOW);
    write(
        dir.path(),
        "deploy/k8s/app.yaml",
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: app\nspec:\n  replicas: many\n",
    );
    write(
        dir.path(),
        "other.yaml",
        "# yaml-language-server: $schema=https://schemas.invalid/own.json\nname: x\n",
    );
    let server = start_server(dir.path(), &config::yaml(), AnalysisPolicy::LIMITED).await;

    let offered = server.features();
    assert!(offered.offers(Feature::DocumentSymbols) && offered.offers(Feature::Hover));
    for lacking in [
        Feature::References,
        Feature::CallHierarchy,
        Feature::WorkspaceSymbols,
        Feature::Implementations,
    ] {
        assert!(!offered.offers(lacking), "{lacking:?}");
    }

    let workflow = path(".github/workflows/ci.yml");
    let found = server.diagnostics(&workflow).await.unwrap();
    assert!(
        messages(&found).contains("bogus"),
        "the bundled schema is applied: {found:?}"
    );
    let names = symbol_names(server.document_symbols(&workflow).await.unwrap());
    assert!(names.contains(&"jobs".to_owned()), "{names:?}");
    let hover = At {
        file: workflow,
        pos: find(WORKFLOW, "runs-on", 0, 1),
    };
    let text = server
        .hover(&hover)
        .await
        .unwrap()
        .expect("the schema describes runs-on");
    assert!(text.to_lowercase().contains("runner"), "{text}");

    let own = server.diagnostics(&path("other.yaml")).await.unwrap();
    assert!(
        messages(&own).contains("not one of the schemas bundled with Vet"),
        "the request for the address reached Vet and was refused: {own:?}"
    );

    // The schema of a Kubernetes kind is picked from the bundled one.
    let manifest = path("deploy/k8s/app.yaml");
    let found = server.diagnostics(&manifest).await.unwrap();
    assert!(
        messages(&found).contains("Missing property \"selector\""),
        "the manifest is checked: {found:?}"
    );
    server.shutdown().await;
}

/// A trusted project lets the server load what a file names itself: the same file fails here
/// with the network's answer, not with Vet's refusal.
#[tokio::test]
#[ignore = "needs yaml-language-server"]
async fn yaml_loads_the_address_a_file_names_itself_once_the_project_is_trusted() {
    let dir = TempDir::new().unwrap();
    write(
        dir.path(),
        "other.yaml",
        "# yaml-language-server: $schema=https://schemas.invalid/own.json\nname: x\n",
    );
    let server = start_server(dir.path(), &config::yaml(), AnalysisPolicy::EXECUTING).await;
    let own = server.diagnostics(&path("other.yaml")).await.unwrap();
    let text = messages(&own);
    assert!(
        text.contains("schemas.invalid"),
        "the server tried the address: {own:?}"
    );
    assert!(!text.contains("bundled with Vet"), "{own:?}");
    server.shutdown().await;
}

/// The JSON server checks `package.json` against the bundled schema without the network, and
/// refuses an address a `$schema` names.
#[tokio::test]
#[ignore = "needs vscode-json-language-server"]
async fn json_checks_against_the_bundled_schemas_and_refuses_the_address_a_file_names() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "package.json", "{\"name\": 1, \"version\": \"1.0.0\"}\n");
    write(dir.path(), "broken.json", "{\"a\": 1,\n");
    write(
        dir.path(),
        "own.json",
        "{\"$schema\": \"https://schemas.invalid/own.json\", \"a\": 1}\n",
    );
    let server = start_server(dir.path(), &config::json(), AnalysisPolicy::LIMITED).await;

    let offered = server.features();
    assert!(offered.offers(Feature::DocumentSymbols) && offered.offers(Feature::Hover));
    for lacking in [
        Feature::Definition,
        Feature::References,
        Feature::CallHierarchy,
        Feature::WorkspaceSymbols,
    ] {
        assert!(!offered.offers(lacking), "{lacking:?}");
    }

    let package = server.diagnostics(&path("package.json")).await.unwrap();
    assert!(
        messages(&package).contains("string"),
        "the bundled schema is applied: {package:?}"
    );
    let broken = server.diagnostics(&path("broken.json")).await.unwrap();
    assert!(!broken.is_empty(), "a syntax error is reported");
    let own = server.diagnostics(&path("own.json")).await.unwrap();
    assert!(
        messages(&own).contains("not one of the schemas bundled with Vet"),
        "the request for the address reached Vet and was refused: {own:?}"
    );
    assert_eq!(
        symbol_names(server.document_symbols(&path("package.json")).await.unwrap()),
        ["name", "version"]
    );
    server.shutdown().await;
}

#[tokio::test]
#[ignore = "needs vscode-json-language-server"]
async fn json_loads_the_address_a_file_names_itself_once_the_project_is_trusted() {
    let dir = TempDir::new().unwrap();
    write(
        dir.path(),
        "own.json",
        "{\"$schema\": \"https://schemas.invalid/own.json\", \"a\": 1}\n",
    );
    let server = start_server(dir.path(), &config::json(), AnalysisPolicy::EXECUTING).await;
    let own = server.diagnostics(&path("own.json")).await.unwrap();
    let text = messages(&own);
    assert!(
        text.contains("schemas.invalid"),
        "the server tried the address: {own:?}"
    );
    assert!(!text.contains("bundled with Vet"), "{own:?}");
    server.shutdown().await;
}

const SCRIPT: &str = "\
#!/bin/bash
greet() {
  echo \"hello $1\"
}
name=$1
greet $name
";

/// bash-language-server answers symbols, definitions, references and hover; it has no call
/// hierarchy and no implementations.
#[tokio::test]
#[ignore = "needs bash-language-server"]
async fn bash_language_server_answers_what_it_offers() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "run.sh", SCRIPT);
    let server = start_server(dir.path(), &config::bash(), AnalysisPolicy::LIMITED).await;
    let offered = server.features();
    assert!(offered.offers(Feature::DocumentSymbols) && offered.offers(Feature::References));
    assert!(!offered.offers(Feature::CallHierarchy) && !offered.offers(Feature::Implementations));

    let file = path("run.sh");
    let names = symbol_names(server.document_symbols(&file).await.unwrap());
    assert!(
        names.contains(&"greet".to_owned()) && names.contains(&"name".to_owned()),
        "{names:?}"
    );
    let call = At {
        file: file.clone(),
        pos: find(SCRIPT, "greet $name", 0, 1),
    };
    assert_eq!(server.definition(&call).await.unwrap().len(), 1);
    assert_eq!(server.references(&call).await.unwrap().len(), 1);
    server.shutdown().await;
}

/// With ShellCheck on `PATH` the server reports its findings as diagnostics.
#[tokio::test]
#[ignore = "needs bash-language-server and shellcheck"]
async fn bash_language_server_reports_shellcheck_findings() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "run.sh", SCRIPT);
    let server = start_server(dir.path(), &config::bash(), AnalysisPolicy::LIMITED).await;
    let found = server.diagnostics(&path("run.sh")).await.unwrap();
    let shellcheck: Vec<_> = found
        .iter()
        .filter(|d| d.source.as_deref() == Some("shellcheck"))
        .collect();
    assert!(
        shellcheck.iter().any(|d| d.code.as_deref() == Some("SC2086")),
        "the unquoted $name is reported: {found:?}"
    );
    server.shutdown().await;
}

const DOCKERFILE: &str = "\
FROM alpine:3.20 AS base
ARG VERSION=1
RUN echo $VERSION
COPY
";

/// The Dockerfile server is routed by file name: it needs no extension.
#[tokio::test]
#[ignore = "needs docker-langserver"]
async fn dockerfile_language_server_answers_what_it_offers() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "Dockerfile", DOCKERFILE);
    let server = start_server(dir.path(), &config::dockerfile(), AnalysisPolicy::LIMITED).await;
    let offered = server.features();
    assert!(offered.offers(Feature::DocumentSymbols) && offered.offers(Feature::Definition));
    assert!(!offered.offers(Feature::CallHierarchy) && !offered.offers(Feature::WorkspaceSymbols));

    let file = path("Dockerfile");
    let found = server.diagnostics(&file).await.unwrap();
    assert!(
        messages(&found).contains("COPY"),
        "the instruction without arguments is reported: {found:?}"
    );
    assert!(!server.document_symbols(&file).await.unwrap().is_empty());
    let hover = At {
        file,
        pos: find(DOCKERFILE, "FROM", 0, 1),
    };
    assert!(server.hover(&hover).await.unwrap().is_some());
    server.shutdown().await;
}

const COMPONENT: &str = "\
<script lang=\"ts\">
  export let count: number = 0;
  function bump(): number {
    return count + 1;
  }
  const bad: string = 1;
</script>

<button on:click={bump}>{count}</button>
";

/// svelte-language-server serves a component through TypeScript, which it finds beside itself.
#[tokio::test]
#[ignore = "needs svelteserver"]
async fn svelte_language_server_answers_through_typescript() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "tsconfig.json", "{\"compilerOptions\":{\"strict\":true}}");
    write(dir.path(), "App.svelte", COMPONENT);
    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            dir.path(),
            Tree::Immutable,
            &config::svelte(),
            AnalysisPolicy::EXECUTING,
            &Unwatched,
        )
        .await
        .expect("svelteserver starts")
        .server;
    let file = path("App.svelte");
    let found = server.diagnostics(&file).await.unwrap();
    assert!(messages(&found).contains("not assignable"), "{found:?}");
    let call = At {
        file: file.clone(),
        pos: find(COMPONENT, "bump", 1, 1),
    };
    let hover = server.hover(&call).await.unwrap().expect("a hover on bump");
    assert!(hover.contains("bump"), "{hover}");
    assert_eq!(server.definition(&call).await.unwrap().len(), 1);
    let names = symbol_names(server.document_symbols(&file).await.unwrap());
    assert!(names.iter().any(|n| n.contains("bump")), "{names:?}");
    server.shutdown().await;
}

/// The Angular server resolves `typescript` and `@angular/language-service` from the project, or
/// from its own install, and reports an error in a template. The project's Angular packages come
/// from the network.
#[tokio::test]
#[ignore = "needs ngserver and npm"]
async fn angular_language_server_checks_a_template() {
    let dir = TempDir::new().unwrap();
    write(
        dir.path(),
        "package.json",
        "{\"name\":\"app\",\"version\":\"0.0.0\",\"private\":true,\"dependencies\":{\"@angular/core\":\"22.2.2\",\"@angular/common\":\"22.2.2\",\"@angular/compiler\":\"22.2.2\",\"rxjs\":\"7.8.2\",\"tslib\":\"2.8.1\"}}",
    );
    let installed = Command::new("npm")
        .args(["install", "--no-audit", "--no-fund", "--ignore-scripts"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    write(
        dir.path(),
        "tsconfig.json",
        "{\"compilerOptions\":{\"strict\":true,\"target\":\"ES2022\",\"module\":\"ES2022\",\"moduleResolution\":\"bundler\",\"experimentalDecorators\":false},\"angularCompilerOptions\":{\"strictTemplates\":true},\"files\":[\"app.ts\"]}",
    );
    write(
        dir.path(),
        "app.ts",
        "import { Component } from '@angular/core';\n\n@Component({ selector: 'app-root', templateUrl: './app.html' })\nexport class App {\n  title = 'hello';\n}\n",
    );
    write(dir.path(), "app.html", "<h1>{{ title }}</h1>\n<p>{{ missing }}</p>\n");
    let launcher = LspLauncher {
        ready_timeout: Duration::from_secs(120),
        ..LspLauncher::new(BinaryLocator, NoRestore)
    };
    let server = launcher
        .start(
            dir.path(),
            Tree::Immutable,
            &config::angular(),
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("ngserver starts")
        .server;
    let found = server.diagnostics(&path("app.html")).await.unwrap();
    assert!(messages(&found).contains("missing"), "{found:?}");
    server.shutdown().await;
}
