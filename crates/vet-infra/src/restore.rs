//! `vet_semantic::ports::Restorer`: installs the dependencies of a materialized
//! snapshot with each tool's own fetch command (§7.2). Nothing here sets a cache
//! location: every tool keeps its default, which is the user's own cache and so
//! shared across snapshots and with the user's builds.
//!
//! No command runs project code: not a build script, not a package's install
//! script, not a build backend.
//!
//! | Language | Command | Cache (tool default) |
//! |---|---|---|
//! | Rust | `cargo fetch` | Cargo home |
//! | npm | `npm ci --ignore-scripts`, or `npm install --ignore-scripts` without a lockfile | npm cache |
//! | pnpm | `pnpm install --frozen-lockfile --ignore-scripts --ignore-pnpmfile` | pnpm store |
//! | Yarn classic | `yarn install --frozen-lockfile --ignore-scripts --non-interactive` | Yarn cache |
//! | Yarn berry | `yarn install --immutable --mode=skip-build` | Yarn cache |
//! | Python with `uv.lock` | `uv sync --frozen --no-build --no-install-project --no-python-downloads` | uv cache |
//! | Python otherwise | `uv venv`, then `uv pip install --no-build -r requirements.txt -r pyproject.toml` | uv cache |
//! | C# | `dotnet restore` (runs MSBuild, so only where executing is allowed) | NuGet global packages folder |
//!
//! Every project of the tree is restored where its manifest is, except the
//! members of a workspace (Cargo, pnpm, npm, Yarn, uv): their root's install
//! covers them. A step that fails is reported with its project and the server
//! starts anyway.
//!
//! With a donor tree (the working tree, restored for its own server) a JavaScript
//! or C# step is not run when the donor's restore would give the same result:
//! the copy links the donor's `node_modules` if lockfile, install settings and
//! dependencies in `package.json` are the same, or takes the donor's `obj`
//! restore output if every project, `.props`, `.targets`, NuGet setting and lock
//! file is the same. See `reuse`.
//!
//! A tree kept between launches (`materialize::kept`) is restored once: the
//! restore is written to a record next to the tree (`record`) and used again
//! while the tools, the steps and the donor's inputs are the same. The restores
//! of the languages of one tree run one at a time.

pub(crate) mod discover;
pub(crate) mod record;
mod reuse;
mod tools;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use vet_app::progress::{TaskKind, Tasks};
use vet_core::content::Hash;
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::{Backend, RestoreFailure, Restored, Restorer};

use crate::budget::{Budget, Budgets, SEMANTIC_IO};
use crate::exec::{OUTPUT_TAIL_LINES, Tail, dotnet_env};
use crate::git::list_files;
use crate::language::{self, Language, Restore};
use crate::lsp::dotnet;
use crate::materialize::kept::Layout;
use record::{Digest, Outcome, Record, StepRecord};
use reuse::Reused;
use tools::{Programs, Tools};

/// Restores download packages: far longer than a probe.
const STEP_TIMEOUT: Duration = Duration::from_secs(600);

/// Projects restored per language: a monorepo is not a reason to download the world.
const MAX_PROJECTS: usize = 16;

/// The `.venv` of a Python restore: uv's `UV_PROJECT_ENVIRONMENT` is set to it.
const VENV: &str = ".venv";

#[derive(Clone)]
struct Step {
    program: &'static str,
    args: Vec<String>,
    env: Vec<(String, String)>,
    /// Evaluates project files that can run code (MSBuild, a Yarn release checked into the repository).
    executes: bool,
    /// The project directory, relative to the root; the step runs in it.
    project: PathBuf,
    /// The members of the project's workspace, relative to the root, which the
    /// step installs along with the project.
    covers: Vec<PathBuf>,
}

impl Step {
    fn new(project: &Path, program: &'static str, args: &[&str]) -> Self {
        Self {
            program,
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: Vec::new(),
            executes: false,
            project: project.to_owned(),
            covers: Vec::new(),
        }
    }

    fn describe(&self) -> String {
        format!("{} {}", self.program, self.args.join(" "))
    }

    fn failure(&self, message: String) -> RestoreFailure {
        failure(&self.project, self.program, message)
    }

    fn record(&self, outcome: Outcome) -> StepRecord {
        StepRecord {
            project: label(&self.project),
            program: self.program.to_owned(),
            args: self.args.clone(),
            env: self.env.clone(),
            outcome,
        }
    }
}

fn label(project: &Path) -> String {
    let parts: Vec<_> = project.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    if parts.is_empty() { ".".into() } else { parts.join("/") }
}

fn failure(project: &Path, tool: &str, message: String) -> RestoreFailure {
    RestoreFailure {
        project: label(project),
        tool: tool.to_owned(),
        message,
    }
}

#[derive(Default)]
struct Plan {
    steps: Vec<Step>,
    /// Problems found while planning: a project that cannot be restored safely.
    failures: Vec<RestoreFailure>,
}

impl Plan {
    /// The first `MAX_PROJECTS` of `projects`; the rest are reported.
    fn bounded(&mut self, mut projects: Vec<PathBuf>, tool: &str) -> Vec<PathBuf> {
        if projects.len() > MAX_PROJECTS {
            let message = format!(
                "{} projects found: only the first {MAX_PROJECTS} are restored",
                projects.len()
            );
            self.failures.push(failure(Path::new(""), tool, message));
            projects.truncate(MAX_PROJECTS);
        }
        projects
    }
}

pub struct ToolRestorer {
    /// Added to the environment of every restore command.
    env: Vec<(String, String)>,
    /// A tree already restored: a step whose inputs are the same there is not run.
    donor: Option<PathBuf>,
    tools: Arc<dyn Tools>,
    /// The version of each tool, asked once per launch.
    versions: Mutex<HashMap<String, String>>,
    /// Where the steps of a restore report.
    tasks: Tasks,
    /// Where plans read the tree and a donor's output is taken over.
    semantic_io: Budget,
}

impl Default for ToolRestorer {
    fn default() -> Self {
        Self {
            env: Vec::new(),
            donor: None,
            tools: Arc::new(Programs::default()),
            versions: Mutex::default(),
            tasks: Tasks::default(),
            semantic_io: Budget::new(SEMANTIC_IO),
        }
    }
}

impl ToolRestorer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_env(env: Vec<(String, String)>) -> Self {
        Self { env, ..Self::default() }
    }

    #[cfg(test)]
    fn with_tools(mut self, tools: Arc<dyn Tools>) -> Self {
        self.tools = tools;
        self
    }

    /// Draws on `budgets`: version probes, steps and the reading of trees each on their own.
    pub fn budgeted(mut self, budgets: &Budgets) -> Self {
        self.tools = Arc::new(Programs {
            probes: budgets.probes.clone(),
            restores: budgets.restores.clone(),
        });
        self.semantic_io = budgets.semantic_io.clone();
        self
    }

    /// Reports each restore that runs as a task with a step per tool run. A
    /// restore taken from a kept tree runs nothing and reports nothing.
    pub fn reporting(mut self, tasks: Tasks) -> Self {
        self.tasks = tasks;
        self
    }

    /// Takes over the restored dependencies of `donor`, the working tree, for a
    /// step whose lockfile and manifests are the same in the tree restored.
    pub fn reusing(mut self, donor: PathBuf) -> Self {
        self.donor = Some(donor);
        self
    }

    async fn plan(&self, root: &Path, language: &'static Language) -> Plan {
        let mut plan = Plan::default();
        if language.restore == Restore::Dotnet {
            Self::csharp(&self.semantic_io, root, &mut plan).await;
            return plan;
        }
        let files = match list_files(&self.semantic_io, root).await {
            Ok(files) => files,
            Err(why) => {
                plan.failures.push(failure(Path::new(""), "vet", why));
                return plan;
            }
        };
        let root = root.to_owned();
        let planned = self
            .semantic_io
            .blocking(move || {
                match language.restore {
                    Restore::Cargo => Self::rust(&root, &files, &mut plan),
                    Restore::Javascript => Self::javascript(&root, &files, &mut plan),
                    Restore::Python => Self::python(&root, &files, &mut plan),
                    Restore::Dotnet => {}
                }
                plan
            })
            .await;
        planned.unwrap_or_else(|e| {
            let mut plan = Plan::default();
            plan.failures.push(failure(Path::new(""), "vet", e.to_string()));
            plan
        })
    }

    fn rust(root: &Path, files: &[PathBuf], plan: &mut Plan) {
        let dirs = discover::candidates(files, &["Cargo.toml"]);
        let dirs = discover::without_members(dirs.clone(), &discover::cargo_workspaces(root, &dirs));
        for dir in plan.bounded(dirs, "cargo") {
            plan.steps.push(Step::new(&dir, "cargo", &["fetch"]));
        }
    }

    fn javascript(root: &Path, files: &[PathBuf], plan: &mut Plan) {
        let all = discover::candidates(files, &["package.json"]);
        let workspaces = discover::js_workspaces(root, &all);
        let roots = discover::without_members(all.clone(), &workspaces);
        for dir in plan.bounded(roots, "npm") {
            let mut step = javascript_step(&root.join(&dir), &dir);
            step.covers = discover::members(&all, &workspaces, &dir);
            plan.steps.push(step);
        }
    }

    fn python(root: &Path, files: &[PathBuf], plan: &mut Plan) {
        let dirs = discover::candidates(files, &["pyproject.toml", "requirements.txt"]);
        let workspaces = discover::uv_workspaces(root, &dirs);
        let dirs = discover::without_members(dirs, &workspaces);
        for dir in plan.bounded(dirs, "uv") {
            let abs = root.join(&dir);
            let has = |name: &str| abs.join(name).is_file();
            let venv = abs.join(VENV).to_string_lossy().into_owned();
            let pyproject = std::fs::read_to_string(abs.join("pyproject.toml"))
                .ok()
                .and_then(|t| t.parse::<toml::Table>().ok());
            if pyproject.is_some() && has("uv.lock") {
                let mut args = vec![
                    "sync",
                    "--frozen",
                    "--no-build",
                    "--no-install-project",
                    "--no-python-downloads",
                ];
                // A workspace root only installs its own package otherwise.
                if workspaces.iter().any(|w| w.dir() == dir) {
                    args.push("--all-packages");
                }
                let mut step = Step::new(&dir, "uv", &args);
                step.env.push(("UV_PROJECT_ENVIRONMENT".into(), venv));
                plan.steps.push(step);
                continue;
            }
            // Without a lock there is nothing to resolve from but the declared
            // requirements. `uv pip install` reads them statically and `--no-build`
            // refuses every source distribution, `-e` and local path, so no build
            // backend or `setup.py` runs; what needs one fails and is reported.
            let mut requirements = Vec::new();
            if has("requirements.txt") {
                requirements.push("requirements.txt");
            }
            match &pyproject {
                Some(p) if p.contains_key("project") => requirements.push("pyproject.toml"),
                Some(p) if p.get("tool").is_some_and(|t| t.get("poetry").is_some()) => {
                    let message = "pyproject.toml declares its dependencies for Poetry only: not restored".into();
                    plan.failures.push(failure(&dir, "uv", message));
                }
                _ => {}
            }
            if requirements.is_empty() {
                continue;
            }
            plan.steps.push(Step::new(
                &dir,
                "uv",
                &["venv", &venv, "--allow-existing", "--no-python-downloads"],
            ));
            let mut args = vec!["pip", "install", "--python", &venv, "--no-build"];
            args.extend(requirements.iter().flat_map(|r| ["-r", r]));
            plan.steps.push(Step::new(&dir, "uv", &args));
        }
    }

    async fn csharp(io: &Budget, root: &Path, plan: &mut Plan) {
        let targets = match dotnet::find(io, root).await {
            Ok(Some(dotnet::Load::Solution(solution))) => vec![solution],
            Ok(Some(dotnet::Load::Projects(projects))) => projects,
            Ok(None) => Vec::new(),
            Err(why) => {
                plan.failures.push(failure(Path::new(""), "dotnet", why));
                return;
            }
        };
        for target in targets {
            let project = target
                .strip_prefix(root)
                .ok()
                .and_then(Path::parent)
                .unwrap_or(Path::new(""));
            let mut step = Step::new(project, "dotnet", &["restore"]);
            step.args.push(target.to_string_lossy().into_owned());
            step.env = dotnet_env();
            step.executes = true;
            plan.steps.push(step);
        }
    }

    async fn run(&self, root: &Path, step: &Step) -> Result<(), String> {
        let env: Vec<_> = self.env.iter().chain(&step.env).cloned().collect();
        let cwd = root.join(&step.project);
        let out = self
            .tools
            .run(step.program, &step.args, &cwd, &env, STEP_TIMEOUT)
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
        Err(format!(
            "`{}` failed:\n{}",
            step.describe(),
            Tail::of(text, OUTPUT_TAIL_LINES).text()
        ))
    }
}

/// The JavaScript package manager of the project in `abs`: `packageManager` in
/// `package.json` first (what Corepack enforces), else the lockfile present.
fn javascript_step(abs: &Path, project: &Path) -> Step {
    enum Manager {
        Pnpm,
        Npm,
        Yarn { berry: bool },
    }
    let has = |name: &str| abs.join(name).is_file();
    let package: Value = std::fs::read_to_string(abs.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let declared = package.get("packageManager").and_then(Value::as_str).unwrap_or("");
    let (name, version) = declared.split_once('@').unwrap_or((declared, ""));
    // Berry's lockfile has a `__metadata` entry, classic's a "yarn lockfile v1" header.
    let berry = || {
        let major = version.split('.').next().and_then(|m| m.parse::<u32>().ok());
        major.map_or_else(
            || {
                has(".yarnrc.yml")
                    || std::fs::read_to_string(abs.join("yarn.lock")).is_ok_and(|l| l.contains("__metadata:"))
            },
            |major| major >= 2,
        )
    };
    let manager = match name {
        "pnpm" => Manager::Pnpm,
        "npm" => Manager::Npm,
        "yarn" => Manager::Yarn { berry: berry() },
        _ if has("pnpm-lock.yaml") => Manager::Pnpm,
        _ if has("package-lock.json") || has("npm-shrinkwrap.json") => Manager::Npm,
        _ if has("yarn.lock") => Manager::Yarn { berry: berry() },
        _ => Manager::Npm,
    };
    let mut step = match manager {
        // pnpm runs the repository's `.pnpmfile.cjs` hooks during an install.
        Manager::Pnpm if has("pnpm-lock.yaml") => Step::new(
            project,
            "pnpm",
            &["install", "--frozen-lockfile", "--ignore-scripts", "--ignore-pnpmfile"],
        ),
        Manager::Pnpm => Step::new(project, "pnpm", &["install", "--ignore-scripts", "--ignore-pnpmfile"]),
        Manager::Npm if has("package-lock.json") || has("npm-shrinkwrap.json") => {
            Step::new(project, "npm", &["ci", "--ignore-scripts"])
        }
        Manager::Npm => Step::new(project, "npm", &["install", "--ignore-scripts"]),
        Manager::Yarn { berry: false } => {
            let mut args = vec!["install", "--ignore-scripts", "--non-interactive"];
            if has("yarn.lock") {
                args.push("--frozen-lockfile");
            }
            Step::new(project, "yarn", &args)
        }
        Manager::Yarn { berry: true } => {
            let mut args = vec!["install", "--mode=skip-build"];
            if has("yarn.lock") {
                args.push("--immutable");
            }
            let mut step = Step::new(project, "yarn", &args);
            step.env.push(("YARN_ENABLE_SCRIPTS".into(), "false".into()));
            step.env.push(("YARN_ENABLE_TELEMETRY".into(), "0".into()));
            step
        }
    };
    // Yarn runs the release a repository checks in (`yarnPath`, `yarn-path`) and its plugins.
    let configured = |file: &str, keys: &[&str]| {
        std::fs::read_to_string(abs.join(file)).is_ok_and(|c| keys.iter().any(|k| c.contains(k)))
    };
    step.executes = step.program == "yarn"
        && (configured(".yarnrc.yml", &["yarnPath", "plugins:"]) || configured(".yarnrc", &["yarn-path"]));
    // Corepack would otherwise wait for an answer to download the pinned manager.
    step.env.push(("COREPACK_ENABLE_DOWNLOAD_PROMPT".into(), "0".into()));
    step
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

/// The environment the server runs in: one server, one environment, the first
/// project's, which is the root's if it has one.
fn environment(root: &Path, language: &Language, plan: &Plan) -> Vec<(String, String)> {
    let venv = plan
        .steps
        .iter()
        .map(|s| root.join(&s.project).join(VENV))
        .find(|v| v.is_dir());
    let mut env = Vec::new();
    if let (Restore::Python, Some(venv)) = (language.restore, venv) {
        env.push(("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned()));
        env.extend(path_with_first(&venv_bin(&venv)).map(|p| ("PATH".into(), p)));
    }
    env
}

impl ToolRestorer {
    /// Runs the steps of `plan` that the policy allows, and records how each ended.
    async fn execute(
        &self,
        root: &Path,
        language: &Language,
        plan: &Plan,
        policy: AnalysisPolicy,
    ) -> (Restored, Vec<StepRecord>) {
        let mut restored = Restored {
            failures: plan.failures.clone(),
            ..Restored::default()
        };
        let mut records = Vec::new();
        let mut failed: Vec<&Path> = Vec::new();
        let steps: Vec<&Step> = allowed(plan, policy).collect();
        let total = steps.len() as u32;
        let task = (total > 0).then(|| {
            self.tasks
                .start(TaskKind::Restore, format!("Restoring {} dependencies", language.title))
        });
        for (done, step) in steps.into_iter().enumerate() {
            if let Some(task) = &task {
                let at = format!("{}: {}", label(&step.project), step.describe());
                task.report(Some((done as u32, total)), Some(at));
            }
            // The rest of a project's steps need the one that failed.
            if failed.contains(&step.project.as_path()) {
                records.push(step.record(Outcome::Failed("an earlier step of the project failed".into())));
                continue;
            }
            restored.ran.push((label(&step.project), step.program.to_owned()));
            if let Some(donor) = &self.donor
                && let Some(reused) = reuse::reuse(&self.semantic_io, donor, root, step).await
            {
                records.push(step.record(match reused {
                    Reused::Linked(inputs) => Outcome::Linked {
                        donor: donor.to_string_lossy().into_owned(),
                        inputs,
                    },
                    Reused::Copied => Outcome::Copied,
                }));
                continue;
            }
            match self.run(root, step).await {
                Ok(()) => records.push(step.record(Outcome::Ran)),
                Err(why) => {
                    // The tool's output can quote code: only its first line is logged above `debug`.
                    tracing::warn!(
                        project = %label(&step.project),
                        tool = step.program,
                        reason = vet_log::first_line(&why),
                        "a restore step failed"
                    );
                    tracing::debug!(project = %label(&step.project), tool = step.program, output = %why, "the failed restore step's output");
                    records.push(step.record(Outcome::Failed(why.clone())));
                    restored.failures.push(step.failure(why));
                    failed.push(&step.project);
                }
            }
        }
        if let Some(task) = task {
            if failed.is_empty() {
                task.finish();
            } else {
                task.fail("a restore step failed".into());
            }
        }
        restored.env = environment(root, language, plan);
        (restored, records)
    }

    /// `(program, version)` of every tool the steps run, asked once per launch.
    async fn versions(&self, steps: &[&Step]) -> Vec<(String, String)> {
        let mut programs: BTreeSet<&str> = steps.iter().map(|s| s.program).collect();
        // The package managers run on it.
        if programs.iter().any(|p| matches!(*p, "npm" | "pnpm" | "yarn")) {
            programs.insert("node");
        }
        let mut versions = Vec::new();
        for program in programs {
            let known = self
                .versions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(program)
                .cloned();
            let version = match known {
                Some(version) => version,
                None => {
                    let version = self.tools.version(program).await;
                    self.versions
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(program.to_owned(), version.clone());
                    version
                }
            };
            versions.push((program.to_owned(), version));
        }
        versions
    }

    /// Everything that decides what a restore does before it runs: the policy,
    /// the environment, the tools and their versions, the steps and what the
    /// plan could not restore. A record made under other inputs is not reused.
    fn inputs(&self, plan: &Plan, steps: &[&Step], tools: &[(String, String)], policy: AnalysisPolicy) -> Hash {
        let mut digest = Digest::default();
        digest.field(if policy.allow_executing { "exec" } else { "noexec" });
        digest.field("environment");
        for (key, value) in &self.env {
            digest.field(key).field(value);
        }
        digest.field("tools");
        for (program, version) in tools {
            digest.field(program).field(version);
        }
        digest.field("steps");
        for step in steps {
            digest.field(&label(&step.project)).field(step.program);
            for arg in &step.args {
                digest.field(arg);
            }
            digest.field("with");
            for (key, value) in &step.env {
                digest.field(key).field(value);
            }
        }
        digest.field("failures");
        for failure in &plan.failures {
            digest
                .field(&failure.project)
                .field(&failure.tool)
                .field(&failure.message);
        }
        digest.finish()
    }

    /// Whether `record` stands for the restore that would run now: same inputs,
    /// every step done, and the donor's `node_modules` still what they were linked as.
    fn reusable(&self, record: &Record, inputs: Hash, root: &Path, steps: &[&Step]) -> bool {
        record.inputs == inputs
            && record.succeeded()
            && record.steps.len() == steps.len()
            && record.steps.iter().zip(steps).all(|(done, step)| match &done.outcome {
                Outcome::Linked { donor, inputs } => {
                    self.donor.as_deref().is_some_and(|d| {
                        d.to_string_lossy() == donor.as_str() && reuse::linked_inputs(d, step) == *inputs
                    }) && reuse::link_intact(root, step)
                }
                _ => true,
            })
    }

    /// The restore of `plan` as it would run now, and the kept tree's record of it if
    /// the record stands for it.
    async fn standing<'p>(
        &self,
        layout: &Layout,
        root: &Path,
        language: &Language,
        plan: &'p Plan,
        policy: AnalysisPolicy,
    ) -> Standing<'p> {
        let steps: Vec<&Step> = allowed(plan, policy).collect();
        let tools = self.versions(&steps).await;
        let inputs = self.inputs(plan, &steps, &tools, policy);
        let file = layout.meta().join(format!("restore-{}.json", language.id));
        let record = Record::read(&file).filter(|record| self.reusable(record, inputs, root, &steps));
        Standing {
            steps,
            tools,
            inputs,
            file,
            record,
        }
    }

    /// Restores a kept tree unless its record says it is restored. The tree's
    /// languages take turns.
    async fn restore_kept(
        &self,
        layout: &Layout,
        root: &Path,
        language: &Language,
        plan: &Plan,
        policy: AnalysisPolicy,
    ) -> Restored {
        let started = Instant::now();
        let Ok(_turn) = layout.lock_restore(&self.semantic_io).await else {
            return self.execute(root, language, plan, policy).await.0;
        };
        let Standing {
            steps,
            tools,
            inputs,
            file,
            record,
        } = self.standing(layout, root, language, plan, policy).await;
        if let Some(record) = record {
            vet_semantic::trace::log(|| format!("restore {} (kept)", language.id), started);
            return Restored {
                failures: plan.failures.clone(),
                ran: steps
                    .iter()
                    .map(|s| (label(&s.project), s.program.to_owned()))
                    .collect(),
                env: environment(root, language, plan),
                fingerprint: Some(record.fingerprint()),
            };
        }
        // What an earlier restore left must not stand in for this one.
        for step in &steps {
            reuse::unlink(root, step);
        }
        let (mut restored, done) = self.execute(root, language, plan, policy).await;
        let record = Record {
            inputs,
            tools,
            steps: done,
        };
        restored.fingerprint = Some(record.fingerprint());
        // Without the record the next use restores again, which is only slower.
        let _ = record.write(&file);
        layout.measure(&self.semantic_io).await;
        vet_semantic::trace::log(|| format!("restore {} (built)", language.id), started);
        restored
    }
}

/// A restore as it would run now, and the record that stands for it.
struct Standing<'p> {
    steps: Vec<&'p Step>,
    tools: Vec<(String, String)>,
    inputs: Hash,
    /// Where the record of the language is kept.
    file: PathBuf,
    /// The kept tree's record, if the restore would be taken from it.
    record: Option<Record>,
}

/// The steps the policy lets run.
fn allowed(plan: &Plan, policy: AnalysisPolicy) -> impl Iterator<Item = &Step> {
    plan.steps.iter().filter(move |s| policy.allow_executing || !s.executes)
}

impl Restorer for ToolRestorer {
    async fn restore(&self, root: &Path, backend: &Backend, policy: AnalysisPolicy) -> Restored {
        let Some(language) = language::by_id(&backend.config().backend.0) else {
            return Restored::default();
        };
        let plan = self.plan(root, language).await;
        match Layout::of_tree(root) {
            Some(layout) => self.restore_kept(&layout, root, language, &plan, policy).await,
            None => self.execute(root, language, &plan, policy).await.0,
        }
    }

    async fn recorded(&self, root: &Path, backend: &Backend, policy: AnalysisPolicy) -> Option<Hash> {
        let language = language::by_id(&backend.config().backend.0)?;
        let layout = Layout::of_tree(root)?;
        let plan = self.plan(root, language).await;
        let standing = self.standing(&layout, root, language, &plan, policy).await;
        standing.record.map(|record| record.fingerprint())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use vet_app::progress::TaskState;

    use super::*;
    use crate::exec::{Host, is_file};

    fn lang(id: &str) -> &'static Language {
        language::by_id(id).expect("a registered language")
    }

    fn backend(id: &str) -> Backend {
        lang(id).backend()
    }

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir
    }

    /// `project: command` of each step, and the plan's own failures.
    async fn planned(files: &[(&str, &str)], language: &str) -> (Vec<String>, Vec<RestoreFailure>) {
        let dir = tree(files);
        let plan = ToolRestorer::new().plan(dir.path(), lang(language)).await;
        let steps = plan.steps.iter();
        (
            steps
                .map(|s| format!("{}: {}", label(&s.project), s.describe()))
                .collect(),
            plan.failures,
        )
    }

    async fn commands(files: &[(&str, &str)], language: &str) -> Vec<String> {
        let (steps, failures) = planned(files, language).await;
        assert!(failures.is_empty(), "{failures:?}");
        steps
    }

    #[tokio::test]
    async fn rust_fetches_once_per_workspace_and_per_stand_alone_manifest() {
        assert_eq!(commands(&[("Cargo.toml", "")], "rust").await, [".: cargo fetch"]);
        assert!(commands(&[("README.md", "")], "rust").await.is_empty());
        assert_eq!(
            commands(
                &[
                    ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
                    ("crates/a/Cargo.toml", ""),
                    ("crates/b/Cargo.toml", ""),
                    ("tools/gen/Cargo.toml", ""),
                    ("tools/gen/node_modules/x/Cargo.toml", ""),
                ],
                "rust"
            )
            .await,
            [".: cargo fetch"]
        );
        assert_eq!(
            commands(&[("svc/a/Cargo.toml", ""), ("svc/b/Cargo.toml", "")], "rust").await,
            ["svc/a: cargo fetch", "svc/b: cargo fetch"]
        );
    }

    #[tokio::test]
    async fn the_javascript_install_follows_the_lockfile_and_never_runs_scripts() {
        async fn one(files: &[&str]) -> Vec<String> {
            let files: Vec<_> = files.iter().map(|f| (*f, "")).collect();
            commands(&files, "typescript").await
        }
        assert_eq!(
            one(&["package.json", "pnpm-lock.yaml", "package-lock.json"]).await,
            [".: pnpm install --frozen-lockfile --ignore-scripts --ignore-pnpmfile"]
        );
        assert_eq!(
            one(&["package.json", "package-lock.json"]).await,
            [".: npm ci --ignore-scripts"]
        );
        assert_eq!(
            one(&["package.json", "npm-shrinkwrap.json"]).await,
            [".: npm ci --ignore-scripts"]
        );
        assert_eq!(one(&["package.json"]).await, [".: npm install --ignore-scripts"]);
        assert_eq!(
            one(&["package.json", "yarn.lock"]).await,
            [".: yarn install --ignore-scripts --non-interactive --frozen-lockfile"]
        );
        assert!(one(&["tsconfig.json"]).await.is_empty());
    }

    #[tokio::test]
    async fn yarn_berry_is_told_apart_and_skips_builds() {
        let package = |manager: &str| format!("{{\"packageManager\": \"{manager}\"}}");
        let berry = ".: yarn install --mode=skip-build --immutable";
        let classic = ".: yarn install --ignore-scripts --non-interactive --frozen-lockfile";
        // By the lockfile's header, or `.yarnrc.yml`.
        assert_eq!(
            commands(
                &[("package.json", "{}"), ("yarn.lock", "__metadata:\n  version: 8\n")],
                "typescript"
            )
            .await,
            [berry]
        );
        assert_eq!(
            commands(
                &[
                    ("package.json", "{}"),
                    ("yarn.lock", ""),
                    (".yarnrc.yml", "nodeLinker: node-modules\n")
                ],
                "typescript"
            )
            .await,
            [berry]
        );
        assert_eq!(
            commands(
                &[("package.json", "{}"), ("yarn.lock", "# yarn lockfile v1\n")],
                "typescript"
            )
            .await,
            [classic]
        );
        // By `packageManager`, which wins over a lockfile of another manager.
        assert_eq!(
            commands(
                &[("package.json", &package("yarn@4.1.0+sha224.abc")), ("yarn.lock", "")],
                "typescript"
            )
            .await,
            [berry]
        );
        assert_eq!(
            commands(
                &[("package.json", &package("yarn@1.22.22")), ("yarn.lock", "__metadata:")],
                "typescript"
            )
            .await,
            [classic]
        );
        // Without a lockfile nothing is frozen.
        assert_eq!(
            commands(&[("package.json", &package("yarn@4.1.0"))], "typescript").await,
            [".: yarn install --mode=skip-build"]
        );
        assert_eq!(
            commands(
                &[("package.json", &package("pnpm@9.1.0")), ("package-lock.json", "")],
                "typescript"
            )
            .await,
            [".: pnpm install --ignore-scripts --ignore-pnpmfile"]
        );
        assert_eq!(
            commands(
                &[("package.json", &package("npm@10.0.0")), ("yarn.lock", "")],
                "typescript"
            )
            .await,
            [".: npm install --ignore-scripts"]
        );
    }

    #[tokio::test]
    async fn a_yarn_release_checked_into_the_repository_counts_as_executing() {
        async fn executes(rc: &str, content: &str) -> bool {
            let dir = tree(&[("package.json", "{}"), ("yarn.lock", ""), (rc, content)]);
            ToolRestorer::new().plan(dir.path(), lang("typescript")).await.steps[0].executes
        }
        assert!(executes(".yarnrc.yml", "yarnPath: .yarn/releases/yarn-4.js\n").await);
        assert!(executes(".yarnrc.yml", "plugins:\n  - path: p.cjs\n").await);
        assert!(executes(".yarnrc", "yarn-path \".yarn/releases/yarn-1.js\"\n").await);
        assert!(!executes(".yarnrc.yml", "nodeLinker: node-modules\n").await);
    }

    #[tokio::test]
    async fn javascript_workspace_members_are_installed_with_their_root() {
        assert_eq!(
            commands(
                &[
                    ("package.json", "{}"),
                    ("pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n"),
                    ("pnpm-lock.yaml", ""),
                    ("packages/a/package.json", "{}"),
                    ("packages/b/package.json", "{}"),
                    ("website/package.json", "{}"),
                    ("website/package-lock.json", ""),
                ],
                "typescript"
            )
            .await,
            [
                ".: pnpm install --frozen-lockfile --ignore-scripts --ignore-pnpmfile",
                "website: npm ci --ignore-scripts"
            ]
        );
    }

    #[tokio::test]
    async fn projects_are_bounded_and_the_rest_is_reported() {
        let files: Vec<_> = (0..MAX_PROJECTS + 2).map(|i| format!("p{i:02}/Cargo.toml")).collect();
        let files: Vec<_> = files.iter().map(|f| (f.as_str(), "")).collect();
        let (steps, failures) = planned(&files, "rust").await;
        assert_eq!(steps.len(), MAX_PROJECTS);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].message.contains("only the first 16"), "{failures:?}");
    }

    #[tokio::test]
    async fn python_with_a_uv_lock_syncs_without_building() {
        let sync = "uv sync --frozen --no-build --no-install-project --no-python-downloads";
        assert_eq!(
            commands(
                &[("pyproject.toml", "[project]\nname = \"p\"\n"), ("uv.lock", "")],
                "python"
            )
            .await,
            [format!(".: {sync}")]
        );
        // A workspace root installs its members' packages too; the members have no steps.
        assert_eq!(
            commands(
                &[
                    ("pyproject.toml", "[tool.uv.workspace]\nmembers = [\"pkgs/*\"]\n"),
                    ("uv.lock", ""),
                    ("pkgs/a/pyproject.toml", "[project]\nname = \"a\"\n"),
                ],
                "python"
            )
            .await,
            [format!(".: {sync} --all-packages")]
        );
    }

    #[tokio::test]
    async fn python_without_a_lock_installs_declared_requirements_without_building() {
        let dir = tree(&[]);
        let venv = dir.path().join(VENV);
        let venv = venv.to_str().unwrap();
        let restorer = ToolRestorer::new();
        let steps = |plan: &Plan| plan.steps.iter().map(Step::describe).collect::<Vec<_>>();
        let create = format!("uv venv {venv} --allow-existing --no-python-downloads");

        std::fs::write(dir.path().join("requirements.txt"), "").unwrap();
        let plan = restorer.plan(dir.path(), lang("python")).await;
        assert_eq!(
            steps(&plan),
            [
                create.clone(),
                format!("uv pip install --python {venv} --no-build -r requirements.txt")
            ]
        );

        std::fs::write(dir.path().join("pyproject.toml"), "[project]\nname = \"p\"\n").unwrap();
        let plan = restorer.plan(dir.path(), lang("python")).await;
        assert_eq!(
            steps(&plan),
            [
                create,
                format!("uv pip install --python {venv} --no-build -r requirements.txt -r pyproject.toml")
            ]
        );

        // Tool configuration alone has nothing to install; Poetry's own format is reported.
        std::fs::remove_file(dir.path().join("requirements.txt")).unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.ruff]\nline-length = 100\n").unwrap();
        let plan = restorer.plan(dir.path(), lang("python")).await;
        assert!(plan.steps.is_empty() && plan.failures.is_empty());
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.poetry]\nname = \"p\"\n").unwrap();
        let plan = restorer.plan(dir.path(), lang("python")).await;
        assert!(plan.steps.is_empty());
        assert_eq!(plan.failures.len(), 1);
        assert!(plan.failures[0].message.contains("Poetry"), "{:?}", plan.failures);
    }

    #[tokio::test]
    async fn python_projects_in_subdirectories_get_their_own_environment() {
        let (steps, _) = planned(
            &[
                ("requirements.txt", ""),
                ("svc/pyproject.toml", "[project]\nname = \"s\"\n"),
                ("svc/uv.lock", ""),
            ],
            "python",
        )
        .await;
        let projects: Vec<_> = steps.iter().map(|s| s.split(':').next().unwrap()).collect();
        assert_eq!(projects, [".", ".", "svc"]);
        assert!(steps[2].contains("uv sync"));
    }

    #[tokio::test]
    async fn csharp_restores_what_the_server_loads_and_only_where_executing_is_allowed() {
        let dir = tree(&[("App.csproj", ""), ("Program.cs", "")]);
        let plan = ToolRestorer::new().plan(dir.path(), lang("csharp")).await;
        assert_eq!(plan.steps.len(), 1);
        assert!(plan.steps[0].executes && plan.steps[0].args[0] == "restore");

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
    async fn a_failing_step_is_reported_with_its_project_tool_and_output() {
        if Host::current().find("cargo", is_file).is_none() {
            eprintln!("skipped: cargo is not installed");
            return;
        }
        let dir = tree(&[("Cargo.toml", "not a manifest")]);
        let restored = ToolRestorer::new()
            .restore(dir.path(), &backend("rust"), AnalysisPolicy { allow_executing: false })
            .await;
        assert_eq!(restored.failures.len(), 1, "{restored:?}");
        let failure = &restored.failures[0];
        assert_eq!((failure.project.as_str(), failure.tool.as_str()), (".", "cargo"));
        assert!(failure.message.contains("cargo fetch"), "{restored:?}");
        assert!(failure.message.contains("Cargo.toml"), "{restored:?}");
    }

    /// Runs nothing: records what was asked, and answers the way it is told to.
    #[derive(Default)]
    struct FakeTools {
        versions: Mutex<HashMap<String, String>>,
        failing: Mutex<BTreeSet<String>>,
        runs: Mutex<Vec<String>>,
        active: AtomicUsize,
        overlapped: AtomicBool,
    }

    impl FakeTools {
        fn version_of(self: &Arc<Self>, program: &str, version: &str) -> Arc<Self> {
            self.versions.lock().unwrap().insert(program.into(), version.into());
            self.clone()
        }

        fn fail(self: &Arc<Self>, program: &str, fails: bool) {
            let mut failing = self.failing.lock().unwrap();
            if fails {
                failing.insert(program.into());
            } else {
                failing.remove(program);
            }
        }

        fn runs(&self) -> Vec<String> {
            self.runs.lock().unwrap().clone()
        }
    }

    impl Tools for FakeTools {
        fn version<'a>(&'a self, program: &'a str) -> tools::BoxFuture<'a, String> {
            let version = self.versions.lock().unwrap().get(program).cloned();
            Box::pin(std::future::ready(version.unwrap_or_else(|| "1.0".into())))
        }

        fn run<'a>(
            &'a self,
            program: &'a str,
            args: &'a [String],
            _: &'a Path,
            _: &'a [(String, String)],
            _: Duration,
        ) -> tools::BoxFuture<'a, Result<crate::exec::Captured, String>> {
            Box::pin(async move {
                self.runs.lock().unwrap().push(format!("{program} {}", args.join(" ")));
                if self.active.fetch_add(1, Ordering::SeqCst) > 0 {
                    self.overlapped.store(true, Ordering::SeqCst);
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
                self.active.fetch_sub(1, Ordering::SeqCst);
                let ok = !self.failing.lock().unwrap().contains(program);
                Ok(crate::exec::Captured {
                    ok,
                    stdout: String::new(),
                    stderr: if ok { String::new() } else { "boom".into() },
                })
            })
        }
    }

    /// A tree laid out as `materialize::kept` keeps it: `<key>/tree` and `<key>/meta/complete`.
    fn kept(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("snapshot-noexec");
        std::fs::create_dir_all(key.join("meta")).unwrap();
        std::fs::write(key.join("meta/complete"), "").unwrap();
        for (name, content) in files {
            let path = key.join("tree").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let root = key.join("tree");
        (dir, root)
    }

    const NO_EXEC: AnalysisPolicy = AnalysisPolicy { allow_executing: false };

    fn restorer(tools: &Arc<FakeTools>) -> ToolRestorer {
        ToolRestorer::new().with_tools(tools.clone())
    }

    #[tokio::test]
    async fn a_kept_tree_is_restored_once_and_a_restart_finds_the_record() {
        let (_dir, root) = kept(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());

        let first = restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;
        assert_eq!(tools.runs(), ["cargo fetch"]);
        assert!(first.fingerprint.is_some());

        // A new launch: its restorer has probed nothing.
        let second = restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;
        assert_eq!(tools.runs(), ["cargo fetch"], "nothing ran again");
        assert_eq!(second, first);
    }

    #[tokio::test]
    async fn the_recorded_restore_of_a_kept_tree_is_found_without_restoring_while_the_restore_would_be_taken_from_it() {
        let (_dir, root) = kept(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        let rust = backend("rust");
        assert_eq!(
            restorer(&tools).recorded(&root, &rust, NO_EXEC).await,
            None,
            "no record yet"
        );
        assert!(tools.runs().is_empty());

        let restored = restorer(&tools).restore(&root, &rust, NO_EXEC).await;
        assert_eq!(
            restorer(&tools).recorded(&root, &rust, NO_EXEC).await,
            restored.fingerprint
        );
        assert_eq!(tools.runs(), ["cargo fetch"], "looking did not restore");

        // Anything that would make a start restore again leaves the record without standing.
        let upgraded = Arc::new(FakeTools::default());
        assert_eq!(
            restorer(&upgraded.version_of("cargo", "2.0"))
                .recorded(&root, &rust, NO_EXEC)
                .await,
            None,
            "a tool of another version"
        );
        std::fs::create_dir_all(root.join("svc")).unwrap();
        std::fs::write(root.join("svc/Cargo.toml"), "").unwrap();
        assert_eq!(
            restorer(&tools).recorded(&root, &rust, NO_EXEC).await,
            None,
            "another layout of manifests"
        );
        assert_eq!(
            restorer(&tools)
                .recorded(&root, &rust, AnalysisPolicy { allow_executing: true })
                .await,
            None,
            "another policy"
        );
        assert_eq!(tools.runs(), ["cargo fetch"]);
    }

    #[tokio::test]
    async fn a_record_of_a_failed_restore_or_of_a_stale_link_is_not_found() {
        let (_dir, root) = kept(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        tools.fail("cargo", true);
        restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;
        assert_eq!(restorer(&tools).recorded(&root, &backend("rust"), NO_EXEC).await, None);

        let (_dir, root) = kept(&[("package.json", PACKAGE), ("package-lock.json", "lock")]);
        let donor = tree(&[
            ("package.json", PACKAGE),
            ("package-lock.json", "lock"),
            ("node_modules/a/index.js", ""),
        ]);
        let reusing = || restorer(&tools).reusing(donor.path().to_owned());
        let typescript = backend("typescript");
        let linked = reusing().restore(&root, &typescript, NO_EXEC).await;
        assert_eq!(
            reusing().recorded(&root, &typescript, NO_EXEC).await,
            linked.fingerprint
        );

        std::fs::write(donor.path().join("package-lock.json"), "other lock").unwrap();
        assert_eq!(reusing().recorded(&root, &typescript, NO_EXEC).await, None);
        assert_eq!(
            restorer(&tools).recorded(&root, &typescript, NO_EXEC).await,
            None,
            "a restorer with no donor would restore"
        );
    }

    #[tokio::test]
    async fn a_tree_outside_the_kept_layout_has_no_recorded_restore() {
        let dir = tree(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        restorer(&tools).restore(dir.path(), &backend("rust"), NO_EXEC).await;
        assert_eq!(
            restorer(&tools).recorded(dir.path(), &backend("rust"), NO_EXEC).await,
            None
        );
    }

    #[tokio::test]
    async fn only_a_restore_that_runs_steps_is_reported_as_a_task() {
        let (_dir, root) = kept(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        let announced = Arc::new(Mutex::new(Vec::new()));
        let tasks = Tasks::new(Duration::ZERO, {
            let announced = announced.clone();
            Box::new(move |task| announced.lock().unwrap().push((task.title, task.state)))
        });
        let reporting = || restorer(&tools).reporting(tasks.clone());

        reporting().restore(&root, &backend("rust"), NO_EXEC).await;
        let built = announced.lock().unwrap().clone();
        assert!(built.iter().all(|(title, _)| title == "Restoring Rust dependencies"));
        assert_eq!(built.last().map(|(_, state)| state), Some(&TaskState::Finished));

        announced.lock().unwrap().clear();
        reporting().restore(&root, &backend("rust"), NO_EXEC).await;
        assert_eq!(tools.runs(), ["cargo fetch"], "the second restore was kept");
        assert!(announced.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_tool_that_changed_version_restores_again() {
        let (_dir, root) = kept(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        let before = restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;

        let upgraded = Arc::new(FakeTools::default());
        let after = restorer(&upgraded.version_of("cargo", "2.0"))
            .restore(&root, &backend("rust"), NO_EXEC)
            .await;
        assert_eq!(upgraded.runs(), ["cargo fetch"]);
        assert_ne!(after.fingerprint, before.fingerprint);

        // The record is the new one now.
        let again = restorer(&upgraded).restore(&root, &backend("rust"), NO_EXEC).await;
        assert_eq!(upgraded.runs().len(), 1);
        assert_eq!(again.fingerprint, after.fingerprint);
    }

    #[tokio::test]
    async fn another_manifest_layout_restores_again() {
        let (_dir, root) = kept(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;
        std::fs::create_dir_all(root.join("svc")).unwrap();
        std::fs::write(root.join("svc/Cargo.toml"), "").unwrap();
        restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;
        assert_eq!(tools.runs(), ["cargo fetch", "cargo fetch", "cargo fetch"]);
    }

    #[tokio::test]
    async fn a_restore_that_failed_is_tried_again_the_next_time() {
        let (_dir, root) = kept(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        tools.fail("cargo", true);
        let failed = restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;
        assert_eq!(failed.failures.len(), 1);

        tools.fail("cargo", false);
        let fixed = restorer(&tools).restore(&root, &backend("rust"), NO_EXEC).await;
        assert!(fixed.failures.is_empty());
        assert_eq!(tools.runs().len(), 2);
        assert_ne!(fixed.fingerprint, failed.fingerprint);
    }

    const PACKAGE: &str = r#"{"name":"p","dependencies":{"a":"1"},"scripts":{}}"#;

    #[tokio::test]
    async fn a_link_to_the_donors_node_modules_is_checked_against_the_donor_before_it_is_reused() {
        let (_dir, root) = kept(&[("package.json", PACKAGE), ("package-lock.json", "lock")]);
        let donor = tree(&[
            ("package.json", PACKAGE),
            ("package-lock.json", "lock"),
            ("node_modules/a/index.js", ""),
        ]);
        let tools = Arc::new(FakeTools::default());
        let reusing = || restorer(&tools).reusing(donor.path().to_owned());
        let typescript = backend("typescript");

        let linked = reusing().restore(&root, &typescript, NO_EXEC).await;
        assert!(tools.runs().is_empty(), "the donor's install stood in for it");
        assert!(root.join("node_modules/a/index.js").is_file());

        // Edits that do not change what an install does leave the link.
        std::fs::write(
            donor.path().join("package.json"),
            r#"{"name":"p","dependencies":{"a":"1"},"scripts":{"build":"x"}}"#,
        )
        .unwrap();
        let again = reusing().restore(&root, &typescript, NO_EXEC).await;
        assert!(tools.runs().is_empty());
        assert_eq!(again.fingerprint, linked.fingerprint);

        // The donor installs something else: the link follows it, so the tree is restored for real.
        std::fs::write(donor.path().join("package-lock.json"), "other lock").unwrap();
        let moved = reusing().restore(&root, &typescript, NO_EXEC).await;
        assert_eq!(tools.runs(), ["npm ci --ignore-scripts"]);
        assert!(!root.join("node_modules").exists(), "the link is gone");
        assert!(donor.path().join("node_modules/a/index.js").is_file());
        assert_ne!(moved.fingerprint, linked.fingerprint);
    }

    #[tokio::test]
    async fn a_donor_that_lost_its_install_is_not_linked_to() {
        let (_dir, root) = kept(&[("package.json", PACKAGE), ("package-lock.json", "lock")]);
        let donor = tree(&[
            ("package.json", PACKAGE),
            ("package-lock.json", "lock"),
            ("node_modules/a/index.js", ""),
        ]);
        let tools = Arc::new(FakeTools::default());
        let reusing = || restorer(&tools).reusing(donor.path().to_owned());
        reusing().restore(&root, &backend("typescript"), NO_EXEC).await;

        std::fs::remove_dir_all(donor.path().join("node_modules")).unwrap();
        reusing().restore(&root, &backend("typescript"), NO_EXEC).await;
        assert_eq!(tools.runs(), ["npm ci --ignore-scripts"]);
    }

    #[tokio::test]
    async fn the_languages_of_one_kept_tree_restore_one_at_a_time() {
        let files = [("Cargo.toml", ""), ("package.json", "{}"), ("package-lock.json", "")];
        let both = |tools: &Arc<FakeTools>, root: PathBuf| {
            let (rust, typescript) = (restorer(tools), restorer(tools));
            async move {
                let (rust_backend, typescript_backend) = (backend("rust"), backend("typescript"));
                tokio::join!(
                    rust.restore(&root, &rust_backend, NO_EXEC),
                    typescript.restore(&root, &typescript_backend, NO_EXEC)
                )
            }
        };

        let (_dir, root) = kept(&files);
        let tools = Arc::new(FakeTools::default());
        both(&tools, root).await;
        assert_eq!(tools.runs().len(), 2);
        assert!(!tools.overlapped.load(Ordering::SeqCst), "two restores ran at once");

        // Without the tree's lock the same two would overlap, so the check above can fail.
        let plain = tree(&files);
        let unlocked = Arc::new(FakeTools::default());
        both(&unlocked, plain.path().to_owned()).await;
        assert!(unlocked.overlapped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_tree_outside_the_kept_layout_is_restored_every_time_and_has_no_fingerprint() {
        let dir = tree(&[("Cargo.toml", "")]);
        let tools = Arc::new(FakeTools::default());
        let restored = restorer(&tools).restore(dir.path(), &backend("rust"), NO_EXEC).await;
        restorer(&tools).restore(dir.path(), &backend("rust"), NO_EXEC).await;
        assert_eq!(tools.runs().len(), 2);
        assert_eq!(restored.fingerprint, None);
    }

    #[tokio::test]
    async fn a_failure_in_one_project_does_not_stop_the_others_and_names_its_directory() {
        if Host::current().find("cargo", is_file).is_none() {
            eprintln!("skipped: cargo is not installed");
            return;
        }
        let dir = tree(&[
            ("bad/Cargo.toml", "not a manifest"),
            (
                "good/Cargo.toml",
                "[package]\nname = \"g\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"lib.rs\"\n",
            ),
            ("good/lib.rs", ""),
        ]);
        let restored = ToolRestorer::new()
            .restore(dir.path(), &backend("rust"), AnalysisPolicy { allow_executing: false })
            .await;
        assert_eq!(restored.failures.len(), 1, "{restored:?}");
        assert_eq!(restored.failures[0].project, "bad");
    }
}
