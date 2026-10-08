//! `vet_app::ports::ServerResolver`: finds each language's server and installs
//! the pinned ones into Vet's data directory (§7.1). Programs are located and
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
use vet_semantic::ports::BackendId;

use crate::budget::{Budget, PROBES};
use crate::exec::{
    Captured, Host, LineSplitter, OUTPUT_TAIL_LINES, Tail, TokioRunner, capture as run_to_end, dotnet_env, is_file,
};
use crate::language::{self, Compiler, Install, Language};

/// How long a `--version` probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ToolResolver {
    data_dir: PathBuf,
    host: Host,
    /// The processes that only answer a question: versions and the project's toolchain.
    probes: Budget,
}

impl ToolResolver {
    /// Installs go below `data_dir/servers`.
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_host(data_dir, Host::current())
    }

    pub fn with_host(data_dir: PathBuf, host: Host) -> Self {
        Self {
            data_dir,
            host,
            probes: Budget::new(PROBES),
        }
    }

    /// Runs its probes on `probes`.
    pub fn probing(mut self, probes: Budget) -> Self {
        self.probes = probes;
        self
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
        let dir = self.install_dir(spec);
        match spec.install {
            Install::No => None,
            Install::Npm { packages } => package_version(&dir.join("node_modules").join(packages[0].0)),
            Install::DotnetTool { package, .. } => {
                let args = [
                    "tool".to_owned(),
                    "list".into(),
                    "--tool-path".into(),
                    dir.to_string_lossy().into_owned(),
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
        let Some(rustup) = self.host.find("rustup", is_file) else {
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
    /// the fallback Vet installed beside the server.
    fn tsserver(&self, spec: &Language, project: &Path) -> Option<(PathBuf, String)> {
        let project_package = project.join("node_modules").join("typescript");
        let fallback = self.install_dir(spec).join("node_modules").join("typescript");
        [(project_package, "the project's"), (fallback, "Vet's fallback")]
            .into_iter()
            .find_map(|(package, whose)| {
                let tsserver = package.join("lib").join("tsserver.js");
                tsserver.is_file().then(|| {
                    let version = package_version(&package).map_or(String::new(), |v| format!(" {v}"));
                    (tsserver, format!("{whose} typescript{version}"))
                })
            })
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
    async fn resolve(&self, language: &BackendId, project: &Path, custom: Option<&str>) -> ServerState {
        let Some(spec) = Self::spec(language) else {
            return ServerState::Missing {
                reason: format!("there is no language server for {}", language.0),
            };
        };
        let mut looked = Vec::new();
        let mut found: Option<(PathBuf, ServerOrigin)> = None;

        if let Some(custom) = custom {
            match self.host.find(custom, is_file) {
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
            match self.installed(spec) {
                Some(path) => found = Some((path, ServerOrigin::Installed)),
                None if !matches!(spec.install, Install::No) => {
                    looked.push(format!("not installed in {}", self.install_dir(spec).display()));
                }
                None => {}
            }
        }
        if found.is_none() {
            match self.host.find(spec.binary, is_file) {
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
            tsserver = Some(match self.tsserver(spec, project) {
                Some((path, whose)) => {
                    compiler = Some(path.clone());
                    Tsserver::Using { whose, path }
                }
                None => Tsserver::Missing {
                    reason: "the project has no typescript with lib/tsserver.js (TypeScript 7 ships none) \
                             and Vet's fallback is not installed"
                        .into(),
                },
            });
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

    fn extensions(&self, language: &BackendId) -> Vec<String> {
        Self::spec(language).map_or_else(Vec::new, |s| s.files.iter().map(|(ext, _)| (*ext).to_owned()).collect())
    }

    fn pinned(&self, language: &BackendId) -> Option<String> {
        match Self::spec(language)?.install {
            Install::No => None,
            Install::Npm { packages } => Some(packages[0].1.to_owned()),
            Install::DotnetTool { version, .. } => Some(version.to_owned()),
        }
    }

    fn install_blocker(&self, language: &BackendId) -> Option<String> {
        let tool = match Self::spec(language)?.install {
            Install::No => return None,
            Install::Npm { .. } => ("npm", "installing needs npm (Node.js), which is not on PATH"),
            Install::DotnetTool { .. } => (
                "dotnet",
                "installing needs the .NET SDK (`dotnet`), which is not on PATH",
            ),
        };
        self.host.find(tool.0, is_file).is_none().then(|| tool.1.to_owned())
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
                let _ = std::fs::remove_dir_all(&dir);
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
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
        if code != Some(0) || self.installed(spec).is_none() {
            let status = code.map_or("was killed".to_owned(), |c| format!("exited with {c}"));
            return Err(format!("{program} {status}:\n{}", tail.text()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::parse_pathext;
    use crate::language::PYRIGHT_VERSION;

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

        let resolver = ToolResolver::with_host(data.path().to_owned(), host(false, vec![]));
        let project = tempfile::tempdir().unwrap();
        let server = found(resolver.resolve(&language("typescript"), project.path(), None).await);
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
        let resolver = ToolResolver::with_host(data.path().to_owned(), host(true, vec![]));
        let project = tempfile::tempdir().unwrap();
        let server = found(resolver.resolve(&language("python"), project.path(), None).await);
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

        let resolver = ToolResolver::with_host(data.path().to_owned(), host(false, vec![path_dir.path().to_owned()]));
        let server = found(resolver.resolve(&language("typescript"), project.path(), None).await);
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
        let server = found(resolver.resolve(&language("typescript"), bare.path(), None).await);
        assert!(server.command.compiler.is_none());
        assert!(
            matches!(server.tsserver, Some(Tsserver::Missing { .. })),
            "{:?}",
            server.tsserver
        );

        let fallback = data.path().join("servers/typescript/node_modules/typescript");
        touch(&fallback.join("lib/tsserver.js"));
        let server = found(resolver.resolve(&language("typescript"), bare.path(), None).await);
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
        let server = found(resolver.resolve(&language("typescript"), project.path(), None).await);
        assert!(
            matches!(&server.tsserver, Some(Tsserver::Using { whose, .. }) if whose.contains("the project's typescript 5.9.2")),
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
        let resolver = ToolResolver::with_host(data.path().to_owned(), host(false, vec![path_dir.path().to_owned()]));
        let project = tempfile::tempdir().unwrap();

        let custom = tool.to_string_lossy().into_owned();
        let server = found(resolver.resolve(&language("rust"), project.path(), Some(&custom)).await);
        assert_eq!(server.origin, ServerOrigin::Custom);
        assert_eq!(server.command.program, custom);

        let state = resolver
            .resolve(&language("rust"), project.path(), Some("/no/such/ra"))
            .await;
        let ServerState::Missing { reason } = state else {
            panic!("{state:?}")
        };
        assert!(reason.contains("/no/such/ra"), "{reason}");
    }

    #[tokio::test]
    async fn a_missing_server_says_where_it_looked_and_what_to_do() {
        let data = tempfile::tempdir().unwrap();
        let resolver = ToolResolver::with_host(data.path().to_owned(), host(false, vec![]));
        let project = tempfile::tempdir().unwrap();
        let ServerState::Missing { reason } = resolver.resolve(&language("python"), project.path(), None).await else {
            panic!("found");
        };
        assert!(
            reason.contains("pyright-langserver") && reason.contains("not on PATH"),
            "{reason}"
        );
        assert!(reason.contains("settings"), "{reason}");
        assert_eq!(resolver.pinned(&language("python")).as_deref(), Some(PYRIGHT_VERSION));
        assert_eq!(resolver.pinned(&language("rust")), None);
        assert!(resolver.install_blocker(&language("python")).unwrap().contains("npm"));
        assert!(
            resolver
                .install_blocker(&language("csharp"))
                .unwrap()
                .contains("dotnet")
        );
    }

    #[tokio::test]
    async fn the_project_toolchain_is_asked_for_rust_analyzer() {
        if Host::current().find("rustup", is_file).is_none() {
            eprintln!("skipped: rustup is not installed");
            return;
        }
        let data = tempfile::tempdir().unwrap();
        let resolver = ToolResolver::with_host(data.path().to_owned(), Host::current());
        let project = tempfile::tempdir().unwrap();
        let state = resolver.resolve(&language("rust"), project.path(), None).await;
        // Either rustup knows rust-analyzer for the toolchain, or PATH does; both are reported with an origin.
        match state {
            ServerState::Found(f) => assert!(matches!(f.origin, ServerOrigin::Project | ServerOrigin::Path)),
            ServerState::Missing { reason } => assert!(reason.contains("rustup"), "{reason}"),
        }
    }
}
