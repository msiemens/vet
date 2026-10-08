//! Mock orchestrator for the shell spike (SPEC §12.0): the real transport and
//! handshake, answering from a real git changeset (see `corpus`). Prints the same
//! `Ready` line as the backend.
//!
//! Flags (each also settable through `VET_CORPUS_{URL,REPO,BASE,HEAD}`):
//! `--url`, `--repo`, `--base`, `--head`; `--stats` prints corpus statistics and
//! exits non-zero if §12.0's thresholds are unmet; `--prepare` only fetches the
//! corpus's refs; `--exit-on-stdin-close` makes it exit with its parent shell.

mod corpus;

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use corpus::{Config, Corpus};
use vet_app::changeset::{BaseChoice, CommitId, Source};
use vet_app::editor::{last_text_line, line_count, text_changes};
use vet_app::ports::CommitInfo;
use vet_app::session::{
    BaseView, FileStatus, FileView, Lifecycle, ManifestView, ReviewUpdateView, ReviewView, TargetView,
};
use vet_core::completion::{Completion, Coverage};
use vet_core::content::{FileDiff, Hash, LineDelta, RepoPath, SnapshotId};
use vet_core::filter::ReviewFilter;
use vet_core::marking::hunk_span;
use vet_core::review::{ReviewedSpan, target_line};
use vet_highlight::{Cache, Language};
use vet_protocol::{ProjectId, Request, Response};
use vet_transport::shell::{announce, exit_when_stdin_closes};
use vet_transport::{Events, Handler, Server};

struct Mock {
    corpus: Corpus,
    index: HashMap<String, usize>,
    /// The diff of each corpus file.
    diffs: Vec<FileDiff>,
    /// The span that marking a hunk seen creates, per file and hunk.
    spans: Vec<Vec<Option<ReviewedSpan>>>,
    /// Review state: one seen flag per hunk per file. It lives here, not in the shell (I5).
    seen: Mutex<Vec<Vec<bool>>>,
    /// Highlights by file and side (true for the target).
    highlights: Cache<(usize, bool)>,
}

const HIGHLIGHT_CACHE_BYTES: usize = 64 << 20;
const PROJECT: ProjectId = 0;
const REVISION: u32 = 0;
/// The corpus has no snapshots; every span is anchored in this one.
const SNAPSHOT: SnapshotId = SnapshotId(Hash([0; 32]));

impl Mock {
    fn new(corpus: Corpus) -> Self {
        let index = corpus
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i))
            .collect();
        let diffs: Vec<FileDiff> = corpus
            .files
            .iter()
            .map(|f| FileDiff {
                old_path: Some(RepoPath::from(f.path.as_str())),
                new_path: Some(RepoPath::from(f.path.as_str())),
                old_blob: None,
                new_blob: None,
                hunks: f.hunks.clone(),
            })
            .collect();
        let spans = corpus
            .files
            .iter()
            .zip(&diffs)
            .map(|(f, diff)| {
                let texts = (Some(f.base.as_bytes()), Some(f.target.as_bytes()));
                (diff.hunks.iter())
                    .map(|h| hunk_span(diff, h, (SNAPSHOT, SNAPSHOT), texts))
                    .collect()
            })
            .collect();
        let seen = Mutex::new(corpus.files.iter().map(|f| vec![false; f.hunks.len()]).collect());
        Self {
            corpus,
            index,
            diffs,
            spans,
            seen,
            highlights: Cache::new(HIGHLIGHT_CACHE_BYTES),
        }
    }

    /// The spans of the hunks marked seen.
    fn seen_spans(&self, seen: &[Vec<bool>]) -> Vec<ReviewedSpan> {
        (seen.iter().zip(&self.spans))
            .flat_map(|(flags, spans)| flags.iter().zip(spans))
            .filter_map(|(&seen, span)| span.clone().filter(|_| seen))
            .collect()
    }

    /// The corpus has no filter, non-text items or old base.
    fn completion(coverage: &Coverage, diffs: &[FileDiff]) -> Completion {
        coverage.completion(&ReviewFilter::default(), diffs, &[], &[])
    }

    /// The answer to a change of the review state of file `i`.
    fn review_update(&self, i: usize, seen: &[Vec<bool>]) -> Response {
        let spans = self.seen_spans(seen);
        let coverage = Coverage::new(&spans);
        let diff = &self.diffs[i];
        Response::Review(vet_mapping::review_update(&ReviewUpdateView {
            revision: REVISION,
            path: diff.path().clone(),
            review: Mock::review(&coverage, diff),
            completion: Mock::completion(&coverage, std::slice::from_ref(diff)),
            changeset: Mock::completion(&coverage, &self.diffs),
        }))
    }

    fn review(coverage: &Coverage, diff: &FileDiff) -> ReviewView {
        ReviewView {
            hunks: coverage.hunk_states(diff),
            old_base_lines: coverage.old_base_lines(diff),
            acknowledged: vec![],
        }
    }
}

impl Handler for Mock {
    async fn handle(&self, request: Request, _events: Events) -> Result<Response, String> {
        let position = |path: &str| {
            self.index
                .get(path)
                .copied()
                .ok_or_else(|| format!("no such file: {path}"))
        };
        let seen = || self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let current = |project: ProjectId, revision: u32| match (project == PROJECT, revision == REVISION) {
            (true, true) => Ok(()),
            (true, false) => Err("stale revision".to_string()),
            _ => Err("no such project".to_string()),
        };
        match request {
            Request::Ping { payload } => Ok(Response::Pong { payload }),
            Request::Accounts => Ok(Response::Accounts { accounts: Vec::new() }),
            Request::SetCredentials { .. } | Request::RemoveCredentials { .. } => {
                Err("the corpus has no hosting platform".into())
            }
            Request::RecentProjects | Request::RecordRecentProject { .. } | Request::ForgetRecentProject { .. } => {
                Ok(Response::RecentProjects { projects: Vec::new() })
            }
            // The mock has one project over its corpus; `repo` is ignored.
            Request::OpenProject { repo } => Ok(Response::Project {
                project: PROJECT,
                root: repo,
            }),
            Request::CloseProject { .. } => Ok(Response::Done),
            Request::Tasks { .. } => Ok(Response::Tasks { tasks: Vec::new() }),
            Request::Manifest { .. } => {
                let spans = self.seen_spans(&seen());
                let coverage = Coverage::new(&spans);
                let files: Vec<FileView> = (self.corpus.files.iter().zip(&self.diffs))
                    .map(|(f, diff)| FileView {
                        path: diff.path().clone(),
                        old_path: None,
                        status: FileStatus::Modified,
                        base_lines: line_count(f.base.as_bytes()),
                        target_lines: line_count(f.target.as_bytes()),
                        hunks: f.hunks.clone(),
                        delta: LineDelta::of(&f.hunks),
                        non_text: vec![],
                        review: Mock::review(&coverage, diff),
                        completion: Mock::completion(&coverage, std::slice::from_ref(diff)),
                    })
                    .collect();
                let view = ManifestView {
                    revision: REVISION,
                    name: self.corpus.name.clone(),
                    lifecycle: Lifecycle::Browsing,
                    base: BaseView {
                        choice: BaseChoice::MergeBase {
                            reference: "main".into(),
                        },
                        commit: CommitInfo {
                            id: CommitId("0123456789abcdef0123456789abcdef01234567".into()),
                            subject: "Spike corpus".into(),
                            author: "Vet".into(),
                            time: 0,
                        },
                        name: "main".into(),
                        behind: None,
                        contained_in: None,
                    },
                    target: TargetView {
                        source: Source::WorkingTree,
                        name: "Working tree".into(),
                        branch: Some("main".into()),
                        head: None,
                    },
                    filter: ReviewFilter::default(),
                    delta: files.iter().map(|f| f.delta).sum(),
                    completion: Mock::completion(&coverage, &self.diffs),
                    changeset_completion: Mock::completion(&coverage, &self.diffs),
                    files,
                    degraded: None,
                    interdiff: None,
                    lens: vet_app::changeset::Lens::Squash,
                    commits: 0,
                    changeset_files: self.corpus.files.len() as u32,
                };
                Ok(Response::Manifest(Box::new(vet_mapping::manifest(PROJECT, &view, &[]))))
            }
            Request::FileContents { path, .. } => {
                let at = position(&path)?;
                let f = &self.corpus.files[at];
                let changes = text_changes(&f.base, &f.target, &f.hunks);
                // The corpus has no content ids: a side of a file is its own.
                let highlights = |is_target: bool, text: &str| match Language::of_path(&f.path) {
                    Some(language) => self.highlights.get(&(at, is_target), text, language).to_vec(),
                    None => Vec::new(),
                };
                Ok(Response::FileContents {
                    revision: REVISION,
                    base: Some(f.base.clone()),
                    target: Some(f.target.clone()),
                    base_highlights: highlights(false, &f.base),
                    target_highlights: highlights(true, &f.target),
                    changes: changes.iter().map(vet_mapping::text_change).collect(),
                })
            }
            Request::Versions { .. } => Ok(Response::Versions { versions: Vec::new() }),
            Request::Commits { .. } => Ok(Response::Commits {
                revision: 0,
                lens: vet_protocol::Lens::Squash,
                commits: Vec::new(),
            }),
            Request::SetLens { .. } => Err("the corpus is one changeset".into()),
            Request::ToggleHunk {
                project,
                revision,
                path,
                hunk,
                ..
            } => {
                current(project, revision)?;
                let i = position(&path)?;
                let mut all = seen();
                let flag = all[i].get_mut(hunk as usize).ok_or("no such hunk")?;
                *flag = !*flag;
                Ok(self.review_update(i, &all))
            }
            Request::ToggleFileSeen {
                project,
                revision,
                path,
                ..
            } => {
                current(project, revision)?;
                let i = position(&path)?;
                let mut all = seen();
                let value = !all[i].iter().all(|&seen| seen);
                all[i].fill(value);
                Ok(self.review_update(i, &all))
            }
            Request::ToggleItem { .. } | Request::ItemBlob { .. } => Err("the corpus has no non-text items".into()),
            Request::ReconfirmFile { .. } | Request::BaseDiff { .. } => Err("the corpus has one base".into()),
            Request::SetBase { .. } | Request::ListBaseCandidates { .. } => Err("the corpus has one base".into()),
            Request::Fetch { .. } => Ok(Response::Done),
            Request::SetProjectFilter { .. } | Request::ExcludePath { .. } => Err("the corpus has no filter".into()),
            Request::FinishReview { .. } => Err("the corpus has no review to finish".into()),
            Request::ListDirectory { .. } | Request::FileText { .. } => Err("the corpus has no file tree".into()),
            Request::TargetLine {
                project,
                revision,
                side,
                path,
                line,
                ..
            } => {
                current(project, revision)?;
                let f = &self.corpus.files[position(&path)?];
                let line =
                    target_line(&f.hunks, vet_mapping::core_side(side), line).min(last_text_line(f.target.as_bytes()));
                Ok(Response::TargetLine { revision, line })
            }
            Request::Threads { .. }
            | Request::CommentTarget { .. }
            | Request::CreateThread { .. }
            | Request::Reply { .. }
            | Request::SetThreadStatus { .. }
            | Request::DeleteThread { .. }
            | Request::EditMessage { .. }
            | Request::RemoveMessage { .. }
            | Request::KeepEdit { .. }
            | Request::SyncThreads { .. }
            | Request::SubmitReview { .. }
            | Request::PostDrafts { .. }
            | Request::SetVote { .. }
            | Request::PullRequestInfo { .. }
            | Request::ConfirmAnchor { .. }
            | Request::AttachAnchor { .. } => Err("the corpus has no threads".into()),
            Request::Hover {
                project,
                revision,
                side,
                path,
                line,
                col,
            } => {
                current(project, revision)?;
                position(&path)?;
                Ok(Response::Hover {
                    revision,
                    side,
                    text: Some(format!(
                        "{path}:{}:{}\n\nmock hover (derived: reachable from 2 entry points)",
                        line + 1,
                        col + 1
                    )),
                    code_highlights: Vec::new(),
                    unavailable: None,
                })
            }
            Request::Definition { .. }
            | Request::References { .. }
            | Request::Diagnostics { .. }
            | Request::ChangedSymbols { .. }
            | Request::Signature { .. }
            | Request::Impact { .. }
            | Request::RemovalImpact { .. }
            | Request::Reachability { .. }
            | Request::ListServers { .. }
            | Request::InstallServer { .. }
            | Request::SetServerPath { .. }
            | Request::ClearSemanticCaches => Err("the corpus has no language servers".into()),
            Request::LogDirectory => Err("the corpus keeps no log".into()),
            Request::ListChangesets { .. }
            | Request::ListPullRequests { .. }
            | Request::OpenChangeset { .. }
            | Request::Checks { .. }
            | Request::CheckLog { .. } => Err("the corpus is one changeset".into()),
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    if flag("--exit-on-stdin-close") {
        exit_when_stdin_closes();
    }
    let _log = vet_log::data_dir().map(|dir| vet_log::init(&dir.join(vet_log::LOGS)));
    let config = Config::from_args(&args);
    let fail = |error: String| -> ! {
        tracing::error!("cannot load the corpus: {error}");
        std::process::exit(2);
    };
    if flag("--prepare") {
        corpus::ensure_repo(&config).unwrap_or_else(|e| fail(e));
        return Ok(());
    }
    let corpus = corpus::load(&config).unwrap_or_else(|e| fail(e));

    let stats = corpus.stats();
    tracing::info!(
        "corpus {}: {} files, {} hunks, biggest file {} ({} lines)",
        corpus.name,
        stats.files,
        stats.hunks,
        stats.biggest.0,
        stats.biggest.1
    );
    tracing::info!("hunks are counted as zero-context regions (like git diff -U0)");
    if flag("--stats") {
        let problems = stats.problems();
        for problem in &problems {
            tracing::warn!("below the spike's corpus thresholds: {problem}");
        }
        std::process::exit(i32::from(!problems.is_empty()));
    }

    let mock = Mock::new(corpus);
    let server = Server::bind().await?;
    announce(&server)?;
    server.serve(mock).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use vet_core::completion::HunkState;
    use vet_core::hunks::line_hunks;

    fn mock() -> Mock {
        let base: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let target = base.replacen("line 3\n", "changed 3\n", 1).replacen("line 30\n", "", 1);
        let hunks = line_hunks(base.as_bytes(), target.as_bytes());
        assert_eq!(hunks.len(), 2);
        Mock::new(Corpus {
            name: "test".into(),
            files: vec![corpus::File {
                path: "a.txt".into(),
                base,
                target,
                hunks,
            }],
        })
    }

    #[test]
    fn a_hunk_marked_seen_counts_as_seen_on_both_sides() {
        let mock = mock();
        let spans = mock.seen_spans(&[vec![false, true]]);
        let coverage = Coverage::new(&spans);
        let diff = &mock.diffs[0];
        assert_eq!(coverage.hunk_states(diff), [HunkState::Unseen, HunkState::Seen]);
        let completion = Mock::completion(&coverage, &mock.diffs);
        // The first hunk changes one line on each side, the second removes one.
        assert_eq!((completion.lines, completion.seen_lines), (3, 1));
    }
}
