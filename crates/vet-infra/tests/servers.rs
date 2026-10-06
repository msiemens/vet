//! Installing the pinned servers into a data directory and starting them
//! through the resolver. It downloads packages, so it only runs when
//! `VET_TEST_INSTALL` lists the languages to try (`typescript,python,csharp`);
//! run it alone (`--test-threads=1`).

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use tempfile::TempDir;
use vet_app::ports::{ServerOrigin, ServerResolver, ServerState};
use vet_app::servers::Servers;
use vet_core::content::RepoPath;
use vet_infra::lsp::{LspLauncher, config};
use vet_infra::servers::ToolResolver;
use vet_infra::store::SqliteStore;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{At, BackendId, LanguageServer, Pos, ServerLauncher, Tree};

fn wanted(language: &str) -> bool {
    let Ok(list) = std::env::var("VET_TEST_INSTALL") else {
        eprintln!("skipped: set VET_TEST_INSTALL={language} to download and run the server");
        return false;
    };
    list.split(',').any(|l| l.trim() == language)
}

fn path(p: &str) -> RepoPath {
    RepoPath(p.as_bytes().to_vec())
}

struct Setup {
    data: TempDir,
    store: SqliteStore,
}

async fn setup() -> Setup {
    let data = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(data.path(), Default::default()).await.unwrap();
    Setup { data, store }
}

fn servers(setup: &Setup) -> Arc<Servers<ToolResolver, SqliteStore>> {
    let languages = ["rust", "typescript", "python", "csharp"]
        .map(|l| BackendId(l.into()))
        .to_vec();
    Arc::new(Servers::new(
        ToolResolver::new(setup.data.path().to_owned()),
        setup.store.clone(),
        languages,
    ))
}

async fn install(setup: &Setup, project: &Path, language: &str) -> Vec<String> {
    let resolver = ToolResolver::new(setup.data.path().to_owned());
    let id = BackendId(language.into());
    let lines = Mutex::new(Vec::new());
    let progress = |line: &str| lines.lock().unwrap().push(line.to_owned());
    resolver.install(&id, &progress).await.expect("installs");
    let ServerState::Found(found) = resolver.resolve(&id, project, None).await else {
        panic!("not found after the install");
    };
    assert_eq!(found.origin, ServerOrigin::Installed);
    assert_eq!(found.version, resolver.pinned(&id), "{found:?}");
    lines.into_inner().unwrap()
}

#[tokio::test]
async fn typescript_installs_and_uses_the_project_typescript() {
    if !wanted("typescript") {
        return;
    }
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    let src = "export function helper(): number {\n  return 1;\n}\nexport const x: number = helper();\n";
    std::fs::write(
        project.path().join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true}}",
    )
    .unwrap();
    std::fs::write(project.path().join("a.ts"), src).unwrap();
    // The project's own typescript, installed like a dependency would be.
    let npm = std::process::Command::new("npm")
        .args(["install", "--no-audit", "--no-fund", "--prefix"])
        .arg(project.path())
        .arg("typescript")
        .output()
        .unwrap();
    assert!(npm.status.success(), "{}", String::from_utf8_lossy(&npm.stderr));

    let output = install(&setup, project.path(), "typescript").await;
    assert!(!output.is_empty());
    // The latest `typescript` is TypeScript 7, which has no tsserver.js: the fallback serves.
    let resolver = ToolResolver::new(setup.data.path().to_owned());
    let ServerState::Found(found) = resolver
        .resolve(&BackendId("typescript".into()), project.path(), None)
        .await
    else {
        panic!("not found");
    };
    assert!(found.notes[0].contains("Vet's fallback"), "{:?}", found.notes);

    let launcher = LspLauncher::new(servers(&setup));
    let server = launcher
        .start(
            project.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy { allow_executing: false },
        )
        .await
        .expect("the installed server starts and has every required capability");
    let at = At {
        file: path("a.ts"),
        pos: Pos { line: 3, col: 25 },
    };
    assert!(server.hover(&at).await.unwrap().is_some());
    assert_eq!(server.definition(&at).await.unwrap().len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn csharp_installs_and_starts() {
    if !wanted("csharp") {
        return;
    }
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    let program = "namespace App;\n\npublic static class Helper\n{\n    public static int One() => 1;\n}\n\npublic static class Program\n{\n    public static int Main() => Helper.One();\n}\n";
    std::fs::write(
        project.path().join("App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n    <OutputType>Library</OutputType>\n  </PropertyGroup>\n</Project>\n",
    )
    .unwrap();
    std::fs::write(project.path().join("Program.cs"), program).unwrap();
    let restore = std::process::Command::new("dotnet")
        .arg("restore")
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stdout));
    install(&setup, project.path(), "csharp").await;

    let launcher = LspLauncher::new(servers(&setup));
    let server = launcher
        .start(
            project.path(),
            Tree::Live,
            &config::roslyn(),
            AnalysisPolicy { allow_executing: true },
        )
        .await
        .expect("the installed server starts and has every required capability");
    let file = path("Program.cs");
    let names: Vec<String> = server
        .document_symbols(&file)
        .await
        .unwrap()
        .iter()
        .map(|s| s.sym.name.join("::"))
        .collect();
    assert!(names.iter().any(|n| n.contains("Helper")), "{names:?}");
    let call = At {
        file: file.clone(),
        pos: Pos { line: 9, col: 40 },
    };
    assert!(server.hover(&call).await.unwrap().is_some());
    assert_eq!(server.definition(&call).await.unwrap().len(), 1);
    let decl = At {
        file: file.clone(),
        pos: Pos { line: 4, col: 23 },
    };
    assert_eq!(server.references(&decl).await.unwrap().len(), 1);
    let callers: Vec<String> = server
        .incoming_calls(&decl)
        .await
        .unwrap()
        .iter()
        .map(|s| s.sym.name.last().cloned().unwrap_or_default())
        .collect();
    assert!(callers.iter().any(|c| c.contains("Main")), "{callers:?}");
    assert!(!server.workspace_symbols("Helper").await.unwrap().is_empty());
    assert!(server.diagnostics(&file).await.unwrap().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn python_installs_and_starts() {
    if !wanted("python") {
        return;
    }
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("pyproject.toml"),
        "[project]\nname = \"p\"\nversion = \"0\"\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("a.py"),
        "def helper() -> int:\n    return 1\n\nx = helper()\n",
    )
    .unwrap();
    install(&setup, project.path(), "python").await;

    let launcher = LspLauncher::new(servers(&setup));
    let server = launcher
        .start(
            project.path(),
            Tree::Immutable,
            &config::pyright(),
            AnalysisPolicy { allow_executing: false },
        )
        .await
        .expect("the installed server starts and has every required capability");
    let at = At {
        file: path("a.py"),
        pos: Pos { line: 3, col: 5 },
    };
    assert!(server.hover(&at).await.unwrap().is_some());
    assert_eq!(server.definition(&at).await.unwrap().len(), 1);
    server.shutdown().await;
}
