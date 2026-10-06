//! Restores with the real tools. They download packages, so a test only runs
//! when `VET_TEST_INSTALL` lists `restore`, and only if its tool is installed.
//! Caches are redirected to temporary directories with each tool's documented
//! variable where there is one, so the user's own caches are left alone.

use std::path::Path;
use std::process::Command;

use vet_infra::exec::{Host, is_file};
use vet_infra::lsp::config;
use vet_infra::restore::ToolRestorer;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Backend, Restored, Restorer};

fn wanted(tool: &str) -> bool {
    let listed = std::env::var("VET_TEST_INSTALL").is_ok_and(|l| l.split(',').any(|x| x.trim() == "restore"));
    if !listed {
        eprintln!("skipped: set VET_TEST_INSTALL=restore to download packages");
        return false;
    }
    if Host::current().find(tool, is_file).is_none() {
        eprintln!("skipped: {tool} is not installed");
        return false;
    }
    true
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

fn run(dir: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program).args(args).current_dir(dir).output().unwrap();
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn env(key: &str, dir: &Path) -> Vec<(String, String)> {
    vec![(key.into(), dir.to_string_lossy().into_owned())]
}

async fn restore(restorer: &ToolRestorer, root: &Path, backend: &Backend, allow_executing: bool) -> Restored {
    let restored = restorer
        .restore(root, backend, AnalysisPolicy { allow_executing })
        .await;
    assert!(restored.failures.is_empty(), "{:?}", restored.failures);
    restored
}

#[tokio::test]
async fn cargo_fetches_into_cargo_home_without_running_build_scripts() {
    if !wanted("cargo") {
        return;
    }
    let (project, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "Cargo.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nitoa = \"1\"\n",
    );
    std::fs::create_dir(project.path().join("src")).unwrap();
    write(project.path(), "src/lib.rs", "");
    write(
        project.path(),
        "build.rs",
        "fn main() { std::fs::write(\"build-ran\", \"\").unwrap(); }\n",
    );

    let restorer = ToolRestorer::with_env(env("CARGO_HOME", home.path()));
    restore(&restorer, project.path(), &config::rust_analyzer(), false).await;
    assert!(!project.path().join("build-ran").exists());
    // Everything is in Cargo home now: an offline fetch succeeds there and fails in an empty one.
    let offline = |cargo_home: &Path| {
        Command::new("cargo")
            .args(["fetch", "--offline"])
            .env("CARGO_HOME", cargo_home)
            .current_dir(project.path())
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(offline(home.path()));
    assert!(!offline(tempfile::tempdir().unwrap().path()));
}

#[tokio::test]
async fn npm_installs_from_the_lockfile_without_running_scripts() {
    if !wanted("npm") {
        return;
    }
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "package.json",
        r#"{"name":"p","version":"1.0.0","dependencies":{"picocolors":"1.1.1"},"scripts":{"postinstall":"node -e \"require('fs').writeFileSync('script-ran','')\""}}"#,
    );
    run(
        project.path(),
        "npm",
        &["install", "--package-lock-only", "--ignore-scripts"],
    );
    assert!(project.path().join("package-lock.json").is_file());

    let restorer = ToolRestorer::with_env(env("npm_config_cache", cache.path()));
    restore(&restorer, project.path(), &config::typescript(), false).await;
    assert!(project.path().join("node_modules/picocolors/package.json").is_file());
    assert!(!project.path().join("script-ran").exists());
    // The npm cache took the download.
    assert!(cache.path().join("_cacache").is_dir());
}

#[tokio::test]
async fn pnpm_installs_from_the_lockfile_without_running_scripts() {
    if !wanted("pnpm") {
        return;
    }
    let (project, store) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "package.json",
        r#"{"name":"p","version":"1.0.0","dependencies":{"picocolors":"1.1.1"},"scripts":{"postinstall":"node -e \"require('fs').writeFileSync('script-ran','')\""}}"#,
    );
    run(
        project.path(),
        "pnpm",
        &["install", "--lockfile-only", "--ignore-scripts"],
    );
    assert!(project.path().join("pnpm-lock.yaml").is_file());

    let restorer = ToolRestorer::with_env(env("pnpm_config_store_dir", store.path()));
    restore(&restorer, project.path(), &config::typescript(), false).await;
    assert!(project.path().join("node_modules/picocolors/package.json").is_file());
    assert!(!project.path().join("script-ran").exists());
    assert!(
        std::fs::read_dir(store.path()).unwrap().next().is_some(),
        "the store took the download"
    );
}

#[tokio::test]
async fn uv_installs_the_locked_dependencies_into_a_venv_the_server_gets() {
    if !wanted("uv") {
        return;
    }
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "pyproject.toml",
        "[project]\nname = \"p\"\nversion = \"0\"\nrequires-python = \">=3.9\"\ndependencies = [\"iniconfig==2.1.0\"]\n",
    );
    run(project.path(), "uv", &["lock"]);

    let restorer = ToolRestorer::with_env(env("UV_CACHE_DIR", cache.path()));
    let restored = restore(&restorer, project.path(), &config::pyright(), false).await;
    let venv = project.path().join(".venv");
    let value = |key: &str| restored.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    assert_eq!(value("VIRTUAL_ENV").as_deref(), venv.to_str());
    assert!(
        value("PATH").unwrap().starts_with(
            venv.join(if cfg!(windows) { "Scripts" } else { "bin" })
                .to_str()
                .unwrap()
        )
    );
    let python = venv.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    run(project.path(), python.to_str().unwrap(), &["-c", "import iniconfig"]);
    assert!(cache.path().is_dir() && std::fs::read_dir(cache.path()).unwrap().next().is_some());
}

#[tokio::test]
async fn dotnet_restores_only_where_executing_is_allowed() {
    if !wanted("dotnet") {
        return;
    }
    let (project, packages) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "App.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.3\" />\n  </ItemGroup>\n  <Target Name=\"Mark\" BeforeTargets=\"Restore\">\n    <WriteLinesToFile File=\"$(MSBuildProjectDirectory)/restore-ran\" Lines=\"x\" />\n  </Target>\n</Project>\n",
    );
    write(project.path(), "Program.cs", "");
    // Only nuget.org, whatever feeds the user's own configuration lists.
    write(
        project.path(),
        "nuget.config",
        "<configuration>\n  <packageSources>\n    <clear />\n    <add key=\"nuget.org\" value=\"https://api.nuget.org/v3/index.json\" />\n  </packageSources>\n</configuration>\n",
    );

    let restorer = ToolRestorer::with_env(env("NUGET_PACKAGES", packages.path()));
    let denied = restore(&restorer, project.path(), &config::roslyn(), false).await;
    assert_eq!(denied, Restored::default());
    assert!(!project.path().join("obj").exists());

    restore(&restorer, project.path(), &config::roslyn(), true).await;
    assert!(project.path().join("obj/project.assets.json").is_file());
    assert!(packages.path().join("newtonsoft.json").is_dir());
    // Why it needs the permission: a project's own targets run during restore.
    assert!(project.path().join("restore-ran").exists());
}
