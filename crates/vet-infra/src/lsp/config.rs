//! Server config entries (§7.1). Adding a language is a config entry plus a
//! one-time manual check that its server supports the required operations.

use vet_semantic::ports::{Backend, BackendId, ServerConfig};

fn entry(name: &str, binary: &str, args: &[&str], filetypes: &[&str], markers: &[&str], executes: bool) -> Backend {
    let strings = |xs: &[&str]| xs.iter().map(|s| (*s).to_owned()).collect();
    Backend::GenericLsp(ServerConfig {
        backend: BackendId(name.into()),
        binary: binary.into(),
        args: strings(args),
        filetypes: strings(filetypes),
        root_markers: strings(markers),
        executes_project_code_during_analysis: executes,
    })
}

/// Build scripts and proc macros run during analysis.
pub fn rust_analyzer() -> Backend {
    entry("rust", "rust-analyzer", &[], &["rs"], &["Cargo.toml"], true)
}

pub fn typescript() -> Backend {
    entry(
        "typescript",
        "typescript-language-server",
        &["--stdio"],
        &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
        &["tsconfig.json", "jsconfig.json", "package.json"],
        false,
    )
}

pub fn pyright() -> Backend {
    entry(
        "python",
        "pyright-langserver",
        &["--stdio"],
        &["py", "pyi"],
        &["pyproject.toml", "pyrightconfig.json", "setup.py", "setup.cfg"],
        false,
    )
}

/// MSBuild design-time builds run project code. The solution or projects to load
/// are found by [`super::dotnet`], so there are no root markers.
pub fn roslyn() -> Backend {
    let Backend::GenericLsp(config) = entry(
        "csharp",
        "roslyn-language-server",
        &["--stdio", "--logLevel", "Warning"],
        &["cs"],
        &[],
        true,
    ) else {
        unreachable!("entry builds a generic backend")
    };
    Backend::Roslyn(config)
}

/// The POC languages.
pub fn defaults() -> Vec<Backend> {
    vec![rust_analyzer(), typescript(), pyright(), roslyn()]
}

/// The LSP `languageId` for a file extension.
pub(super) fn language_id(extension: &str) -> &'static str {
    match extension {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "py" | "pyi" => "python",
        "cs" => "csharp",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::RepoPath;
    use vet_semantic::ports::backend_for;

    use super::*;

    #[test]
    fn every_poc_language_serves_its_files() {
        let backends = defaults();
        let language = |file: &str| {
            backend_for(&backends, &RepoPath(file.as_bytes().to_vec())).map(|b| b.config().backend.0.clone())
        };
        assert_eq!(language("src/lib.rs").as_deref(), Some("rust"));
        assert_eq!(language("ui/a.tsx").as_deref(), Some("typescript"));
        assert_eq!(language("tools/a.py").as_deref(), Some("python"));
        assert_eq!(language("src/Program.cs").as_deref(), Some("csharp"));
        assert!(matches!(
            backend_for(&backends, &RepoPath(b"a.cs".to_vec())),
            Some(Backend::Roslyn(_))
        ));
        assert_eq!(language("README.md"), None);
    }
}
