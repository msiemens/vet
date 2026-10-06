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

/// The POC languages.
pub fn defaults() -> Vec<Backend> {
    vec![rust_analyzer(), typescript(), pyright()]
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
        _ => "plaintext",
    }
}
