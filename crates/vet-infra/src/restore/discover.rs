//! Which directories of a tree are projects to restore, and which of those a
//! workspace root already covers. Directories are relative to the tree's root;
//! the root itself is the empty path.

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde_json::Value;

/// How deep below the root a project may be. Four levels reach the usual
/// monorepo layouts (`services/api/clients/rust`); a manifest deeper than that
/// is mostly a fixture or an example, and every project found costs a restore
/// and a workspace folder of the language server. Restoring and the server's
/// workspace folders share it, so neither covers a project the other misses.
const MAX_DEPTH: usize = 4;

/// Folders no manifest inside is a project of the repository: dependencies and build output.
const SKIPPED: &[&str] = &["node_modules", "target", "vendor"];

/// The directories that hold a file called one of `names`, shallowest first.
/// Hidden folders and tool or build folders are not searched.
pub(crate) fn candidates(files: &[PathBuf], names: &[&str]) -> Vec<PathBuf> {
    let usable = |dir: &Path| {
        dir.components().count() <= MAX_DEPTH
            && dir.components().all(|c| {
                let name = c.as_os_str().to_string_lossy();
                !name.starts_with('.') && !SKIPPED.contains(&name.as_ref())
            })
    };
    let named = |file: &&PathBuf| {
        file.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| names.contains(&n))
    };
    let mut dirs: Vec<PathBuf> = (files.iter().filter(named))
        .filter_map(|f| f.parent())
        .filter(|d| usable(d))
        .map(Path::to_owned)
        .collect();
    dirs.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });
    dirs.dedup();
    dirs
}

/// `dirs`, shallowest first, without those below another of them: a project
/// holds the directories under it.
pub(crate) fn outermost(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if !kept.iter().any(|outer| dir.starts_with(outer)) {
            kept.push(dir);
        }
    }
    kept
}

/// A directory whose tool installs the packages of the directories below it in one go.
pub(super) struct Workspace {
    dir: PathBuf,
    members: GlobSet,
    excluded: GlobSet,
}

fn set(patterns: impl IntoIterator<Item = String>) -> GlobSet {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
        // `*` stays within a folder name, as in the tools' own workspace globs.
        if let Ok(glob) = GlobBuilder::new(pattern).literal_separator(true).build() {
            set.add(glob);
        }
    }
    set.build().unwrap_or_default()
}

impl Workspace {
    /// Patterns starting with `!` exclude, as in pnpm's `pnpm-workspace.yaml`.
    fn new(dir: &Path, patterns: Vec<String>, excluded: Vec<String>) -> Self {
        let (negated, members): (Vec<_>, Vec<_>) = patterns.into_iter().partition(|p| p.starts_with('!'));
        let negated = negated.into_iter().map(|p| p[1..].to_owned());
        Self {
            dir: dir.to_owned(),
            members: set(members),
            excluded: set(negated.chain(excluded)),
        }
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether `dir` is below the workspace and one of its members.
    fn holds(&self, dir: &Path) -> bool {
        let Ok(relative) = dir.strip_prefix(&self.dir) else {
            return false;
        };
        !relative.as_os_str().is_empty() && self.members.is_match(relative) && !self.excluded.is_match(relative)
    }
}

/// `dirs` minus the members of a workspace: the root's install covers them.
pub(super) fn without_members(dirs: Vec<PathBuf>, workspaces: &[Workspace]) -> Vec<PathBuf> {
    dirs.into_iter()
        .filter(|dir| !workspaces.iter().any(|w| w.holds(dir)))
        .collect()
}

fn read(root: &Path, dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(root.join(dir).join(name)).ok()
}

fn strings(value: Option<&toml::Value>) -> Vec<String> {
    let list = value.and_then(toml::Value::as_array).into_iter().flatten();
    list.filter_map(|v| v.as_str().map(str::to_owned)).collect()
}

/// A Cargo workspace owns every package below it that it does not exclude: path
/// dependencies are members without being listed.
pub(super) fn cargo_workspaces(root: &Path, dirs: &[PathBuf]) -> Vec<Workspace> {
    dirs.iter()
        .filter_map(|dir| {
            let manifest = read(root, dir, "Cargo.toml")?.parse::<toml::Table>().ok()?;
            let workspace = manifest.get("workspace")?.as_table()?;
            let excluded = strings(workspace.get("exclude"))
                .into_iter()
                .flat_map(|e| [format!("{e}/**"), e])
                .collect();
            Some(Workspace::new(dir, vec!["**".into()], excluded))
        })
        .collect()
}

/// The `packages:` of a `pnpm-workspace.yaml`: the one list of plain strings
/// the file is used for, so no YAML parser.
fn pnpm_packages(text: &str) -> Vec<String> {
    let mut lines = text.lines().skip_while(|l| l.trim_end() != "packages:");
    lines.next();
    let mut packages = Vec::new();
    for line in lines {
        let Some(item) = line.trim().strip_prefix('-') else {
            if line.trim().is_empty() || line.trim().starts_with('#') {
                continue;
            }
            break;
        };
        let item = item.split(" #").next().unwrap_or("").trim();
        packages.push(item.trim_matches(['"', '\'']).to_owned());
    }
    packages
}

/// pnpm (`pnpm-workspace.yaml`), npm and Yarn (`workspaces` in `package.json`).
pub(super) fn js_workspaces(root: &Path, dirs: &[PathBuf]) -> Vec<Workspace> {
    dirs.iter()
        .filter_map(|dir| {
            let mut patterns = read(root, dir, "pnpm-workspace.yaml")
                .map(|t| pnpm_packages(&t))
                .unwrap_or_default();
            let package: Value = serde_json::from_str(&read(root, dir, "package.json")?).unwrap_or_default();
            let workspaces = package.get("workspaces");
            let list = workspaces.and_then(|w| w.get("packages").or(Some(w)));
            let listed = list.and_then(Value::as_array).into_iter().flatten();
            patterns.extend(listed.filter_map(|v| v.as_str().map(str::to_owned)));
            (!patterns.is_empty()).then(|| Workspace::new(dir, patterns, Vec::new()))
        })
        .collect()
}

/// uv's `[tool.uv.workspace]`.
pub(super) fn uv_workspaces(root: &Path, dirs: &[PathBuf]) -> Vec<Workspace> {
    dirs.iter()
        .filter_map(|dir| {
            let manifest = read(root, dir, "pyproject.toml")?.parse::<toml::Table>().ok()?;
            let workspace = manifest.get("tool")?.get("uv")?.get("workspace")?;
            let members = strings(workspace.get("members"));
            Some(Workspace::new(dir, members, strings(workspace.get("exclude"))))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(files: &[&str]) -> Vec<PathBuf> {
        files.iter().map(PathBuf::from).collect()
    }

    fn tree(files: &[(&str, &str)]) -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let names = files.iter().map(|(n, _)| PathBuf::from(n)).collect();
        (dir, names)
    }

    #[test]
    fn candidates_are_shallowest_first_and_bounded() {
        let found = candidates(
            &paths(&[
                "b/package.json",
                "package.json",
                "a/x/package.json",
                "a/package.json",
                "a/b/c/d/e/package.json",
                "a/b/c/d/package.json",
                "README.md",
            ]),
            &["package.json"],
        );
        assert_eq!(found, paths(&["", "a", "b", "a/x", "a/b/c/d"]));
    }

    #[test]
    fn a_project_holds_the_projects_below_it() {
        let dirs = paths(&["a", "b", "a/x", "b/y/z", "ab"]);
        assert_eq!(outermost(dirs), paths(&["a", "b", "ab"]));
        assert_eq!(outermost(paths(&["", "a"])), paths(&[""]));
    }

    #[test]
    fn tool_stores_hidden_and_build_folders_are_not_searched() {
        let found = candidates(
            &paths(&[
                "node_modules/dep/package.json",
                "web/node_modules/dep/package.json",
                ".claude/worktrees/w/package.json",
                "target/debug/Cargo.toml",
                "vendor/lib/Cargo.toml",
                "tools/package.json",
            ]),
            &["package.json", "Cargo.toml"],
        );
        assert_eq!(found, paths(&["tools"]));
    }

    #[test]
    fn cargo_members_are_covered_by_the_workspace_root_unless_excluded() {
        let (dir, files) = tree(&[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"tools/standalone\"]\n",
            ),
            ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
            ("crates/b/Cargo.toml", "[package]\nname = \"b\"\n"),
            ("helper/Cargo.toml", "[package]\nname = \"h\"\n"),
            ("tools/standalone/Cargo.toml", "[package]\nname = \"s\"\n"),
            ("other/Cargo.toml", "[package]\nname = \"o\"\n"),
        ]);
        let dirs = candidates(&files, &["Cargo.toml"]);
        let roots = without_members(dirs.clone(), &cargo_workspaces(dir.path(), &dirs));
        assert_eq!(roots, paths(&["", "tools/standalone"]));
    }

    #[test]
    fn without_a_workspace_every_manifest_is_a_project() {
        let (dir, files) = tree(&[
            ("a/Cargo.toml", "[package]\nname = \"a\"\n"),
            ("b/Cargo.toml", "[package]\nname = \"b\"\n"),
        ]);
        let dirs = candidates(&files, &["Cargo.toml"]);
        assert_eq!(
            without_members(dirs.clone(), &cargo_workspaces(dir.path(), &dirs)),
            dirs
        );
    }

    #[test]
    fn pnpm_packages_are_globs_with_exclusions() {
        let yaml =
            "# comment\npackages:\n  - 'packages/*'\n  - \"apps/**\" # all\n  - '!apps/legacy'\n\ncatalog:\n  - x\n";
        assert_eq!(pnpm_packages(yaml), ["packages/*", "apps/**", "!apps/legacy"]);

        let (dir, files) = tree(&[
            ("package.json", "{}"),
            ("pnpm-workspace.yaml", yaml),
            ("packages/a/package.json", "{}"),
            ("packages/a/nested/package.json", "{}"),
            ("apps/web/package.json", "{}"),
            ("apps/legacy/package.json", "{}"),
            ("docs/package.json", "{}"),
        ]);
        let dirs = candidates(&files, &["package.json"]);
        let roots = without_members(dirs.clone(), &js_workspaces(dir.path(), &dirs));
        // `*` does not cross folders: `packages/a/nested` is not a member.
        assert_eq!(roots, paths(&["", "docs", "apps/legacy", "packages/a/nested"]));
    }

    #[test]
    fn npm_and_yarn_workspaces_come_from_package_json() {
        let (dir, files) = tree(&[
            ("package.json", "{\"workspaces\": [\"./libs/*\"]}"),
            ("libs/a/package.json", "{}"),
            ("sub/package.json", "{\"workspaces\": {\"packages\": [\"p/*\"]}}"),
            ("sub/p/x/package.json", "{}"),
            ("sub/q/package.json", "{}"),
        ]);
        let dirs = candidates(&files, &["package.json"]);
        let roots = without_members(dirs.clone(), &js_workspaces(dir.path(), &dirs));
        assert_eq!(roots, paths(&["", "sub", "sub/q"]));
    }

    #[test]
    fn uv_workspace_members_are_covered_by_the_root() {
        let (dir, files) = tree(&[
            (
                "pyproject.toml",
                "[project]\nname = \"r\"\n[tool.uv.workspace]\nmembers = [\"pkgs/*\"]\nexclude = [\"pkgs/old\"]\n",
            ),
            ("pkgs/a/pyproject.toml", "[project]\nname = \"a\"\n"),
            ("pkgs/old/pyproject.toml", "[project]\nname = \"o\"\n"),
        ]);
        let dirs = candidates(&files, &["pyproject.toml"]);
        let workspaces = uv_workspaces(dir.path(), &dirs);
        assert_eq!(workspaces.iter().map(|w| w.dir()).collect::<Vec<_>>(), [Path::new("")]);
        assert_eq!(without_members(dirs, &workspaces), paths(&["", "pkgs/old"]));
    }
}
