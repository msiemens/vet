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
