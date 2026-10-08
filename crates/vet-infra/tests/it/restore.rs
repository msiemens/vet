//! Restores with the real tools. They download packages, so a test only runs
//! when `VET_TEST_INSTALL` lists `restore`, and only if its tool is installed.
//! Caches are redirected to temporary directories with each tool's documented
//! variable where there is one, so the user's own caches are left alone.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use vet_core::content::RepoPath;
use vet_infra::exec::{Host, is_file};
use vet_infra::lsp::{BinaryLocator, LspLauncher, NoRestore, config};
use vet_infra::restore::ToolRestorer;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{At, Backend, LanguageServer, Pos, Restored, Restorer, ServerLauncher, Tree};

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
async fn a_copy_with_the_same_lockfile_links_the_working_trees_node_modules_and_resolves_into_them() {
    if !wanted("npm") || Host::current().find("typescript-language-server", is_file).is_none() {
        return;
    }
    let (donor, copy, cache) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let package = r#"{"name":"p","version":"1.0.0","dependencies":{"picocolors":"1.1.1"}}"#;
    write(donor.path(), "package.json", package);
    run(
        donor.path(),
        "npm",
        &["install", "--package-lock-only", "--ignore-scripts"],
    );
    let restorer = ToolRestorer::with_env(env("npm_config_cache", cache.path()));
    restore(&restorer, donor.path(), &config::typescript(), false).await;

    // The copy has other sources, and the same lockfile and dependencies.
    let source = "import pc from \"picocolors\";\nexport const x = pc.red(\"a\");\n";
    write(copy.path(), "package.json", package);
    std::fs::copy(
        donor.path().join("package-lock.json"),
        copy.path().join("package-lock.json"),
    )
    .unwrap();
    write(
        copy.path(),
        "tsconfig.json",
        "{\"compilerOptions\":{\"strict\":true,\"moduleResolution\":\"node\",\"esModuleInterop\":true}}",
    );
    write(copy.path(), "a.ts", source);

    // Not installed here: a cache that does not exist would make an install fail.
    let missing = tempfile::tempdir().unwrap().path().join("none");
    let reusing = ToolRestorer::with_env(env("npm_config_cache", &missing)).reusing(donor.path().to_owned());
    let restored = restore(&reusing, copy.path(), &config::typescript(), false).await;
    assert_eq!(restored.ran, [(".".to_owned(), "npm".to_owned())]);
    let linked = copy.path().join("node_modules");
    assert!(linked.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(linked.join("picocolors/package.json").is_file());
    assert!(!missing.exists());

    let server = LspLauncher::new(BinaryLocator, NoRestore)
        .start(
            copy.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy { allow_executing: false },
        )
        .await
        .expect("typescript-language-server starts")
        .server;
    let at = At {
        file: RepoPath(b"a.ts".to_vec()),
        pos: Pos { line: 1, col: 22 },
    };
    let hover = server
        .hover(&at)
        .await
        .unwrap()
        .expect("pc.red resolves through node_modules");
    assert!(hover.contains("red"), "{hover}");
    server.shutdown().await;

    // Another lockfile: the install runs, here failing on the lockfile that does not match `package.json`.
    write(
        copy.path(),
        "package-lock.json",
        "{\"lockfileVersion\":3,\"packages\":{}}",
    );
    std::fs::remove_file(&linked).unwrap();
    let reusing = ToolRestorer::with_env(env("npm_config_cache", cache.path())).reusing(donor.path().to_owned());
    let restored = reusing
        .restore(
            copy.path(),
            &config::typescript(),
            AnalysisPolicy { allow_executing: false },
        )
        .await;
    assert_eq!(restored.failures.len(), 1, "{restored:?}");
    assert_eq!(restored.failures[0].tool, "npm");
    assert!(!linked.exists());
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
    // pnpm would run these hooks during the install.
    write(
        project.path(),
        ".pnpmfile.cjs",
        "require('fs').writeFileSync('pnpmfile-ran', '');\nmodule.exports = { hooks: {} };\n",
    );

    let restorer = ToolRestorer::with_env(env("pnpm_config_store_dir", store.path()));
    restore(&restorer, project.path(), &config::typescript(), false).await;
    assert!(project.path().join("node_modules/picocolors/package.json").is_file());
    assert!(!project.path().join("script-ran").exists());
    assert!(!project.path().join("pnpmfile-ran").exists());
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
async fn npm_restores_a_workspace_once_at_its_root_and_a_project_in_a_subdirectory_on_its_own() {
    if !wanted("npm") {
        return;
    }
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let package = r#"{"name":"NAME","version":"1.0.0","dependencies":{"picocolors":"1.1.1"}}"#;
    write(
        project.path(),
        "package.json",
        r#"{"name":"root","version":"1.0.0","private":true,"workspaces":["packages/*"]}"#,
    );
    for dir in ["packages/a", "web"] {
        std::fs::create_dir_all(project.path().join(dir)).unwrap();
        write(
            project.path(),
            &format!("{dir}/package.json"),
            &package.replace("NAME", dir.replace('/', "-").as_str()),
        );
    }
    run(
        project.path(),
        "npm",
        &["install", "--package-lock-only", "--ignore-scripts"],
    );
    run(
        &project.path().join("web"),
        "npm",
        &["install", "--package-lock-only", "--ignore-scripts"],
    );

    let restorer = ToolRestorer::with_env(env("npm_config_cache", cache.path()));
    restore(&restorer, project.path(), &config::typescript(), false).await;
    // The workspace root's install hoists the member's dependency; the member is not installed on its own.
    assert!(project.path().join("node_modules/picocolors/package.json").is_file());
    assert!(!project.path().join("packages/a/node_modules").exists());
    assert!(
        project
            .path()
            .join("web/node_modules/picocolors/package.json")
            .is_file()
    );
}

#[tokio::test]
async fn yarn_classic_installs_from_the_lockfile_without_running_scripts() {
    if !wanted("yarn") {
        return;
    }
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "package.json",
        r#"{"name":"p","version":"1.0.0","dependencies":{"picocolors":"1.1.1"},"scripts":{"postinstall":"node -e \"require('fs').writeFileSync('script-ran','')\""}}"#,
    );
    let cache_env = env("YARN_CACHE_FOLDER", cache.path());
    let restorer = ToolRestorer::with_env(cache_env.clone());
    // Writes the lockfile; the install itself is thrown away.
    Command::new("yarn")
        .args(["install", "--ignore-scripts", "--non-interactive"])
        .envs(cache_env)
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(project.path().join("yarn.lock").is_file());
    std::fs::remove_dir_all(project.path().join("node_modules")).unwrap();

    restore(&restorer, project.path(), &config::typescript(), false).await;
    assert!(project.path().join("node_modules/picocolors/package.json").is_file());
    assert!(!project.path().join("script-ran").exists());
}

#[tokio::test]
async fn uv_installs_requirements_txt_into_a_venv_and_never_builds_a_source_package() {
    if !wanted("uv") {
        return;
    }
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let restorer = ToolRestorer::with_env(env("UV_CACHE_DIR", cache.path()));
    write(project.path(), "requirements.txt", "iniconfig==2.1.0\n");
    let restored = restore(&restorer, project.path(), &config::pyright(), false).await;
    let venv = project.path().join(".venv");
    let value = |key: &str| restored.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    assert_eq!(value("VIRTUAL_ENV").as_deref(), venv.to_str());
    let python = venv.join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    });
    run(project.path(), python.to_str().unwrap(), &["-c", "import iniconfig"]);

    // A local source package needs its build backend, which does not run: the failure is reported.
    let local = tempfile::tempdir().unwrap();
    write(
        local.path(),
        "setup.py",
        "open('build-ran', 'w').close()\nfrom setuptools import setup\nsetup(name='local', version='0')\n",
    );
    write(
        local.path(),
        "requirements.txt",
        &format!("{}\n", local.path().display()),
    );
    let restored = restorer
        .restore(
            local.path(),
            &config::pyright(),
            AnalysisPolicy { allow_executing: false },
        )
        .await;
    assert_eq!(restored.failures.len(), 1, "{restored:?}");
    assert_eq!(restored.failures[0].tool, "uv");
    assert!(!local.path().join("build-ran").exists());
}

#[tokio::test]
async fn uv_installs_a_pyproject_without_a_lockfile_from_its_subdirectory() {
    if !wanted("uv") {
        return;
    }
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(project.path().join("svc")).unwrap();
    write(
        &project.path().join("svc"),
        "pyproject.toml",
        "[project]\nname = \"p\"\nversion = \"0\"\nrequires-python = \">=3.9\"\ndependencies = [\"iniconfig==2.1.0\"]\n",
    );
    let restorer = ToolRestorer::with_env(env("UV_CACHE_DIR", cache.path()));
    let restored = restore(&restorer, project.path(), &config::pyright(), false).await;
    let venv = project.path().join("svc/.venv");
    assert!(venv.is_dir());
    assert_eq!(
        restored
            .env
            .iter()
            .find(|(k, _)| k == "VIRTUAL_ENV")
            .map(|(_, v)| v.as_str()),
        venv.to_str()
    );
}

#[tokio::test]
async fn a_copy_with_the_same_projects_takes_the_working_trees_restore_output() {
    if !wanted("dotnet") {
        return;
    }
    let (donor, copy, packages) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let project = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.3\" />\n  </ItemGroup>\n  <Target Name=\"Mark\" BeforeTargets=\"Restore\">\n    <WriteLinesToFile File=\"$(MSBuildProjectDirectory)/restore-ran\" Lines=\"x\" />\n  </Target>\n</Project>\n";
    let nuget = "<configuration>\n  <packageSources>\n    <clear />\n    <add key=\"nuget.org\" value=\"https://api.nuget.org/v3/index.json\" />\n  </packageSources>\n</configuration>\n";
    for dir in [donor.path(), copy.path()] {
        write(dir, "App.csproj", project);
        write(dir, "nuget.config", nuget);
    }
    write(donor.path(), "Program.cs", "// working tree");
    write(copy.path(), "Program.cs", "// base");

    let restorer = ToolRestorer::with_env(env("NUGET_PACKAGES", packages.path())).reusing(donor.path().to_owned());
    restore(&restorer, donor.path(), &config::roslyn(), true).await;
    assert!(donor.path().join("restore-ran").exists());

    // The copy's restore is not run: the project's own targets did not execute.
    restore(&restorer, copy.path(), &config::roslyn(), true).await;
    assert!(copy.path().join("obj/project.assets.json").is_file());
    assert!(!copy.path().join("restore-ran").exists());

    // Another package version: restored.
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "App.csproj", &project.replace("13.0.3", "13.0.2"));
    write(other.path(), "nuget.config", nuget);
    restore(&restorer, other.path(), &config::roslyn(), true).await;
    assert!(other.path().join("restore-ran").exists());
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
