//! Restores with the real tools. They download packages, so the tests are ignored and
//! run on request, with the tools installed:
//! `cargo nextest run -p vet-infra --run-ignored ignored-only restore::`.
//! Caches are redirected to temporary directories with each tool's documented
//! variable where there is one, so the user's own caches are left alone.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use vet_core::content::RepoPath;
use vet_infra::budget::Budgets;
use vet_infra::lsp::{BinaryLocator, LspLauncher, NoRestore, config};
use vet_infra::restore::ToolRestorer;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{
    At, Backend, LanguageServer, Pos, RestoreFailureKind, Restored, Restorer, ServerLauncher, Tree, Unwatched,
};

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
    let policy = if allow_executing {
        AnalysisPolicy::EXECUTING
    } else {
        AnalysisPolicy::LIMITED
    };
    let restored = restorer.restore(root, backend, policy).await;
    assert!(restored.failures.is_empty(), "{:?}", restored.failures);
    restored
}

#[tokio::test]
#[ignore = "downloads packages; needs cargo"]
async fn cargo_fetches_into_cargo_home_without_running_build_scripts() {
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

    let restorer = ToolRestorer::with_env(env("CARGO_HOME", home.path()), &Budgets::default());
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
#[ignore = "downloads packages; needs npm"]
async fn npm_installs_from_the_lockfile_without_running_scripts() {
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

    let restorer = ToolRestorer::with_env(env("npm_config_cache", cache.path()), &Budgets::default());
    restore(&restorer, project.path(), &config::typescript(), false).await;
    assert!(project.path().join("node_modules/picocolors/package.json").is_file());
    assert!(!project.path().join("script-ran").exists());
    // The npm cache took the download.
    assert!(cache.path().join("_cacache").is_dir());
}

/// A lock file that lacks a package `package.json` asks for: npm ci refuses, and the install
/// that stands in for it adds the package and rewrites nothing. Only local packages are used.
#[tokio::test]
#[ignore = "needs npm"]
async fn npm_installs_what_an_out_of_sync_lockfile_lacks_and_leaves_the_lockfile_alone() {
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    for name in ["a", "b"] {
        std::fs::create_dir(project.path().join(name)).unwrap();
        write(
            project.path(),
            &format!("{name}/package.json"),
            &format!(r#"{{"name":"{name}","version":"1.0.0"}}"#),
        );
    }
    write(
        project.path(),
        "package.json",
        r#"{"name":"p","version":"1.0.0","dependencies":{"a":"file:a"}}"#,
    );
    run(
        project.path(),
        "npm",
        &["install", "--package-lock-only", "--ignore-scripts"],
    );
    write(
        project.path(),
        "package.json",
        r#"{"name":"p","version":"1.0.0","dependencies":{"a":"file:a","b":"file:b"}}"#,
    );
    let locked = std::fs::read_to_string(project.path().join("package-lock.json")).unwrap();

    let restorer = ToolRestorer::with_env(
        vec![
            ("npm_config_cache".into(), cache.path().to_string_lossy().into_owned()),
            ("npm_config_offline".into(), "true".into()),
        ],
        &Budgets::default(),
    );
    let restored = restorer
        .restore(project.path(), &config::typescript(), AnalysisPolicy::LIMITED)
        .await;
    assert_eq!(restored.failures.len(), 1, "{:?}", restored.failures);
    assert!(
        restored.failures[0].message.contains("lock file out of sync"),
        "{:?}",
        restored.failures
    );
    for name in ["a", "b"] {
        assert!(
            project
                .path()
                .join("node_modules")
                .join(name)
                .join("package.json")
                .is_file()
        );
    }
    assert_eq!(
        std::fs::read_to_string(project.path().join("package-lock.json")).unwrap(),
        locked
    );
}

#[tokio::test]
#[ignore = "downloads packages; needs npm"]
async fn an_executing_npm_restore_runs_the_install_scripts() {
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

    let restorer = ToolRestorer::with_env(env("npm_config_cache", cache.path()), &Budgets::default());
    restore(&restorer, project.path(), &config::typescript(), true).await;
    assert!(project.path().join("node_modules/picocolors/package.json").is_file());
    assert!(project.path().join("script-ran").is_file());
}

#[tokio::test]
#[ignore = "downloads packages; needs npm and typescript-language-server"]
async fn a_copy_with_the_same_lockfile_links_the_working_trees_node_modules_and_resolves_into_them() {
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
    let restorer = ToolRestorer::with_env(env("npm_config_cache", cache.path()), &Budgets::default());
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
    let reusing =
        ToolRestorer::with_env(env("npm_config_cache", &missing), &Budgets::default()).reusing(donor.path().to_owned());
    let restored = restore(&reusing, copy.path(), &config::typescript(), false).await;
    assert_eq!(restored.failures, []);
    let linked = copy.path().join("node_modules");
    assert!(linked.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(linked.join("picocolors/package.json").is_file());
    assert!(!missing.exists());

    let server = LspLauncher::new(BinaryLocator, NoRestore, &Budgets::default())
        .start(
            copy.path(),
            Tree::Immutable,
            &config::typescript(),
            AnalysisPolicy::LIMITED,
            &Unwatched,
        )
        .await
        .expect("typescript-language-server starts")
        .server;
    let at = At {
        file: RepoPath(b"a.ts".to_vec()),
        pos: Pos { line: 1, col: 22 },
    };
    let hover = server
        .hover(&at, None)
        .await
        .unwrap()
        .expect("pc.red resolves through node_modules");
    assert!(hover.contains("red"), "{hover}");
    server.shutdown().await;

    // A lockfile that does not match `package.json`: `npm ci` refuses it and `npm install
    // --no-save` stands in for it (§7.2): it installs the dependencies and leaves the lock
    // file alone, and the restore is reported as stood in for, not failed.
    write(
        copy.path(),
        "package-lock.json",
        "{\"lockfileVersion\":3,\"packages\":{}}",
    );
    std::fs::remove_file(&linked).unwrap();
    let reusing = ToolRestorer::with_env(env("npm_config_cache", cache.path()), &Budgets::default())
        .reusing(donor.path().to_owned());
    let restored = reusing
        .restore(copy.path(), &config::typescript(), AnalysisPolicy::LIMITED)
        .await;
    assert_eq!(restored.failures.len(), 1, "{restored:?}");
    assert_eq!(restored.failures[0].kind, RestoreFailureKind::StoodIn, "{restored:?}");
    assert_eq!(restored.failures[0].tool, "npm");
    assert!(
        restored.failures[0].message.contains("lock file out of sync"),
        "{}",
        restored.failures[0].message
    );
    // `npm ci` left nothing, the stand-in installs: the locked dependencies are there.
    assert!(linked.join("picocolors/package.json").is_file());
}

#[tokio::test]
#[ignore = "downloads packages; needs pnpm"]
async fn pnpm_installs_from_the_lockfile_without_running_scripts() {
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

    let restorer = ToolRestorer::with_env(env("npm_config_store_dir", store.path()), &Budgets::default());
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
#[ignore = "downloads packages; needs uv"]
async fn uv_installs_the_locked_dependencies_into_a_venv_the_server_gets() {
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "pyproject.toml",
        "[project]\nname = \"p\"\nversion = \"0\"\nrequires-python = \">=3.9\"\ndependencies = [\"iniconfig==2.1.0\"]\n",
    );
    run(project.path(), "uv", &["lock"]);

    let restorer = ToolRestorer::with_env(env("UV_CACHE_DIR", cache.path()), &Budgets::default());
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
#[ignore = "downloads packages; needs npm"]
async fn npm_restores_a_workspace_once_at_its_root_and_a_project_in_a_subdirectory_on_its_own() {
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

    let restorer = ToolRestorer::with_env(env("npm_config_cache", cache.path()), &Budgets::default());
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
#[ignore = "downloads packages; needs yarn"]
async fn yarn_classic_installs_from_the_lockfile_without_running_scripts() {
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        project.path(),
        "package.json",
        r#"{"name":"p","version":"1.0.0","dependencies":{"picocolors":"1.1.1"},"scripts":{"postinstall":"node -e \"require('fs').writeFileSync('script-ran','')\""}}"#,
    );
    let cache_env = env("YARN_CACHE_FOLDER", cache.path());
    let restorer = ToolRestorer::with_env(cache_env.clone(), &Budgets::default());
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
#[ignore = "downloads packages; needs uv"]
async fn uv_installs_requirements_txt_into_a_venv_and_never_builds_a_source_package() {
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let restorer = ToolRestorer::with_env(env("UV_CACHE_DIR", cache.path()), &Budgets::default());
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
        .restore(local.path(), &config::pyright(), AnalysisPolicy::LIMITED)
        .await;
    assert_eq!(restored.failures.len(), 1, "{restored:?}");
    assert_eq!(restored.failures[0].tool, "uv");
    assert!(!local.path().join("build-ran").exists());
}

#[tokio::test]
#[ignore = "downloads packages; needs uv"]
async fn uv_installs_a_pyproject_without_a_lockfile_from_its_subdirectory() {
    let (project, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(project.path().join("svc")).unwrap();
    write(
        &project.path().join("svc"),
        "pyproject.toml",
        "[project]\nname = \"p\"\nversion = \"0\"\nrequires-python = \">=3.9\"\ndependencies = [\"iniconfig==2.1.0\"]\n",
    );
    let restorer = ToolRestorer::with_env(env("UV_CACHE_DIR", cache.path()), &Budgets::default());
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
#[ignore = "downloads packages; needs dotnet"]
async fn a_copy_with_the_same_projects_takes_the_working_trees_restore_output() {
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

    let restorer = ToolRestorer::with_env(env("NUGET_PACKAGES", packages.path()), &Budgets::default())
        .reusing(donor.path().to_owned());
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
#[ignore = "downloads packages; needs dotnet"]
async fn dotnet_restores_only_where_executing_is_allowed() {
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

    let restorer = ToolRestorer::with_env(env("NUGET_PACKAGES", packages.path()), &Budgets::default());
    let denied = restore(&restorer, project.path(), &config::roslyn(), false).await;
    assert_eq!(denied, Restored::default());
    assert!(!project.path().join("obj").exists());

    restore(&restorer, project.path(), &config::roslyn(), true).await;
    assert!(project.path().join("obj/project.assets.json").is_file());
    assert!(packages.path().join("newtonsoft.json").is_dir());
    // Why it needs the permission: a project's own targets run during restore.
    assert!(project.path().join("restore-ran").exists());
}
