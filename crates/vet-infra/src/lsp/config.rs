//! The languages' server configurations as the semantic layer sees them (§7.1).

use vet_semantic::ports::Backend;

use crate::language::{self, Language};

pub fn rust_analyzer() -> Backend {
    language::RUST.backend()
}

pub fn typescript() -> Backend {
    language::TYPESCRIPT.backend()
}

pub fn pyright() -> Backend {
    language::PYTHON.backend()
}

pub fn roslyn() -> Backend {
    language::CSHARP.backend()
}

/// The POC languages.
pub fn defaults() -> Vec<Backend> {
    language::ALL.map(Language::backend).into()
}
