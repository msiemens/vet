use crate::Language;

/// What language a piece of code is in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Syntax {
    /// That of the file it is in.
    File,
    /// The info string of the Markdown fence it came from; empty for a fence without one.
    Fence(String),
    /// Prose, not code.
    Plain,
}

impl Syntax {
    /// The grammar for code in the file at `path`; `None` for prose and for a language without one.
    pub fn language(&self, path: &str) -> Option<Language> {
        match self {
            Self::File => Language::of_path(path),
            Self::Fence(info) => Language::of_fence_or(info, Language::of_path(path)),
            Self::Plain => None,
        }
    }
}
