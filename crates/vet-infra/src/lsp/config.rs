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

pub fn svelte() -> Backend {
    language::SVELTE.backend()
}

pub fn angular() -> Backend {
    language::ANGULAR.backend()
}

pub fn yaml() -> Backend {
    language::YAML.backend()
}

pub fn json() -> Backend {
    language::JSON.backend()
}

pub fn bash() -> Backend {
    language::BASH.backend()
}

pub fn dockerfile() -> Backend {
    language::DOCKERFILE.backend()
}

/// Every language.
pub fn defaults() -> Vec<Backend> {
    language::ALL.map(Language::backend).into()
}
