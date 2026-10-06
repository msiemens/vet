//! The base side of a review is a materialized snapshot with no dependencies:
//! its server resolves into them only because the snapshot was restored first.
//! These tests use the real tools and download packages, so they run only when
//! `VET_TEST_INSTALL` lists `restore`, and each only if its tools are installed.
//! Run them alone (`--test-threads=1`). The pinned servers are installed into a
//! temporary data directory; the package caches are the user's own, which is
//! the sharing a restore is meant to have.

mod common;

use std::path::Path;

use common::{Client, find, git, handler, repo_with, run, write};
use vet_infra::exec::{Host, is_file};
use vet_protocol::{Locations, Side};

fn wanted(tools: &[&str]) -> bool {
    let listed = std::env::var("VET_TEST_INSTALL").is_ok_and(|l| l.split(',').any(|x| x.trim() == "restore"));
    if !listed {
        eprintln!("skipped: set VET_TEST_INSTALL=restore to download packages and servers");
        return false;
    }
    if let Some(missing) = tools.iter().find(|t| Host::current().find(t, is_file).is_none()) {
        eprintln!("skipped: {missing} is not installed");
        return false;
    }
    true
}

fn commit(repo: &Path) {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", "base"]);
}

fn only(locations: &Locations) -> &str {
    assert_eq!(locations.locations.len(), 1, "{locations:?}");
    &locations.locations[0].path
}

#[tokio::test]
async fn rust_base_side_resolves_into_a_registry_crate() {
    if !wanted(&["cargo", "rust-analyzer"]) {
        return;
    }
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
    let session = client.open(repo.path()).await;
    let revision = client.revision(session).await;

    let at = find(BASE, "Buffer", 0, 1);
    let defs = client.definition(session, revision, Side::Base, "src/lib.rs", at).await;
    assert!(only(&defs).contains("itoa"), "{defs:?}");
    assert!(defs.locations[0].external);
    let (text, unavailable) = client.hover(session, revision, Side::Base, "src/lib.rs", at).await;
    assert_eq!(unavailable, None);
    assert!(text.unwrap().contains("Buffer"));
}

#[tokio::test]
async fn typescript_base_side_resolves_into_node_modules() {
    if !wanted(&["npm", "node"]) {
        return;
    }
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
    let session = client.open(repo.path()).await;
    client.install(session, "typescript").await;
    let revision = client.revision(session).await;

    // `red` is only known once picocolors is installed in the base snapshot.
    let at = find(BASE, "red", 0, 1);
    let defs = client.definition(session, revision, Side::Base, "a.ts", at).await;
    assert!(only(&defs).starts_with("node_modules/picocolors/"), "{defs:?}");
    let (text, _) = client.hover(session, revision, Side::Base, "a.ts", at).await;
    assert!(text.unwrap().contains("red"));
    // The user's own tree has no node_modules: the target side is not restored.
    let defs = client.definition(session, revision, Side::Target, "a.ts", at).await;
    assert!(defs.locations.is_empty(), "{defs:?}");
}

#[tokio::test]
async fn python_base_side_resolves_into_the_restored_environment() {
    if !wanted(&["uv", "npm", "python3"]) {
        return;
    }
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
    let session = client.open(repo.path()).await;
    client.install(session, "python").await;
    let revision = client.revision(session).await;

    let at = find(BASE, "IniConfig", 0, 1);
    let defs = client.definition(session, revision, Side::Base, "a.py", at).await;
    assert!(only(&defs).contains("iniconfig"), "{defs:?}");
    let (text, _) = client.hover(session, revision, Side::Base, "a.py", at).await;
    assert!(text.unwrap().contains("IniConfig"));
}

#[tokio::test]
async fn csharp_base_side_resolves_into_a_nuget_package() {
    if !wanted(&["dotnet"]) {
        return;
    }
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
    let session = client.open(repo.path()).await;
    client.install(session, "csharp").await;
    let revision = client.revision(session).await;

    let at = find(BASE, "SerializeObject", 0, 1);
    let (text, unavailable) = client.hover(session, revision, Side::Base, "Program.cs", at).await;
    assert_eq!(unavailable, None);
    assert!(text.unwrap().contains("SerializeObject"));
}
