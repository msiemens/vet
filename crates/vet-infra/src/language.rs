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

pub struct Language {
    pub id: &'static str,
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
}

/// Build scripts and proc macros run during analysis.
pub const RUST: Language = Language {
    id: "rust",
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
};

pub const TYPESCRIPT: Language = Language {
    id: "typescript",
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
};

pub const PYTHON: Language = Language {
    id: "python",
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
};

/// MSBuild design-time builds run project code. The solution or projects to load
/// are found by [`crate::lsp::dotnet`], so there are no root markers.
pub const CSHARP: Language = Language {
    id: "csharp",
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

    #[test]
    fn an_extension_takes_its_own_language_id_and_an_unknown_one_is_plain_text() {
        assert_eq!(language_id("tsx"), "typescriptreact");
        assert_eq!(language_id("md"), "plaintext");
    }
}
