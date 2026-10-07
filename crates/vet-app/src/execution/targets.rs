//! Run targets (§10): discovered by parsing manifests from snapshot content,
//! never by executing tools, then overlaid with the user's edits.

use vet_core::content::{BlobSource, EntryKind, RepoPath, Snapshot};

/// What to run and how. `id` is stable across snapshots, so an edit made to a
/// discovered target keeps applying to it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RunTarget {
    pub id: String,
    /// The manifest the target was discovered from, relative to the repository
    /// root; `None` for one the user added. Not stored: discovery supplies it.
    pub source: Option<String>,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    /// Relative to the repository root; empty for the root.
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

/// Targets of `snapshot`, in path order. Manifests whose content is unavailable
/// or that do not parse contribute nothing.
pub fn discover(snapshot: &Snapshot, blobs: &dyn BlobSource) -> Vec<RunTarget> {
    let mut out = Vec::new();
    for (path, entry) in &snapshot.entries {
        if entry.kind != EntryKind::File {
            continue;
        }
        let Ok(path) = std::str::from_utf8(&path.0) else {
            continue;
        };
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        if !matches!(name, "Cargo.toml" | "package.json" | "pyproject.toml") {
            continue;
        }
        let Some(text) = blobs.blob(entry.blob).and_then(|b| std::str::from_utf8(b).ok()) else {
            continue;
        };
        match name {
            "Cargo.toml" => cargo(snapshot, path, dir, text, &mut out),
            "package.json" => package_json(snapshot, path, dir, text, &mut out),
            _ => pyproject(path, dir, text, &mut out),
        }
    }
    out
}

/// Discovered targets with stored ones replacing those of the same id; stored
/// targets that match nothing discovered (user-added) follow.
pub fn merge(discovered: Vec<RunTarget>, stored: Vec<RunTarget>) -> Vec<RunTarget> {
    let mut stored = stored;
    let mut out: Vec<RunTarget> = discovered
        .into_iter()
        .map(|d| match stored.iter().position(|s| s.id == d.id) {
            Some(i) => RunTarget {
                source: d.source,
                ..stored.remove(i)
            },
            None => d,
        })
        .collect();
    out.extend(stored);
    out
}

fn join(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        rel.to_string()
    } else {
        format!("{dir}/{rel}")
    }
}

fn has(snapshot: &Snapshot, path: &str) -> bool {
    snapshot.entries.contains_key(&RepoPath::from(path))
}

fn target(source: &str, kind: &str, dir: &str, name: &str, program: &str, args: &[&str]) -> RunTarget {
    RunTarget {
        id: format!("{kind}:{dir}:{name}"),
        source: Some(source.to_string()),
        label: if dir.is_empty() {
            format!("{kind}: {name}")
        } else {
            format!("{kind}: {name} ({dir})")
        },
        program: program.to_string(),
        args: args.iter().map(ToString::to_string).collect(),
        cwd: dir.to_string(),
        env: Vec::new(),
    }
}

fn cargo(snapshot: &Snapshot, source: &str, dir: &str, text: &str, out: &mut Vec<RunTarget>) {
    let Ok(manifest) = text.parse::<toml::Table>() else {
        return;
    };
    let mut bins: Vec<String> = manifest
        .get("bin")
        .and_then(|b| b.as_array())
        .into_iter()
        .flatten()
        .filter_map(|b| b.get("name")?.as_str().map(String::from))
        .collect();
    let package = manifest
        .get("package")
        .and_then(|p| p.get("name")?.as_str())
        .map(String::from);
    if let Some(name) = package
        && has(snapshot, &join(dir, "src/main.rs"))
    {
        bins.push(name);
    }
    let prefix = join(dir, "src/bin/");
    let start = RepoPath::from(prefix.as_str());
    for (path, _) in snapshot.entries.range(start..) {
        let Some(rest) = path.0.strip_prefix(prefix.as_bytes()) else {
            break;
        };
        let Ok(rest) = std::str::from_utf8(rest) else { continue };
        let name = match rest.split_once('/') {
            None => rest.strip_suffix(".rs"),
            Some((name, "main.rs")) => Some(name),
            Some(_) => None,
        };
        bins.extend(name.map(String::from));
    }
    let mut seen = Vec::new();
    for bin in bins {
        if !seen.contains(&bin) {
            out.push(target(source, "cargo", dir, &bin, "cargo", &["run", "--bin", &bin]));
            seen.push(bin);
        }
    }
}

fn package_json(snapshot: &Snapshot, source: &str, dir: &str, text: &str, out: &mut Vec<RunTarget>) {
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(scripts) = manifest.get("scripts").and_then(|s| s.as_object()) else {
        return;
    };
    let manager = if has(snapshot, &join(dir, "pnpm-lock.yaml")) {
        "pnpm"
    } else if has(snapshot, &join(dir, "yarn.lock")) {
        "yarn"
    } else {
        "npm"
    };
    for (name, command) in scripts {
        if command.is_string() {
            out.push(target(source, "script", dir, name, manager, &["run", name]));
        }
    }
}

/// `[project.scripts]` entries are console scripts, which exist only once the
/// project is installed, so the target runs the script by name.
fn pyproject(source: &str, dir: &str, text: &str, out: &mut Vec<RunTarget>) {
    let Ok(manifest) = text.parse::<toml::Table>() else {
        return;
    };
    let scripts = manifest
        .get("project")
        .and_then(|p| p.get("scripts")?.as_table())
        .into_iter()
        .flatten();
    for (name, _) in scripts {
        out.push(target(source, "python", dir, name, name, &[]));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vet_core::content::{BlobId, FileEntry};
    use vet_core::merkle::snapshot_id;

    use super::*;

    fn snapshot(files: &[(&str, &str)]) -> (Snapshot, BTreeMap<BlobId, Vec<u8>>) {
        let mut entries = BTreeMap::new();
        let mut blobs = BTreeMap::new();
        for (path, content) in files {
            let blob = BlobId::of(content.as_bytes());
            blobs.insert(blob, content.as_bytes().to_vec());
            entries.insert(
                RepoPath::from(*path),
                FileEntry {
                    blob,
                    mode: 0o100644,
                    kind: EntryKind::File,
                    unfiltered: false,
                },
            );
        }
        (
            Snapshot {
                id: snapshot_id(&entries),
                entries,
            },
            blobs,
        )
    }

    fn ids(targets: &[RunTarget]) -> Vec<&str> {
        targets.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn cargo_bins_main_and_bin_dir() {
        let (s, b) = snapshot(&[
            (
                "Cargo.toml",
                "[package]\nname = \"app\"\n[[bin]]\nname = \"tool\"\npath = \"x.rs\"\n",
            ),
            ("src/main.rs", ""),
            ("src/bin/extra.rs", ""),
            ("src/bin/dir/main.rs", ""),
            ("src/bin/dir/helper.rs", ""),
        ]);
        let targets = discover(&s, &b);
        assert_eq!(
            ids(&targets),
            ["cargo::tool", "cargo::app", "cargo::dir", "cargo::extra"]
        );
        assert_eq!(targets[0].program, "cargo");
        assert_eq!(targets[0].args, ["run", "--bin", "tool"]);
    }

    #[test]
    fn a_workspace_manifest_has_no_implicit_bin() {
        let (s, b) = snapshot(&[
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/a\"]\n"),
            ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
            ("crates/a/src/main.rs", ""),
        ]);
        let targets = discover(&s, &b);
        assert_eq!(ids(&targets), ["cargo:crates/a:a"]);
        assert_eq!(targets[0].cwd, "crates/a");
    }

    #[test]
    fn package_json_scripts_use_the_lockfile_manager() {
        let (s, b) = snapshot(&[
            (
                "ui/package.json",
                r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#,
            ),
            ("ui/pnpm-lock.yaml", ""),
            ("package.json", r#"{"scripts": {"start": "node ."}}"#),
        ]);
        let targets = discover(&s, &b);
        assert_eq!(ids(&targets), ["script::start", "script:ui:build", "script:ui:dev"]);
        assert_eq!(targets[0].program, "npm");
        assert_eq!(targets[0].source.as_deref(), Some("package.json"));
        assert_eq!(targets[2].source.as_deref(), Some("ui/package.json"));
        assert_eq!(targets[2].program, "pnpm");
        assert_eq!(targets[2].args, ["run", "dev"]);
    }

    #[test]
    fn pyproject_scripts() {
        let (s, b) = snapshot(&[("pyproject.toml", "[project.scripts]\nserve = \"pkg.cli:main\"\n")]);
        let targets = discover(&s, &b);
        assert_eq!(ids(&targets), ["python::serve"]);
        assert_eq!(targets[0].program, "serve");
    }

    #[test]
    fn broken_manifests_are_skipped() {
        let (s, b) = snapshot(&[("Cargo.toml", "[[[ nope"), ("package.json", "{")]);
        assert!(discover(&s, &b).is_empty());
    }

    #[test]
    fn stored_edits_replace_discovered_targets() {
        let (s, b) = snapshot(&[("package.json", r#"{"scripts": {"dev": "vite"}}"#)]);
        let discovered = discover(&s, &b);
        let mut edited = discovered[0].clone();
        edited.env = vec![("PORT".into(), "1".into())];
        let custom = RunTarget {
            id: "custom:1".into(),
            ..edited.clone()
        };
        let merged = merge(discovered, vec![custom.clone(), edited.clone()]);
        assert_eq!(merged, vec![edited, custom]);
    }
}
