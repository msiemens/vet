//! The languages Vet analyses, one entry each (§7.1): how the server runs, which
//! files it serves, how it is found and installed, and how a tree's dependencies
//! are restored for it. Adding a language is an entry here, a restore plan in
//! [`crate::restore`] if it has dependencies, and a one-time manual check of which
//! operations its server offers.

use vet_core::content::RepoPath;
use vet_semantic::ports::{Backend, BackendId, ServerConfig};

/// Pinned versions of the servers Vet installs. Bump with a check of which
/// operations the server offers (§7.1).
pub const TYPESCRIPT_LANGUAGE_SERVER_VERSION: &str = "6.0.1";
pub const PYRIGHT_VERSION: &str = "1.1.414";
pub const ROSLYN_LANGUAGE_SERVER_VERSION: &str = "5.12.0-1.26475.2";
pub const SVELTE_LANGUAGE_SERVER_VERSION: &str = "0.18.4";
pub const ANGULAR_LANGUAGE_SERVER_VERSION: &str = "22.2.2";
/// Angular's language service is loaded by its server from the project's packages first; this
/// is the one for projects without it.
pub const ANGULAR_LANGUAGE_SERVICE_VERSION: &str = "22.2.2";
pub const YAML_LANGUAGE_SERVER_VERSION: &str = "1.25.0";
/// `vscode-langservers-extracted` ships the JSON server (`vscode-json-language-server`).
pub const VSCODE_LANGSERVERS_VERSION: &str = "4.10.0";
pub const BASH_LANGUAGE_SERVER_VERSION: &str = "5.8.1";
pub const DOCKERFILE_LANGUAGE_SERVER_VERSION: &str = "0.15.0";
/// The `tsserver` for projects without a usable one: the language server
/// bundles none, and TypeScript 7 and later ship no `lib/tsserver.js`.
pub const FALLBACK_TYPESCRIPT_VERSION: &str = "6.0.3";

/// How Vet installs a language's server.
#[derive(Clone, Copy)]
pub enum Install {
    /// Not installed by Vet.
    No,
    /// `npm install --prefix <dir> <package>@<version>...`; the binary is in
    /// `node_modules/.bin`. The first package is the server, the others its companions.
    Npm {
        packages: &'static [(&'static str, &'static str)],
    },
    /// `dotnet tool install <package> --tool-path <dir> --version <version>`.
    DotnetTool {
        package: &'static str,
        version: &'static str,
    },
}

/// How the dependencies of a tree are restored for a language's server.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Restore {
    /// The language's files bring no dependencies the server needs.
    None,
    Cargo,
    Javascript,
    Python,
    Dotnet,
}

impl Restore {
    /// Languages with equal names share one restore of a tree.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Cargo => "cargo",
            Self::Javascript => "javascript",
            Self::Python => "python",
            Self::Dotnet => "dotnet",
        }
    }
}

/// A compiler the language server does not bring and must be told about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compiler {
    None,
    /// The project's `typescript`, or Vet's fallback beside the server.
    TypeScript,
}

/// How a server learns which JSON schema a file is checked against, and from where it may load it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchemaClient {
    /// The server checks nothing against schemas.
    None,
    /// `yaml-language-server`: `yaml.schemas`, the schema store and `$schema` modelines.
    Yaml,
    /// The JSON server: `json.schemas` and the `$schema` property.
    Json,
}

/// A file in the user's home that a server reads.
#[derive(Clone, Copy, Debug)]
pub enum UserFile {
    /// Relative to the home directory.
    Home(&'static str),
    /// In the directory `var` names, else in `fallback` below the home directory.
    Under {
        var: &'static str,
        fallback: &'static str,
        file: &'static str,
    },
}

/// What a language's server reads outside the repository. It shapes the
/// server's answers, so it is part of the server's identity (I2): an answer is
/// reused only while all of it is as it was. Every language lists its own, even
/// if the list is empty, so that adding a language means looking.
pub struct UserConfig {
    /// Files looked for in every directory above the root: configuration the server
    /// finds by walking up from the project (the root's own files are the snapshot's).
    pub above_root: &'static [&'static str],
    /// Configuration in the user's home, such as the user-level server settings.
    pub user_files: &'static [UserFile],
    /// Environment variables the server or its tools read.
    pub env: &'static [&'static str],
    /// Programs run in the root, whose output stands for the toolchain the server
    /// finds there: the one a `rust-toolchain` file or a `global.json` picks.
    pub toolchain: &'static [(&'static str, &'static [&'static str])],
}

pub struct Language {
    pub id: &'static str,
    /// The language's name as a person knows it.
    pub title: &'static str,
    /// The server's name as a person knows it.
    pub display_name: &'static str,
    /// The server's executable, on `PATH` and in the install directory.
    pub binary: &'static str,
    pub args: &'static [&'static str],
    /// The file extensions (without the dot) the server serves, each with its LSP `languageId`.
    pub files: &'static [(&'static str, &'static str)],
    /// The names of files it serves that have no extension of their own, each with its
    /// `languageId` (see `ServerConfig::file_names`).
    pub names: &'static [(&'static str, &'static str)],
    /// The server serves its files only where one of these is in the file's directory or above
    /// it (see `ServerConfig::requires`). Empty: everywhere.
    pub requires: &'static [&'static str],
    /// Files that make a directory a project of its own. Empty: the server finds its projects.
    pub root_markers: &'static [&'static str],
    /// Analysis runs project code (build scripts, proc macros, MSBuild design-time builds,
    /// a configuration file written in JavaScript).
    pub executes_during_analysis: bool,
    pub install: Install,
    /// What to run to learn the server's version; `None` if it has no such flag.
    pub version_flag: Option<&'static str>,
    pub restore: Restore,
    pub compiler: Compiler,
    /// The server checks files against schemas, which it may load from the network.
    pub schemas: SchemaClient,
    /// The project's own toolchain may provide the server (`rustup which`), and wins over an install.
    pub project_toolchain: bool,
    /// The server loads nothing until told which solution or projects (`solution/open`, `project/open`).
    pub loads_on_request: bool,
    /// Opening a file starts loading the project it belongs to (tsserver), so
    /// answers across files wait for the server to go quiet after an open.
    pub loads_project_on_open: bool,
    pub user_config: UserConfig,
}

/// Build scripts and proc macros run during analysis.
pub const RUST: Language = Language {
    id: "rust",
    title: "Rust",
    display_name: "rust-analyzer",
    binary: "rust-analyzer",
    args: &[],
    files: &[("rs", "rust")],
    names: &[],
    requires: &[],
    root_markers: &["Cargo.toml"],
    executes_during_analysis: true,
    install: Install::No,
    version_flag: Some("--version"),
    restore: Restore::Cargo,
    compiler: Compiler::None,
    schemas: SchemaClient::None,
    project_toolchain: true,
    loads_on_request: false,
    loads_project_on_open: false,
    // Cargo's configuration chain: `.cargo/config.toml` in every directory above the
    // project and in `$CARGO_HOME` (source replacement, build flags, target); the
    // toolchain rustup picks (override files, the default in rustup's settings); and
    // rust-analyzer's own settings files.
    user_config: UserConfig {
        above_root: &[
            ".cargo/config.toml",
            ".cargo/config",
            "rust-toolchain.toml",
            "rust-toolchain",
            "rust-analyzer.toml",
            ".rust-analyzer.toml",
        ],
        user_files: &[
            UserFile::Under {
                var: "CARGO_HOME",
                fallback: ".cargo",
                file: "config.toml",
            },
            UserFile::Under {
                var: "CARGO_HOME",
                fallback: ".cargo",
                file: "config",
            },
            UserFile::Under {
                var: "RUSTUP_HOME",
                fallback: ".rustup",
                file: "settings.toml",
            },
            UserFile::Under {
                var: "XDG_CONFIG_HOME",
                fallback: ".config",
                file: "rust-analyzer/rust-analyzer.toml",
            },
            UserFile::Home("Library/Application Support/rust-analyzer/rust-analyzer.toml"),
            UserFile::Under {
                var: "APPDATA",
                fallback: "AppData/Roaming",
                file: "rust-analyzer/rust-analyzer.toml",
            },
        ],
        env: &[
            "RUSTUP_TOOLCHAIN",
            "RUSTUP_HOME",
            "CARGO_HOME",
            "RUSTC",
            "RUSTC_WORKSPACE_WRAPPER",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "CARGO_BUILD_RUSTFLAGS",
            "RUST_SRC_PATH",
        ],
        toolchain: &[("rustc", &["-vV"])],
    },
};

/// What Node.js servers read: the runtime's own variables, and the proxy and certificate
/// variables of the HTTP clients that load schemas.
const NODE_ENV: &[&str] = &["NODE_PATH", "NODE_OPTIONS"];
const NODE_HTTP_ENV: &[&str] = &[
    "NODE_PATH",
    "NODE_OPTIONS",
    "NODE_EXTRA_CA_CERTS",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

pub const TYPESCRIPT: Language = Language {
    id: "typescript",
    title: "TypeScript / JavaScript",
    display_name: "typescript-language-server",
    binary: "typescript-language-server",
    args: &["--stdio"],
    files: &[
        ("ts", "typescript"),
        ("tsx", "typescriptreact"),
        ("mts", "typescript"),
        ("cts", "typescript"),
        ("js", "javascript"),
        ("jsx", "javascriptreact"),
        ("mjs", "javascript"),
        ("cjs", "javascript"),
    ],
    names: &[],
    requires: &[],
    root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[
            ("typescript-language-server", TYPESCRIPT_LANGUAGE_SERVER_VERSION),
            ("typescript", FALLBACK_TYPESCRIPT_VERSION),
        ],
    },
    version_flag: Some("--version"),
    restore: Restore::Javascript,
    compiler: Compiler::TypeScript,
    schemas: SchemaClient::None,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: true,
    // tsserver takes the nearest `tsconfig.json` or `jsconfig.json` above a file, even
    // above the root. It reads no settings of the user's; Node reads the variables.
    user_config: UserConfig {
        above_root: &["tsconfig.json", "jsconfig.json"],
        user_files: &[],
        env: NODE_ENV,
        toolchain: &[("node", &["--version"])],
    },
};

pub const PYTHON: Language = Language {
    id: "python",
    title: "Python",
    display_name: "Pyright",
    binary: "pyright-langserver",
    args: &["--stdio"],
    files: &[("py", "python"), ("pyi", "python")],
    names: &[],
    requires: &[],
    root_markers: &["pyproject.toml", "pyrightconfig.json", "setup.py", "setup.cfg"],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[("pyright", PYRIGHT_VERSION)],
    },
    version_flag: None,
    restore: Restore::Python,
    compiler: Compiler::None,
    schemas: SchemaClient::None,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: false,
    // Pyright reads the configuration in the project root and none of the user's; the
    // interpreter it asks for the search paths comes from the environment (a restore's
    // virtual environment arrives with the restored tree, not here).
    user_config: UserConfig {
        above_root: &[],
        user_files: &[],
        env: &["VIRTUAL_ENV", "CONDA_PREFIX", "PYTHONPATH", "PYTHONHOME"],
        toolchain: &[("python3", &["--version"]), ("python", &["--version"])],
    },
};

/// MSBuild design-time builds run project code. The solution or projects to load
/// are found by [`crate::lsp::dotnet`], so there are no root markers.
pub const CSHARP: Language = Language {
    id: "csharp",
    title: "C#",
    display_name: "roslyn-language-server",
    binary: "roslyn-language-server",
    args: &["--stdio", "--logLevel", "Warning"],
    files: &[("cs", "csharp")],
    names: &[],
    requires: &[],
    root_markers: &[],
    executes_during_analysis: true,
    install: Install::DotnetTool {
        package: "roslyn-language-server",
        version: ROSLYN_LANGUAGE_SERVER_VERSION,
    },
    version_flag: None,
    restore: Restore::Dotnet,
    compiler: Compiler::None,
    schemas: SchemaClient::None,
    project_toolchain: false,
    loads_on_request: true,
    loads_project_on_open: false,
    // The SDK `global.json` picks, MSBuild's inherited props and analyzer settings in the
    // directories above the project, and NuGet's configuration chain (above the project
    // and the user-level file) that decides where packages come from.
    user_config: UserConfig {
        above_root: &[
            "global.json",
            "NuGet.config",
            "NuGet.Config",
            "nuget.config",
            "Directory.Build.props",
            "Directory.Build.targets",
            "Directory.Packages.props",
            ".editorconfig",
        ],
        user_files: &[
            UserFile::Home(".nuget/NuGet/NuGet.Config"),
            UserFile::Home(".nuget/NuGet/nuget.config"),
            UserFile::Under {
                var: "APPDATA",
                fallback: "AppData/Roaming",
                file: "NuGet/NuGet.Config",
            },
        ],
        env: &[
            "DOTNET_ROOT",
            "DOTNET_ROOT_X64",
            "DOTNET_ROOT_ARM64",
            "DOTNET_CLI_HOME",
            "DOTNET_ROLL_FORWARD",
            "DOTNET_MSBUILD_SDK_RESOLVER_SDKS_DIR",
            "MSBuildSDKsPath",
            "MSBUILD_EXE_PATH",
            "NUGET_PACKAGES",
        ],
        toolchain: &[("dotnet", &["--version"])],
    },
};

/// `svelte.config.js` is a program: the server runs it (through `@sveltejs/load-config`) to learn
/// the preprocessors, so analysis runs project code. It finds `typescript` and `svelte` in the
/// project first, and in its own install after.
pub const SVELTE: Language = Language {
    id: "svelte",
    title: "Svelte",
    display_name: "svelte-language-server",
    binary: "svelteserver",
    args: &["--stdio"],
    files: &[("svelte", "svelte")],
    names: &[],
    requires: &[],
    root_markers: &[
        "svelte.config.js",
        "svelte.config.cjs",
        "svelte.config.mjs",
        "svelte.config.ts",
        "tsconfig.json",
        "jsconfig.json",
        "package.json",
    ],
    executes_during_analysis: true,
    install: Install::Npm {
        packages: &[
            ("svelte-language-server", SVELTE_LANGUAGE_SERVER_VERSION),
            ("typescript", FALLBACK_TYPESCRIPT_VERSION),
        ],
    },
    version_flag: None,
    restore: Restore::Javascript,
    compiler: Compiler::None,
    schemas: SchemaClient::None,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: true,
    // The nearest `tsconfig.json` or `jsconfig.json` above a file, as for tsserver, and the
    // `svelte.config.js` of the project (in the snapshot). It reads no settings of the user's.
    user_config: UserConfig {
        above_root: &["tsconfig.json", "jsconfig.json"],
        user_files: &[],
        env: NODE_ENV,
        toolchain: &[("node", &["--version"])],
    },
};

/// Serves `.html`: the templates of Angular components. TypeScript files stay with the TypeScript
/// server, so templates written inline in a component are not analysed. The server loads
/// `typescript` and `@angular/language-service` from the project's packages, then from its own
/// install, and runs no file of the project itself: what it loads is what the restore installed,
/// as for the TypeScript server and the plugins a `tsconfig.json` names.
pub const ANGULAR: Language = Language {
    id: "angular",
    title: "Angular templates",
    display_name: "Angular Language Server",
    binary: "ngserver",
    args: &["--stdio"],
    files: &[("html", "html")],
    names: &[],
    requires: &["angular.json"],
    root_markers: &["angular.json", "tsconfig.json", "package.json"],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[
            ("@angular/language-server", ANGULAR_LANGUAGE_SERVER_VERSION),
            ("@angular/language-service", ANGULAR_LANGUAGE_SERVICE_VERSION),
            ("typescript", FALLBACK_TYPESCRIPT_VERSION),
        ],
    },
    version_flag: None,
    restore: Restore::Javascript,
    compiler: Compiler::TypeScript,
    schemas: SchemaClient::None,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: true,
    user_config: UserConfig {
        above_root: &["tsconfig.json"],
        user_files: &[],
        env: NODE_ENV,
        toolchain: &[("node", &["--version"])],
    },
};

/// Checks files against schemas: a fixed set is bundled with Vet, and the server may load
/// others from the network only where the analysis policy allows it (see [`crate::schemas`]).
pub const YAML: Language = Language {
    id: "yaml",
    title: "YAML",
    display_name: "yaml-language-server",
    binary: "yaml-language-server",
    args: &["--stdio"],
    files: &[("yml", "yaml"), ("yaml", "yaml")],
    names: &[],
    requires: &[],
    root_markers: &[],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[("yaml-language-server", YAML_LANGUAGE_SERVER_VERSION)],
    },
    version_flag: Some("--version"),
    restore: Restore::None,
    compiler: Compiler::None,
    schemas: SchemaClient::Yaml,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: false,
    // It reads the settings Vet gives it and no file of the user's.
    user_config: UserConfig {
        above_root: &[],
        user_files: &[],
        env: NODE_HTTP_ENV,
        toolchain: &[("node", &["--version"])],
    },
};

/// The JSON server of `vscode-langservers-extracted`; schemas as for [`YAML`].
pub const JSON: Language = Language {
    id: "json",
    title: "JSON",
    display_name: "vscode-json-language-server",
    binary: "vscode-json-language-server",
    args: &["--stdio"],
    files: &[("json", "json"), ("jsonc", "jsonc")],
    names: &[],
    requires: &[],
    root_markers: &[],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[("vscode-langservers-extracted", VSCODE_LANGSERVERS_VERSION)],
    },
    version_flag: None,
    restore: Restore::None,
    compiler: Compiler::None,
    schemas: SchemaClient::Json,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: false,
    user_config: UserConfig {
        above_root: &[],
        user_files: &[],
        env: NODE_HTTP_ENV,
        toolchain: &[("node", &["--version"])],
    },
};

/// Runs `shellcheck` from `PATH` if there is one, on the scripts it is asked about; that program
/// reads the script and the `.shellcheckrc` files above it and runs nothing of the project's.
pub const BASH: Language = Language {
    id: "bash",
    title: "Shell",
    display_name: "bash-language-server",
    binary: "bash-language-server",
    args: &["start"],
    files: &[("sh", "shellscript"), ("bash", "shellscript")],
    names: &[],
    requires: &[],
    root_markers: &[],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[("bash-language-server", BASH_LANGUAGE_SERVER_VERSION)],
    },
    version_flag: Some("--version"),
    restore: Restore::None,
    compiler: Compiler::None,
    schemas: SchemaClient::None,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: false,
    // ShellCheck's configuration chain and the variables the server and ShellCheck read. Which
    // ShellCheck runs, and whether one does, changes the diagnostics, so its version is observed.
    user_config: UserConfig {
        above_root: &[".shellcheckrc", "shellcheckrc"],
        user_files: &[
            UserFile::Home(".shellcheckrc"),
            UserFile::Under {
                var: "XDG_CONFIG_HOME",
                fallback: ".config",
                file: "shellcheckrc",
            },
        ],
        env: &[
            "NODE_PATH",
            "NODE_OPTIONS",
            "SHELLCHECK_PATH",
            "SHELLCHECK_OPTS",
            "SHELLCHECK_ARGUMENTS",
            "SHFMT_PATH",
            "EXPLAINSHELL_ENDPOINT",
            "GLOB_PATTERN",
            "BACKGROUND_ANALYSIS_MAX_FILES",
            "INCLUDE_ALL_WORKSPACE_SYMBOLS",
            "HIGHLIGHT_PARSING_ERRORS",
            "ENABLE_SOURCE_ERROR_DIAGNOSTICS",
        ],
        toolchain: &[("node", &["--version"]), ("shellcheck", &["--version"])],
    },
};

/// rcjsuen's Dockerfile language server: pure Node.js, offline. Docker's own
/// `docker-language-server` is the more active project but needs Buildx and asks Docker Scout
/// over the network.
pub const DOCKERFILE: Language = Language {
    id: "dockerfile",
    title: "Dockerfile",
    display_name: "dockerfile-language-server",
    binary: "docker-langserver",
    args: &["--stdio"],
    files: &[("dockerfile", "dockerfile")],
    names: &[("Dockerfile", "dockerfile"), ("Containerfile", "dockerfile")],
    requires: &[],
    root_markers: &[],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[("dockerfile-language-server-nodejs", DOCKERFILE_LANGUAGE_SERVER_VERSION)],
    },
    version_flag: None,
    restore: Restore::None,
    compiler: Compiler::None,
    schemas: SchemaClient::None,
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: false,
    // It reads nothing outside the repository.
    user_config: UserConfig {
        above_root: &[],
        user_files: &[],
        env: NODE_ENV,
        toolchain: &[("node", &["--version"])],
    },
};

pub const ALL: [&Language; 10] = [
    &RUST,
    &TYPESCRIPT,
    &PYTHON,
    &CSHARP,
    &SVELTE,
    &ANGULAR,
    &YAML,
    &JSON,
    &BASH,
    &DOCKERFILE,
];

pub fn by_id(id: &str) -> Option<&'static Language> {
    ALL.into_iter().find(|l| l.id == id)
}

/// The LSP `languageId` for a file: by its extension, else by its name.
pub fn language_id(file: &RepoPath) -> &'static str {
    let name = String::from_utf8_lossy(file.0.rsplit(|&b| b == b'/').next().unwrap_or_default()).into_owned();
    let extension = file.extension().map(|e| String::from_utf8_lossy(e).into_owned());
    ALL.into_iter()
        .find_map(|l| {
            let by_extension = l.files.iter().find(|(ext, _)| Some(*ext) == extension.as_deref());
            let by_name = l
                .names
                .iter()
                .find(|(n, _)| name == *n || (name.starts_with(n) && name.as_bytes().get(n.len()) == Some(&b'.')));
            by_extension.or(by_name).map(|(_, id)| *id)
        })
        .unwrap_or("plaintext")
}

impl Language {
    /// The semantic layer's view of the language.
    pub fn backend(&self) -> Backend {
        let strings = |xs: &[&str]| xs.iter().map(|s| (*s).to_owned()).collect();
        let config = ServerConfig {
            backend: BackendId(self.id.into()),
            binary: self.binary.into(),
            args: strings(self.args),
            filetypes: self.files.iter().map(|(ext, _)| (*ext).to_owned()).collect(),
            file_names: self.names.iter().map(|(name, _)| (*name).to_owned()).collect(),
            requires: strings(self.requires),
            root_markers: strings(self.root_markers),
            executes_project_code_during_analysis: self.executes_during_analysis,
            loads_project_on_open: self.loads_project_on_open,
        };
        if self.loads_on_request {
            Backend::Roslyn(config)
        } else {
            Backend::GenericLsp(config)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn ids_extensions_and_names_are_unique() {
        let ids: HashSet<_> = ALL.iter().map(|l| l.id).collect();
        assert_eq!(ids.len(), ALL.len());
        let extensions: Vec<_> = ALL.iter().flat_map(|l| l.files).map(|(ext, _)| ext).collect();
        assert_eq!(extensions.len(), extensions.iter().collect::<HashSet<_>>().len());
        let names: Vec<_> = ALL.iter().flat_map(|l| l.names).map(|(name, _)| name).collect();
        assert_eq!(names.len(), names.iter().collect::<HashSet<_>>().len());
    }

    #[test]
    fn the_language_id_follows_the_extension_or_the_name() {
        let id = |p: &str| language_id(&RepoPath::from(p));
        assert_eq!(id("src/app.tsx"), "typescriptreact");
        assert_eq!(id("ci/build.yml"), "yaml");
        assert_eq!(id("Dockerfile"), "dockerfile");
        assert_eq!(id("svc/Dockerfile.prod"), "dockerfile");
        assert_eq!(id("svc/api.dockerfile"), "dockerfile");
        assert_eq!(id("run.sh"), "shellscript");
        assert_eq!(id("README"), "plaintext");
        assert_eq!(id("notes.txt"), "plaintext");
    }
}
