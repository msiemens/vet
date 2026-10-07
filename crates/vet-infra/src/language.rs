//! The languages Vet analyses, one entry each (§7.1): how the server runs, which
//! files it serves, how it is found and installed, and how a tree's dependencies
//! are restored for it. Adding a language is an entry here, a restore plan in
//! [`crate::restore`], and a one-time manual check that its server supports the
//! required operations.

use vet_semantic::ports::{Backend, BackendId, ServerConfig};

/// Pinned versions of the servers Vet installs. Bump with a check that the
/// server still has every required capability (§7.1).
pub const TYPESCRIPT_LANGUAGE_SERVER_VERSION: &str = "6.0.1";
pub const PYRIGHT_VERSION: &str = "1.1.414";
pub const ROSLYN_LANGUAGE_SERVER_VERSION: &str = "5.12.0-1.26475.2";
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
    Cargo,
    Javascript,
    Python,
    Dotnet,
}

/// A compiler the language server does not bring and must be told about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compiler {
    None,
    /// The project's `typescript`, or Vet's fallback beside the server.
    TypeScript,
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
    /// Files that make a directory a project of its own. Empty: the server finds its projects.
    pub root_markers: &'static [&'static str],
    /// Analysis runs project code (build scripts, proc macros, MSBuild design-time builds).
    pub executes_during_analysis: bool,
    pub install: Install,
    /// What to run to learn the server's version; `None` if it has no such flag.
    pub version_flag: Option<&'static str>,
    pub restore: Restore,
    pub compiler: Compiler,
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
    root_markers: &["Cargo.toml"],
    executes_during_analysis: true,
    install: Install::No,
    version_flag: Some("--version"),
    restore: Restore::Cargo,
    compiler: Compiler::None,
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
    project_toolchain: false,
    loads_on_request: false,
    loads_project_on_open: true,
    // tsserver takes the nearest `tsconfig.json` or `jsconfig.json` above a file, even
    // above the root. It reads no settings of the user's; Node reads the variables.
    user_config: UserConfig {
        above_root: &["tsconfig.json", "jsconfig.json"],
        user_files: &[],
        env: &["NODE_PATH", "NODE_OPTIONS"],
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
    root_markers: &["pyproject.toml", "pyrightconfig.json", "setup.py", "setup.cfg"],
    executes_during_analysis: false,
    install: Install::Npm {
        packages: &[("pyright", PYRIGHT_VERSION)],
    },
    version_flag: None,
    restore: Restore::Python,
    compiler: Compiler::None,
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
    root_markers: &[],
    executes_during_analysis: true,
    install: Install::DotnetTool {
        package: "roslyn-language-server",
        version: ROSLYN_LANGUAGE_SERVER_VERSION,
    },
    version_flag: None,
    restore: Restore::Dotnet,
    compiler: Compiler::None,
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

/// The POC languages.
pub const ALL: [&Language; 4] = [&RUST, &TYPESCRIPT, &PYTHON, &CSHARP];

pub fn by_id(id: &str) -> Option<&'static Language> {
    ALL.into_iter().find(|l| l.id == id)
}

/// The LSP `languageId` for a file extension.
pub fn language_id(extension: &str) -> &'static str {
    ALL.into_iter()
        .flat_map(|l| l.files)
        .find(|(ext, _)| *ext == extension)
        .map_or("plaintext", |(_, id)| id)
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
    fn ids_and_extensions_are_unique() {
        let ids: HashSet<_> = ALL.iter().map(|l| l.id).collect();
        assert_eq!(ids.len(), ALL.len());
        let extensions: Vec<_> = ALL.iter().flat_map(|l| l.files).map(|(ext, _)| ext).collect();
        assert_eq!(extensions.len(), extensions.iter().collect::<HashSet<_>>().len());
    }
}
