//! Installing the pinned servers into a data directory and starting them
//! through the resolver. It downloads packages, so its tests are ignored and run on
//! request: `cargo nextest run -p vet-infra --run-ignored ignored-only servers::`.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use tempfile::TempDir;
use vet_app::ports::{ServerOrigin, ServerResolver, ServerState, Tsserver};
use vet_app::servers::Servers;
use vet_core::content::RepoPath;
use vet_infra::lsp::{LspLauncher, NoRestore, config};
use vet_infra::restore::ToolRestorer;
use vet_infra::servers::ToolResolver;
use vet_infra::store::SqliteStore;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{At, BackendId, LanguageServer, Pos, Restorer, ServerLauncher, Tree, Unwatched};

fn path(p: &str) -> RepoPath {
    RepoPath(p.as_bytes().to_vec())
}

struct Setup {
    data: TempDir,
    store: SqliteStore,
}

async fn setup() -> Setup {
    let data = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(data.path()).await.unwrap();
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
    let ServerState::Found(found) = resolver.resolve(&id, project, None, AnalysisPolicy::EXECUTING).await else {
        panic!("not found after the install");
    };
    assert_eq!(found.origin, ServerOrigin::Installed);
    assert_eq!(found.version, resolver.pinned(&id), "{found:?}");
    lines.into_inner().unwrap()
}

#[tokio::test]
#[ignore = "downloads packages"]
async fn typescript_installs_and_uses_the_project_typescript() {
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    let src = "export function helper(): number {\n  return 1;\n}\nexport const x: number = helper();\nexport interface Greeter {\n  greet(): string;\n}\nexport class Hello implements Greeter {\n  greet(): string {\n    return 'hi';\n  }\n}\n";
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
        .resolve(
            &BackendId("typescript".into()),
            project.path(),
            None,
            AnalysisPolicy::EXECUTING,
        )
        .await
    else {
        panic!("not found");
    };
    assert!(
        matches!(&found.tsserver, Some(Tsserver::Using { whose, .. }) if whose.contains("Vet's fallback")),
        "{:?}",
        found.tsserver
    );

    let launcher = LspLauncher::new(servers(&setup), NoRestore);
    let server = launcher
        .start(
            project.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("the installed server starts and has every required capability")
        .server;
    let at = At {
        file: path("a.ts"),
        pos: Pos { line: 3, col: 25 },
    };
    assert!(server.hover(&at).await.unwrap().is_some());
    assert_eq!(server.definition(&at).await.unwrap().len(), 1);
    let interface_method = At {
        file: path("a.ts"),
        pos: Pos { line: 5, col: 3 },
    };
    let implementations = server.implementations(&interface_method).await.unwrap();
    assert_eq!(implementations.len(), 1, "{implementations:?}");
    assert_eq!(implementations[0].span.start.line, 8);
    server.shutdown().await;
}

/// The server hands `tsserver.js` to node, so a project that commits one decides what runs.
#[tokio::test]
#[ignore = "needs typescript-language-server"]
async fn the_projects_tsserver_runs_only_where_analysis_may_execute_project_code() {
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    let marker = project.path().join("marker");
    let tsserver = project.path().join("node_modules/typescript/lib/tsserver.js");
    std::fs::create_dir_all(tsserver.parent().unwrap()).unwrap();
    std::fs::write(
        &tsserver,
        format!("require('fs').writeFileSync({:?}, 'ran');\n", marker.to_string_lossy()),
    )
    .unwrap();
    std::fs::write(
        tsserver.parent().unwrap().join("../package.json"),
        r#"{"name":"typescript","version":"5.9.2"}"#,
    )
    .unwrap();
    std::fs::write(project.path().join("tsconfig.json"), "{}").unwrap();
    std::fs::write(project.path().join("a.ts"), "export const x = 1;\n").unwrap();

    let launcher = LspLauncher::new(servers(&setup), NoRestore);
    // The typescript beside the server on PATH serves, or none does and the start fails.
    let _ = launcher
        .start(
            project.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await;
    assert!(!marker.exists(), "the project's tsserver ran without trust");

    // Trusted, the project's own typescript is the one the server answers through.
    // The script is no tsserver, so the start fails after it ran.
    let _ = launcher
        .start(
            project.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy::EXECUTING,
            &Unwatched,
        )
        .await;
    assert!(marker.exists(), "the project's tsserver did not run when trusted");
}

/// The Angular server loads `@angular/language-service` from the probe locations it is given.
#[tokio::test]
#[ignore = "downloads packages"]
async fn the_projects_angular_language_service_runs_only_where_analysis_may_execute_project_code() {
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    let marker = project.path().join("marker");
    let service = project.path().join("node_modules/@angular/language-service");
    std::fs::create_dir_all(&service).unwrap();
    std::fs::write(
        service.join("index.js"),
        format!("require('fs').writeFileSync({:?}, 'ran');\n", marker.to_string_lossy()),
    )
    .unwrap();
    std::fs::write(
        service.join("package.json"),
        r#"{"name":"@angular/language-service","version":"22.2.2","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(project.path().join("angular.json"), "{}").unwrap();
    std::fs::write(project.path().join("tsconfig.json"), "{}").unwrap();
    std::fs::write(project.path().join("a.html"), "<p>{{ x }}</p>\n").unwrap();
    ToolResolver::new(setup.data.path().to_owned())
        .install(&BackendId("angular".into()), &|_| {})
        .await
        .expect("installs");

    let launcher = LspLauncher::new(servers(&setup), NoRestore);
    // The service is loaded when a template is asked about; the project's one is no service, so
    // the answers do not matter.
    for (policy, runs) in [(AnalysisPolicy::LIMITED, false), (AnalysisPolicy::EXECUTING, true)] {
        let started = launcher
            .start(project.path(), Tree::Immutable, &config::angular(), policy, &Unwatched)
            .await;
        if let Ok(started) = started {
            let _ = started.server.document_symbols(&path("a.html")).await;
            started.server.shutdown().await;
        }
        assert_eq!(marker.exists(), runs, "the project's language service under {policy:?}");
    }
}

#[tokio::test]
#[ignore = "downloads packages"]
async fn csharp_installs_and_starts() {
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    let program = "namespace App;\n\npublic static class Helper\n{\n    public static int One() => 1;\n}\n\npublic static class Program\n{\n    public static int Main() => Helper.One();\n}\n\npublic interface IShape\n{\n    int Area();\n}\n\npublic class Square : IShape\n{\n    public int Area() => 4;\n}\n";
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

    let launcher = LspLauncher::new(servers(&setup), NoRestore);
    let server = launcher
        .start(
            project.path(),
            Tree::Live,
            &config::roslyn(),
            AnalysisPolicy::EXECUTING,
            &Unwatched,
        )
        .await
        .expect("the installed server starts and has every required capability")
        .server;
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
    // Lines: IShape.Area is on 14, Square.Area on 19.
    let interface_method = At {
        file: file.clone(),
        pos: Pos { line: 14, col: 9 },
    };
    let implementations = server.implementations(&interface_method).await.unwrap();
    assert_eq!(implementations.len(), 1, "{implementations:?}");
    assert_eq!(implementations[0].span.start.line, 19);
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

    // Other text under the file's name, as for a base side.
    let base = "namespace App;\n\npublic static class Gone\n{\n    public static int Two() => 2;\n}\n";
    let names: Vec<String> = server
        .text_symbols(&file, base)
        .await
        .unwrap()
        .iter()
        .map(|s| s.sym.name.join("::"))
        .collect();
    assert!(names.iter().any(|n| n.contains("Gone")), "{names:?}");
    assert!(server.diagnostics(&file).await.unwrap().is_empty());
    server.shutdown().await;
}

#[tokio::test]
#[ignore = "downloads packages"]
async fn csharp_resolves_packages_in_a_copy_that_took_the_working_trees_restore_output() {
    let setup = setup().await;
    let (donor, copy) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let project = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n    <OutputType>Library</OutputType>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.3\" />\n  </ItemGroup>\n</Project>\n";
    let program = "using Newtonsoft.Json;\n\npublic static class Program\n{\n    public static string Main() => JsonConvert.SerializeObject(1);\n}\n";
    for dir in [donor.path(), copy.path()] {
        std::fs::write(dir.join("App.csproj"), project).unwrap();
    }
    std::fs::write(donor.path().join("Program.cs"), format!("{program}// working tree\n")).unwrap();
    std::fs::write(copy.path().join("Program.cs"), program).unwrap();
    install(&setup, donor.path(), "csharp").await;

    let restorer = ToolRestorer::new().reusing(donor.path().to_owned());
    let policy = AnalysisPolicy::EXECUTING;
    let backend = config::roslyn();
    for dir in [donor.path(), copy.path()] {
        let restored = restorer.restore(dir, &backend, policy).await;
        assert!(restored.failures.is_empty(), "{:?}", restored.failures);
    }
    assert!(copy.path().join("obj/project.assets.json").is_file());

    let launcher = LspLauncher::new(servers(&setup), NoRestore);
    let server = launcher
        .start(copy.path(), Tree::Immutable, &backend, policy, &Unwatched)
        .await
        .expect("the server loads the copy")
        .server;
    let call = At {
        file: path("Program.cs"),
        pos: Pos { line: 4, col: 50 },
    };
    let hover = server.hover(&call).await.unwrap().expect("a package member resolves");
    assert!(hover.contains("SerializeObject"), "{hover}");
    assert_eq!(server.definition(&call).await.unwrap().len(), 1);
    server.shutdown().await;
}

#[tokio::test]
#[ignore = "downloads packages"]
async fn python_installs_and_starts() {
    let setup = setup().await;
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("pyproject.toml"),
        "[project]\nname = \"p\"\nversion = \"0\"\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("a.py"),
        "def helper() -> int:\n    return 1\n\nx = helper()\n\nclass Base:\n    def run(self) -> int:\n        raise NotImplementedError\n\nclass Child(Base):\n    def run(self) -> int:\n        return 2\n",
    )
    .unwrap();
    install(&setup, project.path(), "python").await;

    let launcher = LspLauncher::new(servers(&setup), NoRestore);
    let server = launcher
        .start(
            project.path(),
            Tree::Immutable,
            &config::pyright(),
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("the installed server starts and has every required capability")
        .server;
    let at = At {
        file: path("a.py"),
        pos: Pos { line: 3, col: 5 },
    };
    assert!(server.hover(&at).await.unwrap().is_some());
    assert_eq!(server.definition(&at).await.unwrap().len(), 1);
    let base_method = At {
        file: path("a.py"),
        pos: Pos { line: 6, col: 9 },
    };
    // Pyright has no implementation request: the query fails and says so.
    let error = server.implementations(&base_method).await.unwrap_err();
    assert!(error.0.contains("implementation"), "{error:?}");

    // Other text under the file's name, as for a base side.
    let names: Vec<String> = server
        .text_symbols(&path("a.py"), "def gone() -> int:\n    return 2\n")
        .await
        .unwrap()
        .iter()
        .map(|s| s.sym.name.join("::"))
        .collect();
    assert_eq!(names, ["gone"]);
    server.shutdown().await;
}
