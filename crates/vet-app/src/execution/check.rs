//! The run check (§7.5): a guard against running build changes you have not
//! looked at. Pure decision logic; the use case in `run` supplies the inputs.

use std::collections::BTreeMap;

use vet_core::content::{Hash, RepoPath, Snapshot};
use vet_core::filter::ReviewFilter;
use vet_core::merkle::snapshot_id;

/// Which files can make a run execute different code. Patterns follow
/// [`ReviewFilter`] globs: without a `/` they match the file name at any depth.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RunCheckConfig {
    pub executor_files: Vec<String>,
}

impl Default for RunCheckConfig {
    fn default() -> Self {
        let patterns = [
            // Rust
            "build.rs",
            "Cargo.toml",
            "**/.cargo/config.toml",
            "**/.cargo/config",
            // TypeScript and JavaScript
            "package.json",
            ".npmrc",
            ".yarnrc.yml",
            "package-lock.json",
            "npm-shrinkwrap.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lock",
            "bun.lockb",
            // Python
            "setup.py",
            "setup.cfg",
            "pyproject.toml",
            // C#
            "*.csproj",
            "*.targets",
            "*.props",
            "Directory.Build.*",
            // CI configs
            ".github/workflows/*",
            ".gitlab-ci.yml",
            "azure-pipelines.yml",
            ".azure-pipelines/**",
            ".circleci/config.yml",
            "Jenkinsfile",
            // Build scripts
            "Makefile",
            "GNUmakefile",
            "justfile",
            "Justfile",
            "Dockerfile",
            "build.*",
            "ci/**",
        ];
        Self {
            executor_files: patterns.map(String::from).to_vec(),
        }
    }
}

impl RunCheckConfig {
    /// The executor-relevant entries of `snapshot`, as a snapshot of their own:
    /// its id is the run-check hash, and the existing diff path applies to it.
    pub fn relevant(&self, snapshot: &Snapshot) -> Snapshot {
        let entries: BTreeMap<_, _> = if self.executor_files.is_empty() {
            BTreeMap::new()
        } else {
            let filter = ReviewFilter {
                include: self.executor_files.clone(),
                ..Default::default()
            };
            let compiled = filter.compile();
            snapshot
                .entries
                .iter()
                .filter(|(path, _)| compiled.passes(path))
                .map(|(path, entry)| (path.clone(), entry.clone()))
                .collect()
        };
        Snapshot {
            id: snapshot_id(&entries),
            entries,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangedFile {
    pub path: RepoPath,
    pub change: Change,
}

/// What the "run anyway?" dialog shows. `baseline` and `current` are
/// executor-relevant snapshots, so diffing them yields exactly the changes in
/// `changed`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Approval {
    /// Recorded when the user approves.
    pub hash: Hash,
    /// No run was ever approved in this repo.
    pub first_run: bool,
    pub changed: Vec<ChangedFile>,
    pub baseline: Snapshot,
    pub current: Snapshot,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RunCheck {
    Clear,
    NeedsApproval(Approval),
}

/// `current` is the executor-relevant part of the snapshot about to run. `last`
/// is the hash of the last approval and `baseline` its entries (`None` if they
/// are no longer stored, which makes every current file count as added).
pub fn decide(current: Snapshot, last: Option<Hash>, baseline: Option<Snapshot>) -> RunCheck {
    if last == Some(current.id.0) {
        return RunCheck::Clear;
    }
    let baseline = baseline.unwrap_or_else(|| {
        let entries = BTreeMap::new();
        Snapshot {
            id: snapshot_id(&entries),
            entries,
        }
    });
    let mut changed = Vec::new();
    for (path, entry) in &current.entries {
        match baseline.entries.get(path) {
            None => changed.push(ChangedFile {
                path: path.clone(),
                change: Change::Added,
            }),
            Some(old) if old != entry => changed.push(ChangedFile {
                path: path.clone(),
                change: Change::Modified,
            }),
            Some(_) => {}
        }
    }
    for path in baseline.entries.keys().filter(|p| !current.entries.contains_key(*p)) {
        changed.push(ChangedFile {
            path: path.clone(),
            change: Change::Removed,
        });
    }
    RunCheck::NeedsApproval(Approval {
        hash: current.id.0,
        first_run: last.is_none(),
        changed,
        baseline,
        current,
    })
}

#[cfg(test)]
mod tests {
    use vet_core::content::{BlobId, EntryKind, FileEntry};

    use super::*;

    fn snapshot(files: &[(&str, &str)]) -> Snapshot {
        let entries: BTreeMap<_, _> = files
            .iter()
            .map(|(path, content)| {
                let entry = FileEntry {
                    blob: BlobId::of(content.as_bytes()),
                    mode: 0o100644,
                    kind: EntryKind::File,
                    unfiltered: false,
                };
                (RepoPath(path.as_bytes().to_vec()), entry)
            })
            .collect();
        Snapshot {
            id: snapshot_id(&entries),
            entries,
        }
    }

    fn relevant(files: &[(&str, &str)]) -> Snapshot {
        RunCheckConfig::default().relevant(&snapshot(files))
    }

    fn paths(s: &Snapshot) -> Vec<&str> {
        s.entries.keys().map(|p| std::str::from_utf8(&p.0).unwrap()).collect()
    }

    #[test]
    fn defaults_select_executor_files_only() {
        let s = relevant(&[
            ("src/lib.rs", ""),
            ("src/build.rs", ""),
            ("crates/a/Cargo.toml", ""),
            ("a/.cargo/config.toml", ""),
            ("ui/package.json", ""),
            ("ui/src/app.ts", ""),
            ("App/App.csproj", ""),
            ("Directory.Build.props", ""),
            (".github/workflows/ci.yml", ""),
            ("docs/.github/workflows/x.yml", ""),
            ("README.md", ""),
        ]);
        assert_eq!(
            paths(&s),
            [
                ".github/workflows/ci.yml",
                "App/App.csproj",
                "Directory.Build.props",
                "a/.cargo/config.toml",
                "crates/a/Cargo.toml",
                "src/build.rs",
                "ui/package.json",
            ]
        );
    }

    #[test]
    fn an_empty_list_selects_nothing() {
        let config = RunCheckConfig { executor_files: vec![] };
        assert!(config.relevant(&snapshot(&[("Cargo.toml", "")])).entries.is_empty());
    }

    #[test]
    fn the_first_run_ever_prompts() {
        let current = relevant(&[("Cargo.toml", "a")]);
        let RunCheck::NeedsApproval(a) = decide(current.clone(), None, None) else {
            panic!("expected a prompt");
        };
        assert!(a.first_run);
        assert_eq!(a.hash, current.id.0);
        assert_eq!(a.changed.len(), 1);
        assert_eq!(a.changed[0].change, Change::Added);
    }

    #[test]
    fn the_first_run_prompts_even_without_executor_files() {
        assert!(matches!(decide(relevant(&[]), None, None), RunCheck::NeedsApproval(_)));
    }

    #[test]
    fn an_unchanged_build_surface_runs_without_asking() {
        let approved = relevant(&[("Cargo.toml", "a"), ("package.json", "{}")]);
        // Source edits and new non-executor files do not change the relevant part.
        let now = relevant(&[
            ("Cargo.toml", "a"),
            ("package.json", "{}"),
            ("src/main.rs", "changed"),
            ("notes.md", "new"),
        ]);
        assert_eq!(
            decide(now.clone(), Some(approved.id.0), Some(approved)),
            RunCheck::Clear
        );
    }

    #[test]
    fn an_edited_package_json_prompts_with_the_changes() {
        let approved = relevant(&[
            ("Cargo.toml", "a"),
            ("package.json", r#"{"scripts":{}}"#),
            ("setup.py", ""),
        ]);
        let now = relevant(&[
            ("Cargo.toml", "a"),
            ("package.json", r#"{"scripts":{"dev":"rm -rf"}}"#),
            ("build.rs", "fn main(){}"),
        ]);
        let RunCheck::NeedsApproval(a) = decide(now.clone(), Some(approved.id.0), Some(approved.clone())) else {
            panic!("expected a prompt");
        };
        assert!(!a.first_run);
        let changed: Vec<_> = a
            .changed
            .iter()
            .map(|c| (std::str::from_utf8(&c.path.0).unwrap(), c.change))
            .collect();
        assert_eq!(
            changed,
            [
                ("build.rs", Change::Added),
                ("package.json", Change::Modified),
                ("setup.py", Change::Removed)
            ]
        );
        assert_eq!(a.baseline, approved);
        assert_eq!(a.current, now);
    }

    #[test]
    fn a_mode_change_counts() {
        let approved = relevant(&[("Makefile", "a")]);
        let mut now = approved.clone();
        now.entries.values_mut().for_each(|e| e.mode = 0o100755);
        now.id = snapshot_id(&now.entries);
        assert!(matches!(
            decide(now, Some(approved.id.0), Some(approved)),
            RunCheck::NeedsApproval(_)
        ));
    }

    #[test]
    fn a_missing_baseline_lists_everything_as_added() {
        let approved = relevant(&[("Cargo.toml", "a")]);
        let now = relevant(&[("Cargo.toml", "b")]);
        let RunCheck::NeedsApproval(a) = decide(now, Some(approved.id.0), None) else {
            panic!("expected a prompt");
        };
        assert!(!a.first_run);
        assert_eq!(a.changed[0].change, Change::Added);
    }
}
