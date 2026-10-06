//! What `roslyn-language-server` is told to load. It loads nothing by itself
//! unless sent `solution/open` or `project/open` after `initialized`.

use std::path::{Path, PathBuf};

/// How many directory levels below the root are searched for projects.
const SEARCH_DEPTH: usize = 3;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Load {
    /// The only solution in the root.
    Solution(PathBuf),
    /// Every project found, when there is no single solution to load.
    Projects(Vec<PathBuf>),
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The solution in `root` if there is exactly one (`.sln` or `.slnx`), else the
/// `.csproj` files below it. `None` if there is nothing to load.
pub fn find(root: &Path) -> Option<Load> {
    let mut solutions: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && has_extension(p, &["sln", "slnx"]))
        .collect();
    if solutions.len() == 1 {
        return solutions.pop().map(Load::Solution);
    }

    let mut projects = Vec::new();
    let mut level = vec![root.to_owned()];
    for _ in 0..=SEARCH_DEPTH {
        let mut next = Vec::new();
        for dir in level {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if !name.starts_with('.') && !matches!(name.as_ref(), "node_modules" | "bin" | "obj" | "target") {
                        next.push(path);
                    }
                } else if has_extension(&path, &["csproj"]) {
                    projects.push(path);
                }
            }
        }
        level = next;
    }
    projects.sort();
    (!projects.is_empty()).then_some(Load::Projects(projects))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, name: &str) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn a_single_solution_is_loaded_whole() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "App.sln");
        touch(dir.path(), "src/App.csproj");
        assert_eq!(find(dir.path()), Some(Load::Solution(dir.path().join("App.sln"))));
    }

    #[test]
    fn without_a_single_solution_the_projects_are_loaded() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "A.sln");
        touch(dir.path(), "B.slnx");
        touch(dir.path(), "src/App/App.csproj");
        touch(dir.path(), "tests/App.Tests/App.Tests.csproj");
        touch(dir.path(), "src/App/obj/Generated.csproj");
        touch(dir.path(), "node_modules/x/X.csproj");
        assert_eq!(
            find(dir.path()),
            Some(Load::Projects(vec![
                dir.path().join("src/App/App.csproj"),
                dir.path().join("tests/App.Tests/App.Tests.csproj"),
            ]))
        );
    }

    #[test]
    fn nothing_to_load_is_none() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "readme.md");
        assert_eq!(find(dir.path()), None);
    }
}
