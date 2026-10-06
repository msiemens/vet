//! `vet_semantic::ports::Restorer`: installs the dependencies of a materialized
//! snapshot with each tool's own fetch command (§7.2). Nothing here sets a cache
//! location: every tool keeps its default, which is the user's own cache and so
//! shared across snapshots and with the user's builds.
//!
//! | Language | Command | Cache (tool default) |
//! |---|---|---|
//! | Rust | `cargo fetch` | Cargo home |
//! | TypeScript | `pnpm install --frozen-lockfile --ignore-scripts`, `npm ci --ignore-scripts`, or `npm install --ignore-scripts` | pnpm store, npm cache |
//! | Python | `uv sync --frozen --no-build --no-install-project --no-python-downloads` | uv cache |
//! | C# | `dotnet restore` (runs MSBuild, so only where executing is allowed) | NuGet global packages folder |
//!
//! Only the repository root is restored. A step that fails is reported and the
//! server starts anyway.

use std::path::{Path, PathBuf};
use std::time::Duration;

use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Backend, Restored, Restorer};

use crate::exec::capture;
use crate::lsp::dotnet;

/// Restores download packages: far longer than a probe.
const STEP_TIMEOUT: Duration = Duration::from_secs(600);
const OUTPUT_TAIL_LINES: usize = 12;

/// The `.venv` of a Python restore: uv's `UV_PROJECT_ENVIRONMENT` is set to it.
const VENV: &str = ".venv";

struct Step {
    program: &'static str,
    args: Vec<String>,
    env: Vec<(String, String)>,
    /// Evaluates project files that can run code (MSBuild).
    executes: bool,
}

impl Step {
    fn new(program: &'static str, args: &[&str]) -> Self {
        Self {
            program,
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: Vec::new(),
            executes: false,
        }
    }

    fn describe(&self) -> String {
        format!("{} {}", self.program, self.args.join(" "))
    }
}

#[derive(Default)]
pub struct ToolRestorer {
    /// Added to the environment of every restore command.
    env: Vec<(String, String)>,
}

impl ToolRestorer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_env(env: Vec<(String, String)>) -> Self {
        Self { env }
    }

    async fn plan(&self, root: &Path, language: &str) -> Result<Vec<Step>, String> {
        let has = |name: &str| root.join(name).is_file();
        Ok(match language {
            "rust" if has("Cargo.toml") => vec![Step::new("cargo", &["fetch"])],
            "typescript" if has("package.json") => vec![if has("pnpm-lock.yaml") {
                Step::new("pnpm", &["install", "--frozen-lockfile", "--ignore-scripts"])
            } else if has("package-lock.json") || has("npm-shrinkwrap.json") {
                Step::new("npm", &["ci", "--ignore-scripts"])
            } else if has("yarn.lock") {
                return Err("yarn.lock: Yarn projects are not restored".into());
            } else {
                Step::new("npm", &["install", "--ignore-scripts"])
            }],
            "python" if has("pyproject.toml") && has("uv.lock") => {
                let mut step = Step::new(
                    "uv",
                    &[
                        "sync",
                        "--frozen",
                        "--no-build",
                        "--no-install-project",
                        "--no-python-downloads",
                    ],
                );
                step.env.push((
                    "UV_PROJECT_ENVIRONMENT".into(),
                    root.join(VENV).to_string_lossy().into_owned(),
                ));
                vec![step]
            }
            "csharp" => {
                let targets = match dotnet::find(root).await? {
                    Some(dotnet::Load::Solution(solution)) => vec![solution],
                    Some(dotnet::Load::Projects(projects)) => projects,
                    None => Vec::new(),
                };
                targets
                    .iter()
                    .map(|target| {
                        let mut step = Step::new("dotnet", &["restore"]);
                        step.args.push(target.to_string_lossy().into_owned());
                        step.env = vec![
                            ("DOTNET_NOLOGO".into(), "1".into()),
                            ("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into()),
                        ];
                        step.executes = true;
                        step
                    })
                    .collect()
            }
            _ => Vec::new(),
        })
    }

    async fn run(&self, root: &Path, step: &Step) -> Result<(), String> {
        let env: Vec<_> = self.env.iter().chain(&step.env).cloned().collect();
        let out = capture(step.program, &step.args, Some(root), &env, STEP_TIMEOUT)
            .await
            .map_err(|e| format!("`{}`: {e}", step.describe()))?;
        if out.ok {
            return Ok(());
        }
        let text = if out.stderr.trim().is_empty() {
            &out.stdout
        } else {
            &out.stderr
        };
        let lines: Vec<_> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(OUTPUT_TAIL_LINES)..].join("\n");
        Err(format!("`{}` failed:\n{tail}", step.describe()))
    }
}

/// `PATH` with `dir` first: Pyright runs the `python` that a shell would run
/// (its import resolution documentation), so this selects the restored environment.
fn path_with_first(dir: &Path) -> Option<String> {
    let rest = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(dir.to_owned()).chain(std::env::split_paths(&rest));
    std::env::join_paths(paths)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

fn venv_bin(venv: &Path) -> PathBuf {
    venv.join(if cfg!(windows) { "Scripts" } else { "bin" })
}

impl Restorer for ToolRestorer {
    async fn restore(&self, root: &Path, backend: &Backend, policy: AnalysisPolicy) -> Restored {
        let language = backend.config().backend.0.as_str();
        let mut restored = Restored::default();
        let steps = match self.plan(root, language).await {
            Ok(steps) => steps,
            Err(why) => {
                restored.failures.push(why);
                return restored;
            }
        };
        for step in steps.iter().filter(|s| policy.allow_executing || !s.executes) {
            if let Err(why) = self.run(root, step).await {
                restored.failures.push(why);
            }
        }
        let venv = root.join(VENV);
        if language == "python" && venv.is_dir() {
            restored
                .env
                .push(("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned()));
            restored
                .env
                .extend(path_with_first(&venv_bin(&venv)).map(|p| ("PATH".into(), p)));
        }
        restored
    }
}

#[cfg(test)]
mod tests {
    use vet_semantic::ports::{BackendId, ServerConfig};

    use super::*;
    use crate::exec::{Host, is_file};

    fn backend(language: &str) -> Backend {
        Backend::GenericLsp(ServerConfig {
            backend: BackendId(language.into()),
            binary: String::new(),
            args: vec![],
            filetypes: vec![],
            root_markers: vec![],
            executes_project_code_during_analysis: false,
        })
    }

    fn tree(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            std::fs::write(dir.path().join(file), "").unwrap();
        }
        dir
    }

    async fn commands(files: &[&str], language: &str) -> Result<Vec<String>, String> {
        let dir = tree(files);
        let steps = ToolRestorer::new().plan(dir.path(), language).await?;
        Ok(steps.iter().map(Step::describe).collect())
    }

    #[tokio::test]
    async fn rust_fetches_when_there_is_a_manifest() {
        assert_eq!(commands(&["Cargo.toml"], "rust").await.unwrap(), ["cargo fetch"]);
        assert!(commands(&[], "rust").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_javascript_install_follows_the_lockfile_and_never_runs_scripts() {
        async fn one(files: &[&str]) -> Result<Vec<String>, String> {
            commands(files, "typescript").await
        }
        assert_eq!(
            one(&["package.json", "pnpm-lock.yaml", "package-lock.json"])
                .await
                .unwrap(),
            ["pnpm install --frozen-lockfile --ignore-scripts"]
        );
        assert_eq!(
            one(&["package.json", "package-lock.json"]).await.unwrap(),
            ["npm ci --ignore-scripts"]
        );
        assert_eq!(
            one(&["package.json", "npm-shrinkwrap.json"]).await.unwrap(),
            ["npm ci --ignore-scripts"]
        );
        assert_eq!(one(&["package.json"]).await.unwrap(), ["npm install --ignore-scripts"]);
        assert!(
            one(&["package.json", "yarn.lock"])
                .await
                .unwrap_err()
                .contains("yarn.lock")
        );
        assert!(one(&["tsconfig.json"]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn python_needs_a_uv_lockfile_and_never_builds() {
        let steps = commands(&["pyproject.toml", "uv.lock"], "python").await.unwrap();
        assert_eq!(
            steps,
            ["uv sync --frozen --no-build --no-install-project --no-python-downloads"]
        );
        assert!(commands(&["pyproject.toml"], "python").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn csharp_restores_what_the_server_loads_and_only_where_executing_is_allowed() {
        let dir = tree(&["App.csproj", "Program.cs"]);
        let steps = ToolRestorer::new().plan(dir.path(), "csharp").await.unwrap();
        assert_eq!(steps.len(), 1);
        assert!(steps[0].executes && steps[0].args[0] == "restore");

        // Not allowed: nothing runs, so not even a missing `dotnet` is reported.
        let restored = ToolRestorer::new()
            .restore(
                dir.path(),
                &backend("csharp"),
                AnalysisPolicy { allow_executing: false },
            )
            .await;
        assert_eq!(restored, Restored::default());
    }

    #[tokio::test]
    async fn a_failing_step_is_reported_with_the_tools_output() {
        if Host::current().find("cargo", is_file).is_none() {
            eprintln!("skipped: cargo is not installed");
            return;
        }
        let dir = tree(&[]);
        std::fs::write(dir.path().join("Cargo.toml"), "not a manifest").unwrap();
        let restored = ToolRestorer::new()
            .restore(dir.path(), &backend("rust"), AnalysisPolicy { allow_executing: false })
            .await;
        assert_eq!(restored.failures.len(), 1, "{restored:?}");
        assert!(restored.failures[0].contains("cargo fetch"), "{restored:?}");
        assert!(restored.failures[0].contains("Cargo.toml"), "{restored:?}");
    }
}
