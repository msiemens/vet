//! `vet_app::ports::ServerResolver`: finds each language's server and installs
//! the pinned ones into Vet's data directory (§7.1). Programs are located and
//! invoked through [`crate::exec`], so `PATHEXT` and `cmd /c` live in one place.
//!
//! Where a server comes from, in order: the user's custom path; for Rust the
//! project's toolchain (`rustup which`, which honors `rust-toolchain` files); a
//! pinned install in the data directory; `PATH`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use vet_app::ports::{
    Command, FoundServer, ProcessEvent, ProcessRunner, RunningProcess, ServerCommand, ServerOrigin, ServerResolver,
    ServerState,
};
use vet_semantic::ports::BackendId;

use crate::exec::{Host, TokioRunner, invocation, is_file};

/// Pinned versions of the servers Vet installs. Bump with a check that the
/// server still has every required capability (§7.1).
pub const TYPESCRIPT_LANGUAGE_SERVER_VERSION: &str = "6.0.1";
pub const PYRIGHT_VERSION: &str = "1.1.414";
pub const ROSLYN_LANGUAGE_SERVER_VERSION: &str = "5.12.0-1.26475.2";

/// How long a `--version` probe may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_TAIL_LINES: usize = 12;

/// How Vet installs a language's server.
#[derive(Clone, Copy)]
enum Install {
    /// Not installed by Vet.
    No,
    /// `npm install --prefix <dir> <package>@<version>`; `bin` is in `node_modules/.bin`.
    Npm {
        package: &'static str,
        version: &'static str,
    },
    /// `dotnet tool install <package> --tool-path <dir> --version <version>`.
    DotnetTool {
        package: &'static str,
        version: &'static str,
    },
}

struct Spec {
    language: &'static str,
    /// The executable's name, on `PATH` and in the install directory.
    binary: &'static str,
    install: Install,
    /// What to run to learn the version; `None` if the server has no such flag.
    version_flag: Option<&'static str>,
}

const SPECS: [Spec; 4] = [
    Spec {
        language: "rust",
        binary: "rust-analyzer",
        install: Install::No,
        version_flag: Some("--version"),
    },
    Spec {
        language: "typescript",
        binary: "typescript-language-server",
        install: Install::Npm {
            package: "typescript-language-server",
            version: TYPESCRIPT_LANGUAGE_SERVER_VERSION,
        },
        version_flag: Some("--version"),
    },
    Spec {
        language: "python",
        binary: "pyright-langserver",
        install: Install::Npm {
            package: "pyright",
            version: PYRIGHT_VERSION,
        },
        version_flag: None,
    },
    Spec {
        language: "csharp",
        binary: "roslyn-language-server",
        install: Install::DotnetTool {
            package: "roslyn-language-server",
            version: ROSLYN_LANGUAGE_SERVER_VERSION,
        },
        version_flag: None,
    },
];

pub struct ToolResolver {
    data_dir: PathBuf,
    host: Host,
}

impl ToolResolver {
    /// Installs go below `data_dir/servers`.
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_host(data_dir, Host::current())
    }

    pub fn with_host(data_dir: PathBuf, host: Host) -> Self {
        Self { data_dir, host }
    }

    fn spec(language: &BackendId) -> Option<&'static Spec> {
        SPECS.iter().find(|s| s.language == language.0)
    }

    fn install_dir(&self, spec: &Spec) -> PathBuf {
        self.data_dir.join("servers").join(spec.language)
    }

    /// The server Vet installed, if it did.
    fn installed(&self, spec: &Spec) -> Option<PathBuf> {
        let dir = self.install_dir(spec);
        match spec.install {
            Install::No => None,
            Install::Npm { .. } => self
                .host
                .find_in(&dir.join("node_modules").join(".bin"), spec.binary, is_file),
            Install::DotnetTool { .. } => self.host.find_in(&dir, spec.binary, is_file),
        }
    }

    /// The version of a Vet-installed server, from what the installer left behind.
    fn installed_version(&self, spec: &Spec) -> Option<String> {
        let dir = self.install_dir(spec);
        match spec.install {
            Install::No => None,
            Install::Npm { package, .. } => {
                let manifest =
                    std::fs::read_to_string(dir.join("node_modules").join(package).join("package.json")).ok()?;
                let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
                Some(manifest.get("version")?.as_str()?.to_owned())
            }
            Install::DotnetTool { package, .. } => std::fs::read_dir(dir.join(".store").join(package))
                .ok()?
                .flatten()
                .find_map(|e| e.file_name().into_string().ok()),
        }
    }

    async fn project_rust_analyzer(&self, project: &Path) -> Result<PathBuf, String> {
        let Some(rustup) = self.host.find("rustup", is_file) else {
            return Err("rustup is not on PATH".into());
        };
        let args = ["which".to_owned(), "rust-analyzer".to_owned()];
        let out = capture(&rustup.to_string_lossy(), &args, Some(project)).await?;
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

    /// The project's own `typescript`, which the language server must use so its
    /// answers match what the project compiles with.
    fn tsserver(project: &Path) -> Option<(PathBuf, Option<String>)> {
        let package = project.join("node_modules").join("typescript");
        let tsserver = package.join("lib").join("tsserver.js");
        if !tsserver.is_file() {
            return None;
        }
        let version = std::fs::read_to_string(package.join("package.json"))
            .ok()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(&m).ok())
            .and_then(|m| m.get("version")?.as_str().map(str::to_owned));
        Some((tsserver, version))
    }

    async fn probe_version(spec: &Spec, program: &Path) -> Option<String> {
        let flag = spec.version_flag?;
        let out = capture(&program.to_string_lossy(), &[flag.to_owned()], None)
            .await
            .ok()?;
        out.ok
            .then(|| out.stdout.lines().next().unwrap_or("").trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    fn missing_reason(&self, spec: &Spec, looked: &[String]) -> String {
        let hint = match spec.install {
            Install::No => "Install it with `rustup component add rust-analyzer`, or set a path.".to_owned(),
            _ => "Install it from the language servers settings, or set a path.".to_owned(),
        };
        format!("{} was not found ({}). {hint}", spec.binary, looked.join("; "))
    }
}

struct Captured {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Runs a program to completion with a time limit.
async fn capture(program: &str, args: &[String], cwd: Option<&Path>) -> Result<Captured, String> {
    let (program, args) = invocation(program, args);
    let mut command = TokioCommand::new(&program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|e| format!("{program}: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let run = async {
        let (mut out, mut err) = (String::new(), String::new());
        let (a, b) = tokio::join!(stdout.read_to_string(&mut out), stderr.read_to_string(&mut err));
        a.and(b).map_err(|e| e.to_string())?;
        let status = child.wait().await.map_err(|e| e.to_string())?;
        Ok(Captured {
            ok: status.success(),
            stdout: out,
            stderr: err,
        })
    };
    tokio::time::timeout(PROBE_TIMEOUT, run)
        .await
        .map_err(|_| format!("{program} did not answer within {}s", PROBE_TIMEOUT.as_secs()))?
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
        if found.is_none() && spec.language == "rust" {
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

        let version = match origin {
            ServerOrigin::Installed => self.installed_version(spec),
            _ => Self::probe_version(spec, &path).await,
        };
        let mut notes = Vec::new();
        let mut initialization_options = None;
        if spec.language == "typescript" {
            match Self::tsserver(project) {
                Some((tsserver, ts_version)) => {
                    let ts_version = ts_version.map_or(String::new(), |v| format!(" {v}"));
                    notes.push(format!(
                        "tsserver: the project's typescript{ts_version} ({})",
                        tsserver.display()
                    ));
                    initialization_options = Some(json!({"tsserver": {"path": tsserver.to_string_lossy()}}));
                }
                None => {
                    notes.push("tsserver: the server's own typescript (the project has none in node_modules)".into())
                }
            }
        }
        ServerState::Found(FoundServer {
            command: ServerCommand {
                program: path.to_string_lossy().into_owned(),
                initialization_options,
            },
            origin,
            version,
            notes,
        })
    }

    fn pinned(&self, language: &BackendId) -> Option<String> {
        match Self::spec(language)?.install {
            Install::No => None,
            Install::Npm { version, .. } | Install::DotnetTool { version, .. } => Some(version.to_owned()),
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
            Install::Npm { package, version } => (
                "npm",
                vec![
                    "install".to_owned(),
                    "--prefix".into(),
                    dir_arg,
                    "--no-audit".into(),
                    "--no-fund".into(),
                    "--save-exact".into(),
                    format!("{package}@{version}"),
                ],
            ),
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
            env: vec![
                ("DOTNET_NOLOGO".into(), "1".into()),
                ("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into()),
            ],
        };
        let mut process = TokioRunner.spawn(&command)?;
        let mut tail: Vec<String> = Vec::new();
        let mut partial: [Vec<u8>; 2] = Default::default();
        let code = loop {
            let (stream, bytes) = match process.next().await {
                ProcessEvent::Stdout(b) => (0, b),
                ProcessEvent::Stderr(b) => (1, b),
                ProcessEvent::Exit(code) => break code,
            };
            partial[stream].extend(bytes);
            while let Some(end) = partial[stream].iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = partial[stream].drain(..=end).collect();
                let line = String::from_utf8_lossy(&line).trim_end().to_owned();
                if !line.is_empty() {
                    progress(&line);
                    tail.push(line);
                    if tail.len() > OUTPUT_TAIL_LINES {
                        tail.remove(0);
                    }
                }
            }
        };
        if code != Some(0) || self.installed(spec).is_none() {
            let status = code.map_or("was killed".to_owned(), |c| format!("exited with {c}"));
            return Err(format!("{program} {status}:\n{}", tail.join("\n")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::parse_pathext;

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
        assert_eq!(server.version.as_deref(), Some("6.0.1"));
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
        let options = server.command.initialization_options.unwrap();
        let tsserver = project.path().join("node_modules/typescript/lib/tsserver.js");
        assert_eq!(options["tsserver"]["path"], tsserver.to_string_lossy().as_ref());
        assert!(server.notes[0].contains("5.9.2"), "{:?}", server.notes);

        let bare = tempfile::tempdir().unwrap();
        let server = found(resolver.resolve(&language("typescript"), bare.path(), None).await);
        assert!(server.command.initialization_options.is_none());
        assert!(server.notes[0].contains("none in node_modules"));
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
