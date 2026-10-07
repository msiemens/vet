//! The spike's corpus: one real git changeset (`base..head` of a repo), with
//! hunks and inline marks computed per file.
//!
//! Default: ruff 0.11.12..0.11.13 (649 files, ~5k hunks, a 10k-line file),
//! shallow-fetched into `.corpus/ruff` on first use.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use similar::{Algorithm, DiffTag, TextDiff};
use vet_core::content::LineRange;
use vet_core::inline::join_close_hunks;
use vet_protocol::{Hunk, Mark, MarkKind};

pub const MIN_FILES: usize = 300;
pub const MIN_HUNKS: usize = 3000;
pub const MIN_BIG_FILE_LINES: usize = 10_000;
pub const MIN_MARKS_PER_HUNK: usize = 3;

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
    pub marks: Vec<Mark>,
    /// Hunks with fewer than `MIN_MARKS_PER_HUNK` marks (the text had no room).
    pub short_hunks: usize,
}

pub struct Corpus {
    pub name: String,
    pub files: Vec<File>,
}

pub struct Stats {
    pub files: usize,
    pub hunks: usize,
    pub biggest: (String, usize),
    pub hunks_with_few_marks: usize,
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
        // A hunk in a near-empty file has no text to mark; tolerate under 1%.
        if self.hunks_with_few_marks * 100 > self.hunks {
            out.push(format!(
                "{} hunks with < {MIN_MARKS_PER_HUNK} marks",
                self.hunks_with_few_marks
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
            .map(|f| (f.path.clone(), line_count(&f.target).max(line_count(&f.base))))
            .max_by_key(|f| f.1);
        Stats {
            files: self.files.len(),
            hunks: self.files.iter().map(|f| f.hunks.len()).sum(),
            biggest: biggest.unwrap_or_default(),
            hunks_with_few_marks: self.files.iter().map(|f| f.short_hunks).sum(),
        }
    }
}

/// Lines that carry a hunk's marks: its changed target lines, or for a pure
/// deletion the context right after it.
fn hunk_mark_lines(hunk: &Hunk, total: u32) -> (u32, u32) {
    if hunk.target_len > 0 {
        (hunk.target_start, hunk.target_start + hunk.target_len)
    } else {
        (
            hunk.target_start.min(total.saturating_sub(1)),
            (hunk.target_start + 3).min(total),
        )
    }
}

pub fn line_count(text: &str) -> usize {
    text.matches('\n').count() + 1
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
        let (hunks, marks, short_hunks) = analyse(&base, &target);
        if !hunks.is_empty() {
            files.push(File {
                path,
                base,
                target,
                hunks,
                marks,
                short_hunks,
            });
        }
    }
    Ok(Corpus {
        name: format!("{} {}..{}", cfg.url, cfg.base, cfg.head),
        files,
    })
}

fn analyse(base: &str, target: &str) -> (Vec<Hunk>, Vec<Mark>, usize) {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .diff_lines(base, target);
    let base_lines: Vec<&str> = base.split_inclusive('\n').collect();
    let range = |r: std::ops::Range<usize>| LineRange {
        start: r.start as u32,
        end: r.end as u32,
    };
    let hunks: Vec<Hunk> = join_close_hunks(
        diff.ops().iter().filter(|op| op.tag() != DiffTag::Equal).map(|op| {
            let (_, old, new) = op.as_tag_tuple();
            vet_core::content::Hunk {
                old: range(old),
                new: range(new),
            }
        }),
        |r| base_lines[r.start as usize..r.end as usize].concat(),
    )
    .into_iter()
    .map(|h| Hunk {
        base_start: h.old.start,
        base_len: h.old.len(),
        target_start: h.new.start,
        target_len: h.new.len(),
    })
    .collect();
    let lines: Vec<&str> = target.split('\n').collect();
    let (mut marks, mut short) = (vec![], 0);
    for hunk in &hunks {
        let found = marks_for(hunk, &lines);
        // A deleted file has no target text to mark.
        short += usize::from(found.len() < MIN_MARKS_PER_HUNK && !target.is_empty());
        marks.extend(found);
    }
    (hunks, marks, short)
}

/// Inline marks for one hunk: identifier-like tokens on its lines (stand-ins for
/// changed symbols, reachability and notes), at least `MIN_MARKS_PER_HUNK`
/// where the text allows.
fn marks_for(hunk: &Hunk, lines: &[&str]) -> Vec<Mark> {
    let (lo, hi) = hunk_mark_lines(hunk, lines.len() as u32);
    let mut marks = vec![];
    // Identifiers first; failing that any non-blank text, also looking before the hunk.
    for (token_ok, before) in [
        (is_ident_char as fn(char) -> bool, 0),
        (|c: char| !c.is_whitespace(), 8),
    ] {
        marks.clear();
        for line in lo.saturating_sub(before)..(hi + 8).min(lines.len() as u32) {
            tokens(lines[line as usize], token_ok, line, &mut marks);
            if line >= hi.saturating_sub(1) && marks.len() > MIN_MARKS_PER_HUNK {
                break;
            }
        }
        if marks.len() >= MIN_MARKS_PER_HUNK {
            break;
        }
    }
    marks.truncate(MIN_MARKS_PER_HUNK + 1);
    for (i, mark) in marks.iter_mut().enumerate() {
        mark.kind = [MarkKind::Symbol, MarkKind::Reachability, MarkKind::Note][i % 3];
    }
    marks
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn tokens(line: &str, ok: fn(char) -> bool, line_no: u32, out: &mut Vec<Mark>) {
    let (mut col, mut start) = (0u32, None);
    for c in line.chars().chain(std::iter::once(' ')) {
        match (ok(c), start) {
            (true, None) => start = Some(col),
            (false, Some(from)) => {
                out.push(Mark {
                    line: line_no,
                    from,
                    to: col,
                    kind: MarkKind::Symbol,
                });
                start = None;
            }
            _ => {}
        }
        col += c.len_utf16() as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            corpus.files[1].hunks,
            [Hunk {
                base_start: 0,
                base_len: 1,
                target_start: 0,
                target_len: 0
            }]
        );
        assert_eq!(
            corpus.files[2].hunks,
            [Hunk {
                base_start: 0,
                base_len: 0,
                target_start: 0,
                target_len: 2
            }]
        );
        // Every hunk has marks, including the pure deletion, and columns are UTF-16.
        assert_eq!(corpus.stats().hunks_with_few_marks, 0);
        let new = &corpus.files[2];
        assert!(
            new.marks.iter().any(|m| m.line == 1 && m.from == 3 && m.to == 8),
            "{:?}",
            new.marks
        );
    }
}
