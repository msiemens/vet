//! The base side of a review is a materialized snapshot with no dependencies:
//! its server resolves into them only because the snapshot was restored first.
//! Except for the first, these tests use the real tools and download packages, so they
//! are ignored and run on request, with the tools installed:
//! `cargo nextest run -p vet-backend --run-ignored ignored-only restore::`. The pinned
//! servers are installed into a temporary data directory; the package caches are the
//! user's own, which is the sharing a restore is meant to have.

use std::path::Path;

use crate::common::{Client, find, git, handler, repo_with, run, write};
use vet_protocol::{Event, Locations, Request, Response, ServerMessage, Side};

fn commit(repo: &Path) {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", "base"]);
}

fn only(locations: &Locations) -> &str {
    assert_eq!(locations.locations.len(), 1, "{locations:?}");
    &locations.locations[0].path
}

/// A restore that fails downloads nothing, so this runs everywhere.
#[tokio::test]
async fn a_failed_restore_limits_the_analysis_and_says_which_project() {
    const LIB: &str = "pub fn show() {}\n";
    let repo = repo_with(
        &[("svc/Cargo.toml", "not a manifest"), ("svc/src/lib.rs", LIB)],
        &[("svc/src/lib.rs", "pub fn render() {}\n")],
    );
    let (handler, _data) = handler().await;
    let mut client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    let revision = client.revision(project).await;
    async fn manifest(client: &Client, project: u32) -> vet_protocol::Manifest {
        match client
            .call(Request::Manifest {
                project,
                view: vet_protocol::View::Changeset,
            })
            .await
            .unwrap()
        {
            Response::Manifest(m) => *m,
            other => panic!("{other:?}"),
        }
    }
    assert!(manifest(&client, project).await.restore_failures.is_empty());

    // The base side starts a server on a restored snapshot, whether or not rust-analyzer is there.
    client
        .hover(project, revision, Side::Base, "svc/src/lib.rs", find(LIB, "show", 0, 1))
        .await;
    let failures = manifest(&client, project).await.restore_failures;
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(
        (failures[0].project.as_str(), failures[0].tool.as_str()),
        ("svc", "cargo")
    );
    assert!(failures[0].message.contains("Cargo.toml"), "{failures:?}");

    let pushed = std::iter::from_fn(|| client.inbox.try_recv().ok()).find_map(|m| match m {
        ServerMessage::Event {
            event: Event::RestoreFailed { project: s, failures },
        } if s == project => Some(failures),
        _ => None,
    });
    assert_eq!(pushed, Some(failures));
}

#[tokio::test]
#[ignore = "downloads packages; needs rust-analyzer"]
async fn rust_base_side_resolves_into_a_registry_crate() {
    const BASE: &str = "pub fn show(n: u32) -> String {\n    let mut buffer = itoa::Buffer::new();\n    buffer.format(n).to_owned()\n}\n";
    let repo = repo_with(&[], &[]);
    write(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nitoa = \"1\"\n",
    );
    write(repo.path(), "src/lib.rs", BASE);
    run(repo.path(), "cargo", &["generate-lockfile"]);
    commit(repo.path());
    write(repo.path(), "src/lib.rs", &BASE.replace("show", "render"));

    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    let revision = client.revision(project).await;

    let at = find(BASE, "Buffer", 0, 1);
    let defs = client.definition(project, revision, Side::Base, "src/lib.rs", at).await;
    assert!(only(&defs).contains("itoa"), "{defs:?}");
    assert!(defs.locations[0].external);
    let (text, unavailable) = client.hover(project, revision, Side::Base, "src/lib.rs", at).await;
    assert_eq!(unavailable, None);
    assert!(text.unwrap().contains("Buffer"));
}

#[tokio::test]
#[ignore = "downloads packages; needs npm and node"]
async fn typescript_base_side_resolves_into_node_modules() {
    const BASE: &str = "import pc from \"picocolors\";\n\nexport const label: string = pc.red(\"x\");\n";
    let repo = repo_with(&[], &[]);
    write(
        repo.path(),
        "package.json",
        "{\"name\":\"p\",\"version\":\"1.0.0\",\"dependencies\":{\"picocolors\":\"1.1.1\"}}\n",
    );
    write(
        repo.path(),
        "tsconfig.json",
        "{\"compilerOptions\":{\"strict\":true,\"esModuleInterop\":true,\"module\":\"commonjs\",\"moduleResolution\":\"node10\"}}\n",
    );
    write(repo.path(), ".gitignore", "node_modules/\n");
    write(repo.path(), "a.ts", BASE);
    run(
        repo.path(),
        "npm",
        &["install", "--package-lock-only", "--ignore-scripts"],
    );
    commit(repo.path());
    write(repo.path(), "a.ts", &BASE.replace("label", "title"));

    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client.install(project, "typescript").await;
    let revision = client.revision(project).await;

    // `red` is only known once picocolors is installed in the base snapshot.
    let at = find(BASE, "red", 0, 1);
    let defs = client.definition(project, revision, Side::Base, "a.ts", at).await;
    assert!(only(&defs).starts_with("node_modules/picocolors/"), "{defs:?}");
    let (text, _) = client.hover(project, revision, Side::Base, "a.ts", at).await;
    assert!(text.unwrap().contains("red"));
    // The user's own tree has no node_modules: the target side is not restored.
    let defs = client.definition(project, revision, Side::Target, "a.ts", at).await;
    assert!(defs.locations.is_empty(), "{defs:?}");
}

#[tokio::test]
#[ignore = "downloads packages; needs uv, npm and python3"]
async fn python_base_side_resolves_into_the_restored_environment() {
    const BASE: &str = "import iniconfig\n\nconfig = iniconfig.IniConfig(\"x.ini\", data=\"[s]\\nk = v\\n\")\n";
    let repo = repo_with(&[], &[]);
    write(repo.path(), ".gitignore", ".venv/\n");
    write(
        repo.path(),
        "pyproject.toml",
        "[project]\nname = \"p\"\nversion = \"0\"\nrequires-python = \">=3.9\"\ndependencies = [\"iniconfig==2.1.0\"]\n",
    );
    write(repo.path(), "a.py", BASE);
    run(repo.path(), "uv", &["lock"]);
    commit(repo.path());
    write(repo.path(), "a.py", &BASE.replace("config", "settings"));

    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client.install(project, "python").await;
    let revision = client.revision(project).await;

    let at = find(BASE, "IniConfig", 0, 1);
    let defs = client.definition(project, revision, Side::Base, "a.py", at).await;
    assert!(only(&defs).contains("iniconfig"), "{defs:?}");
    let (text, _) = client.hover(project, revision, Side::Base, "a.py", at).await;
    assert!(text.unwrap().contains("IniConfig"));
}

#[tokio::test]
#[ignore = "downloads packages; needs dotnet"]
async fn csharp_base_side_resolves_into_a_nuget_package() {
    const BASE: &str = "namespace App;\n\npublic static class Program\n{\n    public static string Main() => Newtonsoft.Json.JsonConvert.SerializeObject(1);\n}\n";
    let repo = repo_with(&[], &[]);
    write(repo.path(), ".gitignore", "bin/\nobj/\n");
    write(
        repo.path(),
        "App.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n    <OutputType>Library</OutputType>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.3\" />\n  </ItemGroup>\n</Project>\n",
    );
    // Only nuget.org, whatever feeds the user's own configuration lists.
    write(
        repo.path(),
        "nuget.config",
        "<configuration>\n  <packageSources>\n    <clear />\n    <add key=\"nuget.org\" value=\"https://api.nuget.org/v3/index.json\" />\n  </packageSources>\n</configuration>\n",
    );
    write(repo.path(), "Program.cs", BASE);
    commit(repo.path());
    write(repo.path(), "Program.cs", &BASE.replace("Main", "Run"));

    let (handler, _data) = handler().await;
    let client = Client::new(&handler);
    let project = client.open(repo.path()).await;
    client.install(project, "csharp").await;
    let revision = client.revision(project).await;

    let at = find(BASE, "SerializeObject", 0, 1);
    let (text, unavailable) = client.hover(project, revision, Side::Base, "Program.cs", at).await;
    assert_eq!(unavailable, None);
    assert!(text.unwrap().contains("SerializeObject"));
}
