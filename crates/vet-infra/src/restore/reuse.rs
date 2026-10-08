//! Restored dependencies of another tree (the working tree), taken over by the
//! copy of a snapshot when everything a restore reads is the same on both
//! sides. Nothing here runs a tool or any project code: a directory is linked
//! or a few generated files are copied.
//!
//! Cargo needs nothing: `cargo fetch` fills Cargo home, which every tree shares.
//! Python is restored as before.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::Step;
use super::record::Digest;
use crate::budget::Budget;
use crate::git::list_files;
use vet_core::content::Hash;

/// How a step was done by the donor's restore.
pub(super) enum Reused {
    /// The donor's `node_modules` are linked in and follow the donor; `inputs`
    /// is what of the donor they depend on (`linked_inputs`).
    Linked(Hash),
    /// The restore output was copied: it is the copy's own from then on.
    Copied,
}

/// Whether `step`, a restore of the copy at `root`, is done by what `donor`
/// (the working tree) has restored. If so, the copy has the result. The files are
/// read and linked on `io`.
pub(super) async fn reuse(io: &Budget, donor: &Path, root: &Path, step: &Step) -> Option<Reused> {
    match step.program {
        "npm" | "pnpm" | "yarn" => {
            let (donor, root, step) = (donor.to_owned(), root.to_owned(), step.clone());
            io.blocking(move || node_modules(&donor, &root, &step))
                .await
                .ok()?
                .map(Reused::Linked)
        }
        "dotnet" => dotnet_assets(io, donor, root).await.then_some(Reused::Copied),
        _ => None,
    }
}

fn read(path: PathBuf) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Files whose content steers what an install puts into `node_modules`, besides
/// the lockfiles and `package.json`.
const NODE_SETTINGS: &[&str] = &[".npmrc", ".yarnrc", ".yarnrc.yml", "pnpm-workspace.yaml"];
const NODE_LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "yarn.lock",
];

/// What `package.json` says about the packages to install.
const NODE_DEPENDENCIES: &[&str] = &[
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
    "overrides",
    "resolutions",
    "pnpm",
    "workspaces",
    "packageManager",
];

fn same_file(donor: &Path, root: &Path, name: &str) -> bool {
    read(donor.join(name)) == read(root.join(name))
}

fn same_dependencies(donor: &Path, root: &Path) -> bool {
    let package = |dir: &Path| -> Option<Value> { serde_json::from_slice(&read(dir.join("package.json"))?).ok() };
    let (Some(donor), Some(root)) = (package(donor), package(root)) else {
        return false;
    };
    NODE_DEPENDENCIES.iter().all(|key| donor.get(key) == root.get(key))
}

/// The step's project and the members of its workspace, relative to the root.
fn node_dirs(step: &Step) -> Vec<&Path> {
    std::iter::once(step.project.as_path())
        .chain(step.covers.iter().map(PathBuf::as_path))
        .collect()
}

/// A hash of what of the donor a link of its `node_modules` depends on: the
/// lockfiles and install settings, the dependencies of each package and whether
/// the donor has installed them. A link that was made when this was something
/// else follows a tree that is not what the copy needs.
pub(super) fn linked_inputs(donor: &Path, step: &Step) -> Hash {
    let mut digest = Digest::default();
    let project = donor.join(&step.project);
    for name in NODE_LOCKFILES.iter().chain(NODE_SETTINGS) {
        let content = read(project.join(name)).map_or_else(|| "-".to_owned(), |c| Hash::of(&c).to_hex());
        digest.field(name).field(&content);
    }
    for dir in node_dirs(step) {
        let package: Value = read(donor.join(dir).join("package.json"))
            .and_then(|c| serde_json::from_slice(&c).ok())
            .unwrap_or_default();
        let dependencies: serde_json::Map<String, Value> = NODE_DEPENDENCIES
            .iter()
            .filter_map(|key| Some(((*key).to_owned(), package.get(key)?.clone())))
            .collect();
        digest
            .field(&dir.to_string_lossy())
            .field(&Value::Object(dependencies).to_string())
            .field(if donor.join(dir).join("node_modules").is_dir() {
                "installed"
            } else {
                "missing"
            });
    }
    digest.finish()
}

/// Whether the links `node_modules` made for `step` still lead somewhere.
pub(super) fn link_intact(root: &Path, step: &Step) -> bool {
    root.join(&step.project).join("node_modules").is_dir()
}

/// Removes the links `node_modules` made for `step`: the links, never what they lead to.
pub(super) fn unlink(root: &Path, step: &Step) {
    for dir in node_dirs(step) {
        let link = root.join(dir).join("node_modules");
        if link.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
            let _ = remove_link(&link);
        }
    }
}

#[cfg(unix)]
fn remove_link(link: &Path) -> io::Result<()> {
    std::fs::remove_file(link)
}

/// A junction is removed as a directory, which leaves what it leads to.
#[cfg(windows)]
fn remove_link(link: &Path) -> io::Result<()> {
    std::fs::remove_dir(link)
}

/// Links the donor's `node_modules` of the step's project and of the members
/// of its workspace into the copy, if the lockfile is the same on both sides.
fn node_modules(donor: &Path, root: &Path, step: &Step) -> Option<Hash> {
    let (donor_project, project) = (donor.join(&step.project), root.join(&step.project));
    let locked = NODE_LOCKFILES.iter().any(|name| project.join(name).is_file());
    let same_inputs = NODE_LOCKFILES
        .iter()
        .chain(NODE_SETTINGS)
        .all(|name| same_file(&donor_project, &project, name));
    if !locked || !same_inputs {
        return None;
    }
    let dirs = node_dirs(step);
    if !dirs
        .iter()
        .all(|dir| same_dependencies(&donor.join(dir), &root.join(dir)))
    {
        return None;
    }
    // The project's own `node_modules` is required; a member may have none.
    let links: Vec<(PathBuf, PathBuf)> = dirs
        .iter()
        .map(|dir| {
            (
                donor.join(dir).join("node_modules"),
                root.join(dir).join("node_modules"),
            )
        })
        .filter(|(from, _)| from.is_dir())
        .collect();
    let has_project = links.iter().any(|(_, to)| to == &project.join("node_modules"));
    if !has_project || links.iter().any(|(_, to)| to.symlink_metadata().is_ok()) {
        return None;
    }
    links
        .iter()
        .all(|(from, to)| link_dir(from, to).is_ok())
        .then(|| linked_inputs(donor, step))
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// A junction, which needs no privilege, unlike a symbolic link.
#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a junction is made by the shell and nothing else"
    )]
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(String::from_utf8_lossy(&out.stdout).into_owned()))
    }
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn is_project(path: &Path) -> bool {
    has_extension(path, &["csproj", "fsproj", "vbproj"])
}

/// The files a NuGet restore reads: the projects, what they import (`.props`,
/// `.targets`), NuGet's settings and lock files, and the SDK pin. Build output
/// is not an input.
fn is_dotnet_input(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let generated = path
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some("obj" | "bin")));
    !generated
        && (is_project(path)
            || has_extension(path, &["sln", "slnx", "props", "targets"])
            || ["nuget.config", "global.json", "packages.lock.json"].contains(&name.as_str()))
}

/// What `dotnet restore` leaves next to a project, which design-time loading reads.
fn is_restore_output(name: &str) -> bool {
    name == "project.assets.json"
        || name == "project.nuget.cache"
        || name.ends_with(".nuget.g.props")
        || name.ends_with(".nuget.g.targets")
        || name.ends_with(".nuget.dgspec.json")
}

/// Copies the donor's restore output into the copy's projects if the projects
/// and everything else a restore reads are the same on both sides. The
/// packages themselves are in the global packages folder, which both share.
async fn dotnet_assets(io: &Budget, donor: &Path, root: &Path) -> bool {
    let (Ok(theirs), Ok(ours)) = (list_files(io, donor).await, list_files(io, root).await) else {
        return false;
    };
    let (donor, root) = (donor.to_owned(), root.to_owned());
    io.blocking(move || {
        let inputs =
            |files: Vec<PathBuf>| -> BTreeSet<PathBuf> { files.into_iter().filter(|f| is_dotnet_input(f)).collect() };
        let (theirs, ours) = (inputs(theirs), inputs(ours));
        if ours.is_empty() || theirs != ours || !ours.iter().all(|f| read(donor.join(f)) == read(root.join(f))) {
            return false;
        }
        let projects: Vec<&Path> = ours
            .iter()
            .filter(|f| is_project(f))
            .filter_map(|f| f.parent())
            .collect();
        if !projects
            .iter()
            .all(|dir| donor.join(dir).join("obj/project.assets.json").is_file())
        {
            return false;
        }
        projects
            .iter()
            .all(|dir| copy_restore_output(&donor.join(dir).join("obj"), &root.join(dir).join("obj")).is_ok())
    })
    .await
    .unwrap_or(false)
}

fn copy_restore_output(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let name = entry.file_name();
        if entry.file_type()?.is_file() && is_restore_output(&name.to_string_lossy()) {
            std::fs::copy(entry.path(), to.join(name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        dir
    }

    fn step(project: &str, covers: &[&str]) -> Step {
        let mut step = Step::new(Path::new(project), "pnpm", &["install"]);
        step.covers = covers.iter().map(PathBuf::from).collect();
        step
    }

    const PACKAGE: &str = r#"{"name":"p","dependencies":{"a":"1"},"scripts":{"build":"x"}}"#;

    fn node_trees(copy: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir) {
        let donor = tree(&[
            ("package.json", PACKAGE),
            ("pnpm-lock.yaml", "lock"),
            ("node_modules/a/index.js", ""),
        ]);
        let mut files = vec![("package.json", PACKAGE), ("pnpm-lock.yaml", "lock")];
        files.extend_from_slice(copy);
        (donor, tree(&files))
    }

    #[test]
    fn node_modules_are_linked_when_the_lockfile_and_dependencies_are_the_same() {
        let (donor, copy) = node_trees(&[]);
        assert!(node_modules(donor.path(), copy.path(), &step("", &[])).is_some());
        assert!(copy.path().join("node_modules/a/index.js").is_file());
    }

    #[test]
    fn other_lockfiles_dependencies_or_settings_mean_a_restore() {
        let different = |copy: &[(&str, &str)]| {
            let (donor, copy) = node_trees(copy);
            let linked = node_modules(donor.path(), copy.path(), &step("", &[])).is_some();
            assert_eq!(linked, copy.path().join("node_modules").exists());
            !linked
        };
        // `tree` writes later entries over earlier ones.
        assert!(different(&[("pnpm-lock.yaml", "other lock")]));
        assert!(different(&[(
            "package.json",
            r#"{"name":"p","dependencies":{"a":"2"}}"#
        )]));
        assert!(different(&[(".npmrc", "node-linker=hoisted")]));
        // Scripts do not steer the install.
        assert!(!different(&[(
            "package.json",
            r#"{"name":"p","dependencies":{"a":"1"},"scripts":{}}"#
        )]));
    }

    #[test]
    fn without_a_lockfile_or_a_donor_install_nothing_is_reused() {
        let donor = tree(&[("package.json", PACKAGE)]);
        let copy = tree(&[("package.json", PACKAGE)]);
        assert!(node_modules(donor.path(), copy.path(), &step("", &[])).is_none());

        let donor = tree(&[("package.json", PACKAGE), ("pnpm-lock.yaml", "lock")]);
        let copy = tree(&[("package.json", PACKAGE), ("pnpm-lock.yaml", "lock")]);
        assert!(node_modules(donor.path(), copy.path(), &step("", &[])).is_none());
        assert!(!copy.path().join("node_modules").exists());
    }

    #[test]
    fn workspace_members_are_linked_and_each_must_have_the_same_dependencies() {
        let files = |member: &'static str| {
            [
                ("package.json", PACKAGE),
                ("pnpm-lock.yaml", "lock"),
                ("packages/a/package.json", member),
            ]
        };
        let donor = tree(
            &[
                files("{}").as_slice(),
                &[("node_modules/x", ""), ("packages/a/node_modules/y", "")],
            ]
            .concat(),
        );
        let copy = tree(&files("{}"));
        assert!(node_modules(donor.path(), copy.path(), &step("", &["packages/a"])).is_some());
        assert!(copy.path().join("packages/a/node_modules/y").is_file());

        let copy = tree(&files(r#"{"dependencies":{"z":"1"}}"#));
        assert!(node_modules(donor.path(), copy.path(), &step("", &["packages/a"])).is_none());
        assert!(!copy.path().join("node_modules").exists());
    }

    #[test]
    fn the_inputs_of_a_nuget_restore_are_projects_what_they_import_and_the_settings() {
        for input in [
            "App.csproj",
            "src/Lib.fsproj",
            "App.sln",
            "Directory.Build.props",
            "Directory.Packages.props",
            "build/Common.targets",
            "NuGet.Config",
            "global.json",
            "src/packages.lock.json",
        ] {
            assert!(is_dotnet_input(Path::new(input)), "{input}");
        }
        for other in [
            "Program.cs",
            "src/App/obj/App.csproj.nuget.g.props",
            "bin/x.targets",
            "README.md",
        ] {
            assert!(!is_dotnet_input(Path::new(other)), "{other}");
        }
    }

    const PROJECT: &str = "<Project><ItemGroup><PackageReference Include=\"A\" Version=\"1\" /></ItemGroup></Project>";

    #[tokio::test]
    async fn the_restore_output_of_the_projects_is_copied_when_the_inputs_are_the_same() {
        let donor = tree(&[
            ("App.csproj", PROJECT),
            ("Program.cs", "changed"),
            ("obj/project.assets.json", "{}"),
            ("obj/App.csproj.nuget.g.props", "<Project/>"),
            ("obj/Debug/App.dll", "binary"),
        ]);
        let copy = tree(&[("App.csproj", PROJECT), ("Program.cs", "base")]);
        assert!(dotnet_assets(&Budget::new(1), donor.path(), copy.path()).await);
        assert!(copy.path().join("obj/project.assets.json").is_file());
        assert!(copy.path().join("obj/App.csproj.nuget.g.props").is_file());
        assert!(!copy.path().join("obj/Debug").exists());
    }

    #[tokio::test]
    async fn another_package_reference_or_a_missing_restore_means_a_restore() {
        let donor = tree(&[("App.csproj", PROJECT), ("obj/project.assets.json", "{}")]);
        let other = tree(&[("App.csproj", "<Project/>")]);
        let io = Budget::new(1);
        assert!(!dotnet_assets(&io, donor.path(), other.path()).await);
        let extra = tree(&[("App.csproj", PROJECT), ("Directory.Build.props", "<Project/>")]);
        assert!(!dotnet_assets(&io, donor.path(), extra.path()).await);
        assert!(!other.path().join("obj").exists() && !extra.path().join("obj").exists());

        let unrestored = tree(&[("App.csproj", PROJECT)]);
        assert!(!dotnet_assets(&io, unrestored.path(), unrestored.path()).await);
    }
}
