//! The spike's corpus: one real git changeset (`base..head` of a repo), with
//! hunks computed per file.
//!
//! Default: ruff 0.11.12..0.11.13 (649 files, ~5k hunks, a 10k-line file),
//! shallow-fetched into `.corpus/ruff` on first use.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use vet_app::editor::line_count;
use vet_core::content::Hunk;
use vet_core::hunks::line_hunks;

pub const MIN_FILES: usize = 300;
pub const MIN_HUNKS: usize = 3000;
pub const MIN_BIG_FILE_LINES: usize = 10_000;

pub struct Config {
    pub url: String,
    pub repo: PathBuf,
    pub base: String,
    pub head: String,
}

impl Config {
    /// `--url/--repo/--base/--head` flags, then `VET_CORPUS_*` env vars, then the
    /// default corpus.
    pub fn from_args(args: &[String]) -> Self {
        let get = |flag: &str, env: &str, default: &str| {
            args.windows(2)
                .find(|pair| pair[0] == flag)
                .map(|pair| pair[1].clone())
                .or_else(|| std::env::var(env).ok())
                .unwrap_or_else(|| default.to_owned())
        };
        let default_repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.corpus/ruff");
        Self {
            url: get("--url", "VET_CORPUS_URL", "https://github.com/astral-sh/ruff.git"),
            repo: get("--repo", "VET_CORPUS_REPO", &default_repo.to_string_lossy()).into(),
            base: get("--base", "VET_CORPUS_BASE", "0.11.12"),
            head: get("--head", "VET_CORPUS_HEAD", "0.11.13"),
        }
    }
}

pub struct File {
    pub path: String,
    pub base: String,
    pub target: String,
    pub hunks: Vec<Hunk>,
}

pub struct Corpus {
    pub name: String,
    pub files: Vec<File>,
}

pub struct Stats {
    pub files: usize,
    pub hunks: usize,
    pub biggest: (String, usize),
}

impl Stats {
    pub fn problems(&self) -> Vec<String> {
        let mut out = vec![];
        if self.files < MIN_FILES {
            out.push(format!("{} files < {MIN_FILES}", self.files));
        }
        if self.hunks < MIN_HUNKS {
            out.push(format!("{} hunks < {MIN_HUNKS}", self.hunks));
        }
        if self.biggest.1 <= MIN_BIG_FILE_LINES {
            out.push(format!(
                "biggest file has {} lines <= {MIN_BIG_FILE_LINES}",
                self.biggest.1
            ));
        }
        out
    }
}

impl Corpus {
    pub fn stats(&self) -> Stats {
        let biggest = self
            .files
            .iter()
            .map(|f| {
                let lines = line_count(f.target.as_bytes()).max(line_count(f.base.as_bytes()));
                (f.path.clone(), lines as usize)
            })
            .max_by_key(|f| f.1);
        Stats {
            files: self.files.len(),
            hunks: self.files.iter().map(|f| f.hunks.len()).sum(),
            biggest: biggest.unwrap_or_default(),
        }
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("git {args:?}: {}", String::from_utf8_lossy(&out.stderr)))
    }
}

/// Makes `base` and `head` resolvable in `repo`, shallow-fetching tags from
/// `url` if they are not.
pub fn ensure_repo(cfg: &Config) -> Result<(), String> {
    let have = |rev: &str| {
        git(
            &cfg.repo,
            &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")],
        )
        .is_ok()
    };
    if cfg.repo.join(".git").exists() && have(&cfg.base) && have(&cfg.head) {
        return Ok(());
    }
    eprintln!(
        "fetching {} {}..{} into {}",
        cfg.url,
        cfg.base,
        cfg.head,
        cfg.repo.display()
    );
    std::fs::create_dir_all(&cfg.repo).map_err(|e| e.to_string())?;
    if !cfg.repo.join(".git").exists() {
        git(&cfg.repo, &["init", "-q"])?;
        git(&cfg.repo, &["remote", "add", "origin", &cfg.url])?;
    }
    for rev in [&cfg.base, &cfg.head] {
        if !have(rev) {
            git(
                &cfg.repo,
                &[
                    "fetch",
                    "-q",
                    "--depth",
                    "1",
                    "origin",
                    &format!("+refs/tags/{rev}:refs/tags/{rev}"),
                ],
            )?;
        }
    }
    Ok(())
}

struct Blobs {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl Blobs {
    fn start(repo: &Path) -> Result<Self, String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run git: {e}"))?;
        let reader = BufReader::new(child.stdout.take().ok_or("no stdout")?);
        Ok(Self { child, reader })
    }

    /// The text of `spec` (`rev:path`): empty if the file is missing on that
    /// side, `None` if it is not UTF-8.
    fn text(&mut self, spec: &str) -> Result<Option<String>, String> {
        let io = |e: std::io::Error| e.to_string();
        writeln!(self.child.stdin.as_mut().ok_or("no stdin")?, "{spec}").map_err(io)?;
        let mut header = String::new();
        self.reader.read_line(&mut header).map_err(io)?;
        let size: usize = match header.trim_end().rsplit_once(' ') {
            Some((_, "missing")) | None => return Ok(Some(String::new())),
            Some((_, size)) => size.parse().map_err(|_| format!("bad header {header:?}"))?,
        };
        let mut bytes = vec![0; size + 1];
        self.reader.read_exact(&mut bytes).map_err(io)?;
        bytes.pop();
        Ok(String::from_utf8(bytes).ok().map(|s| s.replace("\r\n", "\n")))
    }
}

pub fn load(cfg: &Config) -> Result<Corpus, String> {
    ensure_repo(cfg)?;
    let range = [cfg.base.as_str(), cfg.head.as_str()];
    let listing = git(
        &cfg.repo,
        &["diff", "--name-only", "--no-renames", "-z", range[0], range[1]],
    )?;
    let mut blobs = Blobs::start(&cfg.repo)?;
    let mut files = vec![];
    for path in listing.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8_lossy(path).into_owned();
        // Non-UTF-8 files on either side are skipped.
        let base = blobs.text(&format!("{}:{path}", range[0]))?;
        let target = blobs.text(&format!("{}:{path}", range[1]))?;
        let (Some(base), Some(target)) = (base, target) else {
            continue;
        };
        if base.contains('\0') || target.contains('\0') {
            continue;
        }
        let hunks = line_hunks(base.as_bytes(), target.as_bytes());
        if !hunks.is_empty() {
            files.push(File {
                path,
                base,
                target,
                hunks,
            });
        }
    }
    Ok(Corpus {
        name: format!("{} {}..{}", cfg.url, cfg.base, cfg.head),
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vet_core::content::LineRange;

    fn git_ok(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "{args:?}");
    }

    #[test]
    fn loads_a_changeset_from_a_git_repo() {
        let dir = std::env::temp_dir().join(format!("vet-mock-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git_ok(&dir, &["init", "-q"]);
        let body = |n: &str| {
            (0..30)
                .map(|i| format!("let value_{i} = compute({i}); // {n}\n"))
                .collect::<String>()
        };
        std::fs::write(dir.join("a.rs"), body("old")).unwrap();
        std::fs::write(dir.join("gone.txt"), "bye\n").unwrap();
        git_ok(&dir, &["add", "."]);
        git_ok(&dir, &["commit", "-q", "-m", "base"]);
        git_ok(&dir, &["tag", "base"]);
        let mut changed = body("old");
        changed = changed
            .replacen("value_3 ", "other_3 ", 1)
            .replacen("value_20 ", "other_20 ", 1);
        std::fs::write(dir.join("a.rs"), changed).unwrap();
        std::fs::write(dir.join("new.rs"), "fn héllo() {}\nfn wörld() {}\n").unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        git_ok(&dir, &["add", "-A"]);
        git_ok(&dir, &["commit", "-q", "-m", "head"]);
        git_ok(&dir, &["tag", "head"]);

        let corpus = load(&Config {
            url: "unused".into(),
            repo: dir.clone(),
            base: "base".into(),
            head: "head".into(),
        })
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let paths: Vec<_> = corpus.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.rs", "gone.txt", "new.rs"]);
        assert_eq!(corpus.files[0].hunks.len(), 2);
        let range = |start, end| LineRange { start, end };
        assert_eq!(
            corpus.files[1].hunks,
            [Hunk {
                old: range(0, 1),
                new: range(0, 0)
            }]
        );
        assert_eq!(
            corpus.files[2].hunks,
            [Hunk {
                old: range(0, 0),
                new: range(0, 2)
            }]
        );
    }
}
