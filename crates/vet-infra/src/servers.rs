//! `vet_app::ports::ServerResolver`: finds each language's server and installs
//! the pinned ones into Vet's data directory (§6.1). Programs are located and
//! invoked through [`crate::exec`], so `PATHEXT` and `cmd /c` live in one place.
//!
//! Where a server comes from, in order: the user's custom path; for Rust the
//! project's toolchain (`rustup which`, which honors `rust-toolchain` files); a
//! pinned install in the data directory; `PATH`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use vet_app::ports::{
    Command, FoundServer, ProcessEvent, ProcessRunner, RunningProcess, ServerCommand, ServerOrigin, ServerResolver,
    ServerState, Tsserver,
};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::BackendId;

use crate::budget::{Budget, Budgets};
use crate::exec::{
    Captured, Host, LineSplitter, OUTPUT_TAIL_LINES, Tail, TokioRunner, capture as run_to_end, dotnet_env, is_file,
};
use crate::language::{self, Compiler, Install, Language};

/// Where the Angular server finds its language service, below a `node_modules`.
pub(crate) const ANGULAR_SERVICE: &str = "@angular/language-service";

/// How long a `--version` probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Where each language's server comes from and how it is installed. Cheap to clone: clones
/// share the budgets.
#[derive(Clone)]
pub struct ToolResolver {
    data_dir: PathBuf,
    host: Host,
    /// The processes that only answer a question: versions and the project's toolchain.
    probes: Budget,
    /// The file lookups of a resolution.
    io: Budget,
}

impl ToolResolver {
    /// Installs go below `data_dir/servers`; its probes and file lookups draw on `budgets`.
    pub fn new(data_dir: PathBuf, budgets: &Budgets) -> Self {
        Self::with_host(data_dir, Host::current(), budgets)
    }

    pub fn with_host(data_dir: PathBuf, host: Host, budgets: &Budgets) -> Self {
        Self {
            data_dir,
            host,
            probes: budgets.probes.clone(),
            io: budgets.semantic_io.clone(),
        }
    }

    /// A look at files or directories (`host.find`, a check for an install): answered
    /// on the lane's blocking threads, so a slow directory never holds an async worker
    /// while a server starts.
    async fn looking<T: Send + 'static>(&self, look: impl FnOnce(&Self) -> T + Send + 'static) -> T {
        let this = self.clone();
        self.io.blocking(move || look(&this)).await.expect("a blocking task")
    }

    fn spec(language: &BackendId) -> Option<&'static Language> {
        language::by_id(&language.0)
    }

    fn install_dir(&self, spec: &Language) -> PathBuf {
        self.data_dir.join("servers").join(spec.id)
    }

    /// The server Vet installed, if it did.
    fn installed(&self, spec: &Language) -> Option<PathBuf> {
        let dir = self.install_dir(spec);
        match spec.install {
            Install::No => None,
            Install::Npm { .. } => self
                .host
                .find_in(&dir.join("node_modules").join(".bin"), spec.binary, is_file),
            Install::DotnetTool { .. } => self.host.find_in(&dir, spec.binary, is_file),
        }
    }

    /// The version of a Vet-installed server: an npm package's `package.json`,
    /// or what `dotnet tool list --tool-path` reports.
    async fn installed_version(&self, spec: &Language) -> Option<String> {
        match spec.install {
            Install::No => None,
            Install::Npm { packages } => {
                let manifest = self.install_dir(spec).join("node_modules").join(packages[0].0);
                self.looking(move |_| package_version(&manifest)).await
            }
            Install::DotnetTool { package, .. } => {
                let args = [
                    "tool".to_owned(),
                    "list".into(),
                    "--tool-path".into(),
                    self.install_dir(spec).to_string_lossy().into_owned(),
                ];
                let out = capture(&self.probes, "dotnet", &args, None)
                    .await
                    .ok()
                    .filter(|o| o.ok)?;
                tool_list_version(&out.stdout, package)
            }
        }
    }

    async fn project_rust_analyzer(&self, project: &Path) -> Result<PathBuf, String> {
        let Some(rustup) = self.looking(|t| t.host.find("rustup", is_file)).await else {
            return Err("rustup is not on PATH".into());
        };
        let args = ["which".to_owned(), "rust-analyzer".to_owned()];
        let out = capture(&self.probes, &rustup.to_string_lossy(), &args, Some(project)).await?;
        if !out.ok {
            let why = out.stderr.lines().next().unwrap_or("it failed").trim().to_owned();
            return Err(format!("`rustup which rust-analyzer` failed: {why}"));
        }
        let path = PathBuf::from(out.stdout.trim());
        if is_file(&path) {
            Ok(path)
        } else {
            Err(format!(
                "`rustup which rust-analyzer` named {}, which is not a file",
                path.display()
            ))
        }
    }

    /// The `tsserver` the language server must use: the project's own
    /// `typescript`, so its answers match what the project compiles with, else
    /// the fallback Vet installed beside the server. The project's is a program node
    /// runs, so it is used only where the policy lets analysis execute project code; without
    /// that, the typescript installed beside the server itself serves (a user's own install).
    fn tsserver(
        &self,
        spec: &Language,
        project: &Path,
        policy: AnalysisPolicy,
        server: &Path,
    ) -> Option<(PathBuf, String)> {
        let project_package = project.join("node_modules").join("typescript");
        let fallback = self.install_dir(spec).join("node_modules").join("typescript");
        let mut candidates = Vec::new();
        if policy.allow_executing {
            candidates.push((project_package, "the project's"));
        }
        candidates.push((fallback, "Vet's fallback"));
        if !policy.allow_executing {
            candidates.extend(modules_above(server).map(|dir| (dir.join("typescript"), "the server's")));
        }
        candidates.into_iter().find_map(|(package, whose)| {
            let tsserver = package.join("lib").join("tsserver.js");
            tsserver.is_file().then(|| {
                let version = package_version(&package).map_or(String::new(), |v| format!(" {v}"));
                (tsserver, format!("{whose} typescript{version}"))
            })
        })
    }

    /// Whether the Angular server `program` has an `@angular/language-service` of its own to load:
    /// the one Vet installed, or one beside the server.
    fn has_angular_service(&self, spec: &Language, program: &Path) -> bool {
        let install = self.install_dir(spec).join("node_modules");
        std::iter::once(install)
            .chain(modules_above(program))
            .any(|dir| dir.join(ANGULAR_SERVICE).is_dir())
    }

    fn missing_reason(&self, spec: &Language, looked: &[String]) -> String {
        let hint = match spec.install {
            Install::No => "Install it with `rustup component add rust-analyzer`, or set a path.".to_owned(),
            _ => "Install it from the language servers settings, or set a path.".to_owned(),
        };
        format!("{} was not found ({}). {hint}", spec.binary, looked.join("; "))
    }
}

/// What `program --version` says in its first line, for a language whose server has such a flag.
pub(crate) async fn probe_version(probes: &Budget, spec: &Language, program: &Path) -> Option<String> {
    let flag = spec.version_flag?;
    let out = capture(probes, &program.to_string_lossy(), &[flag.to_owned()], None)
        .await
        .ok()?;
    out.ok
        .then(|| out.stdout.lines().next().unwrap_or("").trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// The `node_modules` directories above `program`, once links are resolved, nearest first: where
/// the packages installed beside a server are.
fn modules_above(program: &Path) -> impl Iterator<Item = PathBuf> {
    let real = std::fs::canonicalize(program).unwrap_or_else(|_| program.to_owned());
    real.ancestors()
        .skip(1)
        .filter(|dir| dir.file_name().is_some_and(|name| name == "node_modules"))
        .map(Path::to_owned)
        .collect::<Vec<_>>()
        .into_iter()
}

/// The `version` of the npm package in `package`.
fn package_version(package: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(package.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    Some(manifest.get("version")?.as_str()?.to_owned())
}

/// The version of `package` in the table `dotnet tool list` prints: a header
/// row (Package Id, Version, Commands), a row of dashes, then one row per tool.
fn tool_list_version(table: &str, package: &str) -> Option<String> {
    let mut rows = table.lines().skip_while(|l| !l.starts_with("---")).skip(1);
    rows.find_map(|row| {
        let mut columns = row.split_whitespace();
        let (id, version) = (columns.next()?, columns.next()?);
        id.eq_ignore_ascii_case(package).then(|| version.to_owned())
    })
}

pub(crate) async fn capture(
    probes: &Budget,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<Captured, String> {
    run_to_end(probes, program, args, cwd, &[], PROBE_TIMEOUT).await
}

impl ServerResolver for ToolResolver {
    async fn resolve(
        &self,
        language: &BackendId,
        project: &Path,
        custom: Option<&str>,
        policy: AnalysisPolicy,
    ) -> ServerState {
        let Some(spec) = Self::spec(language) else {
            return ServerState::Missing {
                reason: format!("there is no language server for {}", language.0),
            };
        };
        let mut looked = Vec::new();
        let mut found: Option<(PathBuf, ServerOrigin)> = None;

        if let Some(custom) = custom {
            let path = custom.to_owned();
            match self.looking(move |t| t.host.find(&path, is_file)).await {
                Some(path) => found = Some((path, ServerOrigin::Custom)),
                None => {
                    return ServerState::Missing {
                        reason: format!("the custom path {custom} is not a file"),
                    };
                }
            }
        }
        if found.is_none() && spec.project_toolchain {
            match self.project_rust_analyzer(project).await {
                Ok(path) => found = Some((path, ServerOrigin::Project)),
                Err(why) => looked.push(why),
            }
        }
        if found.is_none() {
            match self.looking(|t| t.installed(spec)).await {
                Some(path) => found = Some((path, ServerOrigin::Installed)),
                None if !matches!(spec.install, Install::No) => {
                    looked.push(format!("not installed in {}", self.install_dir(spec).display()));
                }
                None => {}
            }
        }
        if found.is_none() {
            match self.looking(|t| t.host.find(spec.binary, is_file)).await {
                Some(path) => found = Some((path, ServerOrigin::Path)),
                None => looked.push("not on PATH".into()),
            }
        }
        let Some((path, origin)) = found else {
            return ServerState::Missing {
                reason: self.missing_reason(spec, &looked),
            };
        };

        let mut tsserver = None;
        let mut compiler = None;
        if spec.compiler == Compiler::TypeScript {
            let (project, found) = (project.to_owned(), path.clone());
            tsserver = Some(
                match self.looking(move |t| t.tsserver(spec, &project, policy, &found)).await {
                    Some((path, whose)) => {
                        compiler = Some(path.clone());
                        Tsserver::Using { whose, path }
                    }
                    // Left to itself the server would look for the project's typescript.
                    None if !policy.allow_executing => {
                        return ServerState::Missing {
                            reason: "the project's typescript is not run for what is not trusted, and neither \
                                 Vet's own nor one beside the server is installed. Install Vet's from the \
                                 language servers settings."
                                .into(),
                        };
                    }
                    None => Tsserver::Missing {
                        reason: "the project has no typescript with lib/tsserver.js (TypeScript 7 ships none) \
                             and Vet's fallback is not installed"
                            .into(),
                    },
                },
            );
        }
        if spec.id == language::ANGULAR.id
            && !policy.allow_executing
            && !self
                .looking({
                    let found = path.clone();
                    move |t| t.has_angular_service(spec, &found)
                })
                .await
        {
            return ServerState::Missing {
                reason: "the project's @angular/language-service is not run for what is not trusted, and \
                         Vet's own is not installed. Install it from the language servers settings."
                    .into(),
            };
        }
        ServerState::Found(FoundServer {
            command: ServerCommand {
                program: path.to_string_lossy().into_owned(),
                compiler,
            },
            origin,
            version: None,
            tsserver,
        })
    }

    async fn version(&self, language: &BackendId, found: &FoundServer) -> Option<String> {
        let spec = Self::spec(language)?;
        match found.origin {
            ServerOrigin::Installed => self.installed_version(spec).await,
            _ => probe_version(&self.probes, spec, Path::new(&found.command.program)).await,
        }
    }

    fn title(&self, language: &BackendId) -> String {
        Self::spec(language).map_or_else(|| language.0.clone(), |s| s.title.to_owned())
    }

    fn display_name(&self, language: &BackendId) -> String {
        Self::spec(language).map_or_else(|| language.0.clone(), |s| s.display_name.to_owned())
    }

    fn pinned(&self, language: &BackendId) -> Option<String> {
        match Self::spec(language)?.install {
            Install::No => None,
            Install::Npm { packages } => Some(packages[0].1.to_owned()),
            Install::DotnetTool { version, .. } => Some(version.to_owned()),
        }
    }

    async fn install_blocker(&self, language: &BackendId) -> Option<String> {
        let tool = match Self::spec(language)?.install {
            Install::No => return None,
            Install::Npm { .. } => ("npm", "installing needs npm (Node.js), which is not on PATH"),
            Install::DotnetTool { .. } => (
                "dotnet",
                "installing needs the .NET SDK (`dotnet`), which is not on PATH",
            ),
        };
        self.looking(|t| t.host.find(tool.0, is_file).is_none())
            .await
            .then(|| tool.1.to_owned())
    }

    async fn install(&self, language: &BackendId, progress: &(dyn Fn(&str) + Send + Sync)) -> Result<(), String> {
        let spec = Self::spec(language).ok_or_else(|| format!("there is no language server for {}", language.0))?;
        let dir = self.install_dir(spec);
        let dir_arg = dir.to_string_lossy().into_owned();
        let (program, args) = match spec.install {
            Install::No => return Err("Vet does not install this server".into()),
            Install::Npm { packages } => {
                let mut args: Vec<String> = [
                    "install",
                    "--prefix",
                    &dir_arg,
                    "--no-audit",
                    "--no-fund",
                    "--save-exact",
                ]
                .map(str::to_owned)
                .into();
                args.extend(packages.iter().map(|(package, version)| format!("{package}@{version}")));
                ("npm", args)
            }
            Install::DotnetTool { package, version } => {
                // A tool path holds one version; start clean so the pin replaces any other.
                let _ = self.looking(|t| std::fs::remove_dir_all(t.install_dir(spec))).await;
                (
                    "dotnet",
                    vec![
                        "tool".to_owned(),
                        "install".into(),
                        package.into(),
                        "--tool-path".into(),
                        dir_arg,
                        "--version".into(),
                        version.into(),
                    ],
                )
            }
        };
        self.looking(move |t| {
            std::fs::create_dir_all(t.install_dir(spec)).map_err(|e| format!("{}: {e}", t.install_dir(spec).display()))
        })
        .await?;
        progress(&format!("{program} {}", args.join(" ")));
        let command = Command {
            program: program.into(),
            args,
            cwd: dir.clone(),
            env: dotnet_env(),
        };
        let mut process = TokioRunner.spawn(&command)?;
        let mut tail = Tail::new(OUTPUT_TAIL_LINES);
        let mut streams = [LineSplitter::default(), LineSplitter::default()];
        let code = loop {
            let (stream, bytes) = match process.next().await {
                ProcessEvent::Stdout(b) => (0, b),
                ProcessEvent::Stderr(b) => (1, b),
                ProcessEvent::Exit(code) => break code,
            };
            for line in streams[stream].feed(&bytes) {
                if !line.is_empty() {
                    progress(&line);
                    tail.push(line);
                }
            }
        };
        if code != Some(0) || self.looking(|t| t.installed(spec)).await.is_none() {
            let status = code.map_or("was killed".to_owned(), |c| format!("exited with {c}"));
            return Err(format!("{program} {status}:\n{}", tail.text()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::exec::parse_pathext;

    const TRUSTED: AnalysisPolicy = AnalysisPolicy::EXECUTING;

    fn host(windows: bool, path: Vec<PathBuf>) -> Host {
        Host {
            windows,
            path,
            pathext: parse_pathext(None),
        }
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn language(name: &str) -> BackendId {
        BackendId(name.into())
    }

    fn found(state: ServerState) -> FoundServer {
        match state {
            ServerState::Found(found) => found,
            ServerState::Missing { reason } => panic!("{reason}"),
        }
    }

    #[tokio::test]
    async fn the_pinned_install_in_the_data_dir_is_found_and_its_version_read() {
        let data = tempfile::tempdir().unwrap();
        let bin = data
            .path()
            .join("servers/typescript/node_modules/.bin/typescript-language-server");
        touch(&bin);
        let package = data
            .path()
            .join("servers/typescript/node_modules/typescript-language-server");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("package.json"), r#"{"version":"6.0.1"}"#).unwrap();

        let resolver = ToolResolver::with_host(data.path().to_owned(), host(false, vec![]), &Budgets::default());
        let project = tempfile::tempdir().unwrap();
        let server = found(
            resolver
                .resolve(&language("typescript"), project.path(), None, TRUSTED)
                .await,
        );
        assert_eq!(server.origin, ServerOrigin::Installed);
        assert_eq!(server.command.program, bin.to_string_lossy());
        assert_eq!(server.version, None);
        assert_eq!(
            resolver.version(&language("typescript"), &server).await.as_deref(),
            Some("6.0.1")
        );
    }

    #[test]
    fn the_dotnet_tool_table_is_read_for_the_package() {
        let table = "Package Id                  Version               Commands              \n\
                     ------------------------------------------------------------------------\n\
                     other-tool                  1.0.0                 other\n\
                     roslyn-language-server      5.12.0-1.26475.2      roslyn-language-server\n";
        assert_eq!(
            tool_list_version(table, "roslyn-language-server").as_deref(),
            Some("5.12.0-1.26475.2")
        );
        assert_eq!(tool_list_version(table, "missing"), None);
        assert_eq!(tool_list_version("", "roslyn-language-server"), None);
    }

    #[tokio::test]
    async fn windows_finds_the_cmd_shim_of_an_install() {
        let data = tempfile::tempdir().unwrap();
        let shim = data
            .path()
            .join("servers/python/node_modules/.bin/pyright-langserver.CMD");
        touch(&shim);
        let resolver = ToolResolver::with_host(data.path().to_owned(), host(true, vec![]), &Budgets::default());
        let project = tempfile::tempdir().unwrap();
        let server = found(
            resolver
                .resolve(&language("python"), project.path(), None, TRUSTED)
                .await,
        );
        assert_eq!(server.command.program, shim.to_string_lossy());
    }

    #[tokio::test]
    async fn the_project_typescript_is_passed_as_the_tsserver() {
        let data = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        touch(&path_dir.path().join("typescript-language-server"));
        let project = tempfile::tempdir().unwrap();
        touch(&project.path().join("node_modules/typescript/lib/tsserver.js"));
        std::fs::write(
            project.path().join("node_modules/typescript/package.json"),
            r#"{"version":"5.9.2"}"#,
        )
        .unwrap();

        let resolver = ToolResolver::with_host(
            data.path().to_owned(),
            host(false, vec![path_dir.path().to_owned()]),
            &Budgets::default(),
        );
        let server = found(
            resolver
                .resolve(&language("typescript"), project.path(), None, TRUSTED)
                .await,
        );
        assert_eq!(server.origin, ServerOrigin::Path);
        let tsserver = project.path().join("node_modules/typescript/lib/tsserver.js");
        assert_eq!(server.command.compiler.as_deref(), Some(tsserver.as_path()));
        assert_eq!(
            server.tsserver,
            Some(Tsserver::Using {
                whose: "the project's typescript 5.9.2".into(),
                path: tsserver,
            })
        );

        // A project without a usable typescript (none, or TypeScript 7, which has no tsserver.js).
        let bare = tempfile::tempdir().unwrap();
        let server = found(
            resolver
                .resolve(&language("typescript"), bare.path(), None, TRUSTED)
                .await,
        );
        assert!(server.command.compiler.is_none());
        assert!(
            matches!(server.tsserver, Some(Tsserver::Missing { .. })),
            "{:?}",
            server.tsserver
        );

        let fallback = data.path().join("servers/typescript/node_modules/typescript");
        touch(&fallback.join("lib/tsserver.js"));
        let server = found(
            resolver
                .resolve(&language("typescript"), bare.path(), None, TRUSTED)
                .await,
        );
        assert_eq!(
            server.command.compiler.as_deref(),
            Some(fallback.join("lib/tsserver.js").as_path())
        );
        assert!(
            matches!(&server.tsserver, Some(Tsserver::Using { whose, .. }) if whose.contains("Vet's fallback")),
            "{:?}",
            server.tsserver
        );
        // The project's own still wins over the fallback.
        let server = found(
            resolver
                .resolve(&language("typescript"), project.path(), None, TRUSTED)
                .await,
        );
        assert!(
            matches!(&server.tsserver, Some(Tsserver::Using { whose, .. }) if whose.contains("the project's typescript 5.9.2")),
            "{:?}",
            server.tsserver
        );
    }

    #[tokio::test]
    async fn without_trust_only_vets_own_typescript_serves() {
        let data = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        touch(&path_dir.path().join("typescript-language-server"));
        let project = tempfile::tempdir().unwrap();
        touch(&project.path().join("node_modules/typescript/lib/tsserver.js"));
        let resolver = ToolResolver::with_host(
            data.path().to_owned(),
            host(false, vec![path_dir.path().to_owned()]),
            &Budgets::default(),
        );
        let typescript = language("typescript");

        // With no typescript of Vet's own, the server would look for the project's itself.
        let state = resolver
            .resolve(&typescript, project.path(), None, AnalysisPolicy::LIMITED)
            .await;
        let ServerState::Missing { reason } = state else {
            panic!("{state:?}")
        };
        assert!(reason.contains("not run"), "{reason}");

        let fallback = data.path().join("servers/typescript/node_modules/typescript");
        touch(&fallback.join("lib/tsserver.js"));
        let server = found(
            resolver
                .resolve(&typescript, project.path(), None, AnalysisPolicy::LIMITED)
                .await,
        );
        assert_eq!(
            server.command.compiler.as_deref(),
            Some(fallback.join("lib/tsserver.js").as_path())
        );
        assert!(
            matches!(&server.tsserver, Some(Tsserver::Using { whose, .. }) if whose.contains("Vet's fallback")),
            "{:?}",
            server.tsserver
        );
    }

    #[tokio::test]
    async fn without_trust_angular_needs_a_language_service_that_is_not_the_projects() {
        let data = tempfile::tempdir().unwrap();
        let path_dir = tempfile::tempdir().unwrap();
        touch(&path_dir.path().join("ngserver"));
        let project = tempfile::tempdir().unwrap();
        touch(&project.path().join("node_modules/typescript/lib/tsserver.js"));
        touch(&project.path().join("node_modules/@angular/language-service/index.js"));
        let resolver = ToolResolver::with_host(
            data.path().to_owned(),
            host(false, vec![path_dir.path().to_owned()]),
            &Budgets::default(),
        );
        let angular = language("angular");
        let own = data.path().join("servers/angular/node_modules");
        touch(&own.join("typescript/lib/tsserver.js"));

        let state = resolver
            .resolve(&angular, project.path(), None, AnalysisPolicy::LIMITED)
            .await;
        let ServerState::Missing { reason } = state else {
            panic!("{state:?}")
        };
        assert!(reason.contains("language-service"), "{reason}");

        touch(&own.join(ANGULAR_SERVICE).join("index.js"));
        found(
            resolver
                .resolve(&angular, project.path(), None, AnalysisPolicy::LIMITED)
                .await,
        );
        // Trusted, the project's own serves and Vet's is not needed.
        std::fs::remove_dir_all(own.join("@angular")).unwrap();
        found(
            resolver
                .resolve(&angular, project.path(), None, AnalysisPolicy::EXECUTING)
                .await,
        );
    }

    #[tokio::test]
    async fn without_trust_a_user_installed_server_is_served_by_the_typescript_beside_it() {
        let data = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let modules = std::fs::canonicalize(global.path()).unwrap().join("node_modules");
        let bin = modules.join("typescript-language-server/bin");
        touch(&bin.join("typescript-language-server"));
        let project = tempfile::tempdir().unwrap();
        touch(&project.path().join("node_modules/typescript/lib/tsserver.js"));
        let resolver = ToolResolver::with_host(data.path().to_owned(), host(false, vec![bin]), &Budgets::default());

        let typescript = language("typescript");
        let state = resolver
            .resolve(&typescript, project.path(), None, AnalysisPolicy::LIMITED)
            .await;
        assert!(matches!(state, ServerState::Missing { .. }), "{state:?}");

        let beside = modules.join("typescript/lib/tsserver.js");
        touch(&beside);
        let server = found(
            resolver
                .resolve(&typescript, project.path(), None, AnalysisPolicy::LIMITED)
                .await,
        );
        assert_eq!(server.origin, ServerOrigin::Path);
        assert_eq!(server.command.compiler.as_deref(), Some(beside.as_path()));
        assert!(
            matches!(&server.tsserver, Some(Tsserver::Using { whose, .. }) if whose.contains("the server's")),
            "{:?}",
            server.tsserver
        );
    }

    #[tokio::test]
    async fn a_custom_path_wins_and_a_wrong_one_is_reported_not_skipped() {
        let data = tempfile::tempdir().unwrap();
        let tool = data.path().join("mine/ra");
        touch(&tool);
        let path_dir = tempfile::tempdir().unwrap();
        touch(&path_dir.path().join("rust-analyzer"));
        let resolver = ToolResolver::with_host(
            data.path().to_owned(),
            host(false, vec![path_dir.path().to_owned()]),
            &Budgets::default(),
        );
        let project = tempfile::tempdir().unwrap();

        let custom = tool.to_string_lossy().into_owned();
        let server = found(
            resolver
                .resolve(&language("rust"), project.path(), Some(&custom), TRUSTED)
                .await,
        );
        assert_eq!(server.origin, ServerOrigin::Custom);
        assert_eq!(server.command.program, custom);

        let state = resolver
            .resolve(&language("rust"), project.path(), Some("/no/such/ra"), TRUSTED)
            .await;
        let ServerState::Missing { reason } = state else {
            panic!("{state:?}")
        };
        assert!(reason.contains("/no/such/ra"), "{reason}");
    }

    #[tokio::test]
    async fn a_missing_server_says_where_it_looked_and_what_to_do() {
        let data = tempfile::tempdir().unwrap();
        let resolver = ToolResolver::with_host(data.path().to_owned(), host(false, vec![]), &Budgets::default());
        let project = tempfile::tempdir().unwrap();
        let ServerState::Missing { reason } = resolver
            .resolve(&language("python"), project.path(), None, TRUSTED)
            .await
        else {
            panic!("found");
        };
        assert!(
            reason.contains("pyright-langserver") && reason.contains("not on PATH"),
            "{reason}"
        );
        assert!(reason.contains("settings"), "{reason}");
        assert_eq!(resolver.pinned(&language("rust")), None);
        assert!(
            resolver
                .install_blocker(&language("python"))
                .await
                .unwrap()
                .contains("npm")
        );
        assert!(
            resolver
                .install_blocker(&language("csharp"))
                .await
                .unwrap()
                .contains("dotnet")
        );
    }

    /// A `rustup` on a PATH of its own that answers `which rust-analyzer` with `answer`, or fails.
    #[cfg(unix)]
    fn stub_rustup(bin: &Path, answer: Option<&Path>) {
        let script = match answer {
            Some(path) => format!("#!/bin/sh\necho '{}'\n", path.display()),
            None => "#!/bin/sh\necho 'error: no toolchain' >&2\nexit 1\n".to_owned(),
        };
        let rustup = bin.join("rustup");
        std::fs::write(&rustup, script).unwrap();
        std::fs::set_permissions(&rustup, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The system may scan a program the first time it runs, which the resolver's probe must not wait for.
        std::process::Command::new(&rustup).output().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_project_toolchain_is_asked_for_rust_analyzer_before_the_path() {
        let data = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let toolchain = tempfile::tempdir().unwrap();
        let project_ra = toolchain.path().join("rust-analyzer");
        touch(&project_ra);
        touch(&bin.path().join("rust-analyzer"));
        stub_rustup(bin.path(), Some(&project_ra));
        let resolver = ToolResolver::with_host(
            data.path().to_owned(),
            host(false, vec![bin.path().to_owned()]),
            &Budgets::default(),
        );
        let project = tempfile::tempdir().unwrap();

        let server = found(resolver.resolve(&language("rust"), project.path(), None, TRUSTED).await);
        assert_eq!(server.origin, ServerOrigin::Project);
        assert_eq!(server.command.program, project_ra.to_string_lossy());

        // A toolchain without the component leaves PATH to answer.
        stub_rustup(bin.path(), None);
        let server = found(resolver.resolve(&language("rust"), project.path(), None, TRUSTED).await);
        assert_eq!(server.origin, ServerOrigin::Path);
    }
}
