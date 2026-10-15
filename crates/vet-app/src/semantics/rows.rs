//! The places of a semantic answer as rows to list (§7.3): each with its line, and where it
//! stands against the changeset on a side. What a row shows is decided here; the protocol
//! answer maps it one to one (`vet_mapping::semantic::location_rows`).

use std::collections::HashMap;

use vet_core::content::{FileDiff, LineRange, RepoPath};
use vet_core::num::sat_u32;
use vet_core::review::Side;
use vet_core::text::Lines;
use vet_semantic::derived::Reference;
use vet_semantic::ports::{At, Materializer, ServerLauncher, SymbolInfo};

use crate::ports::{GitRepo, LineSources, LiveStore};
use crate::session::RevisionView;

use super::{Answer, Fallible, Semantics};

/// The most rows an answer lists.
pub const MAX_ROWS: usize = 200;
/// The most UTF-16 units of a line a row shows.
const MAX_ROW_TEXT: u32 = 300;

/// The largest file outside the repository whose lines are shown.
const MAX_OUTSIDE_FILE: u64 = 8 * 1024 * 1024;

/// Where a place stands against the changeset on a side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Standing {
    /// The changeset does not change the file the place is in.
    Unchanged,
    /// The file is changed; the place is outside every hunk.
    File,
    /// The place is in hunk `u32` of its file's diff.
    Hunk(u32),
}

/// A place an answer lists, as a row shows it: its one-based place and the trimmed line it
/// stands in, cut to `MAX_ROW_TEXT` units, with the match inside it and its highlighting.
#[derive(Clone, Debug)]
pub struct Row {
    pub path: RepoPath,
    /// The place, 1-based.
    pub line: u32,
    pub col: u32,
    /// The symbol the place is in, if the server named one.
    pub within: Option<SymbolInfo>,
    /// The spans of the shown text, as the file's highlighting cut to its line has them.
    pub highlights: Vec<u32>,
    pub text: String,
    /// Where the match the row is shown for starts and ends, in units of `text`.
    pub match_start: u32,
    pub match_end: u32,
    pub change: Standing,
    /// The place is outside the repository, where no diff says where it stands.
    pub external: bool,
}

/// The rows an answer about places comes in as, cut to the limit asked for: with how many
/// there were whatever the limit kept.
#[derive(Clone, Debug)]
pub struct Rows {
    pub revision: u32,
    pub rows: Vec<Row>,
    pub total: u32,
}

impl<G, S, L, M> Semantics<G, S, L, M>
where
    G: GitRepo + Send + Sync + 'static,
    S: LiveStore + Send + Sync + 'static,
    L: ServerLauncher,
    M: Materializer,
{
    /// The places of `answer` as rows to list, at most `limit` of them: with their line, and
    /// where they stand against the changeset on `side`. A file's text is read (and highlighted)
    /// once however many of its lines the answer names, and a place outside the repository is
    /// read from the [`LineSources`] only where it is a source of the language that answered.
    pub async fn rows(
        &self,
        view: &RevisionView<G, S>,
        side: Side,
        answer: &Answer<Vec<Reference>>,
        at: &At,
        limit: Option<u32>,
        sources: &impl LineSources,
    ) -> Fallible<Rows> {
        let revision = answer.revision;
        let Ok(found) = &answer.result else {
            return Ok(Rows {
                revision,
                rows: Vec::new(),
                total: 0,
            });
        };
        let answerer = self.language_of(&at.file).map(|language| language.0.as_str());
        let limit = limit.map_or(MAX_ROWS, |l| l as usize);
        let files = view.changed_files(revision).await?;
        let mut texts: HashMap<RepoPath, Option<LineText>> = HashMap::new();
        let mut rows = Vec::with_capacity(found.len());
        for r in found {
            let file = &r.location.file;
            if !texts.contains_key(file) {
                let text = match r.location.external {
                    false => view.side_text(revision, side, file).await?,
                    true => outside(sources, answerer, file).await,
                };
                let lines = match text {
                    Some(text) => {
                        let highlights = sources.highlights(&file.to_string_lossy(), &text).await;
                        Some(LineText::split(text, highlights))
                    }
                    None => None,
                };
                texts.insert(file.clone(), lines);
            }
            let line = r.location.span.start.line;
            let source = texts[file].as_ref();
            let text = source.map_or("", |s| s.line(line));
            let standing = match r.location.external {
                // No diff says where an outside place stands.
                true => Standing::Unchanged,
                false => standing(&files, side, file, line),
            };
            rows.push(row(r, text, standing, |from, to| {
                source.map_or_else(Vec::new, |s| {
                    let start = s.start(line);
                    sources.cut(&s.highlights, start + from, start + to)
                })
            }));
        }
        let (rows, total) = finish(rows, limit);
        Ok(Rows { revision, rows, total })
    }
}

/// The text of `file` outside the repository, which the server of `language` named: read only
/// where the file is one of the language's own sources (a project file is not one), and never
/// one too large to show.
async fn outside<S: LineSources>(sources: &S, language: Option<&str>, file: &RepoPath) -> Option<String> {
    let language = language?;
    if !sources.serves(language, file) {
        return None;
    }
    sources.outside(file, MAX_OUTSIDE_FILE).await
}

/// The text of a file that rows list lines out of, split once ([`Lines::new`]): with the offset
/// of each line's start in bytes, to take a line's text from, and in units, to place its
/// highlighting.
struct LineText {
    text: String,
    bytes: Vec<usize>,
    units: Vec<u32>,
    /// The highlighting of the whole text.
    highlights: Vec<u32>,
}

impl LineText {
    fn split(text: String, highlights: Vec<u32>) -> Self {
        let lines = Lines::new(&text);
        let bytes = lines.byte_starts().to_vec();
        let units = lines.starts().to_vec();
        Self {
            text,
            bytes,
            units,
            highlights,
        }
    }

    /// The text of `line` (0-based), without its line ending; `""` past the end.
    fn line(&self, line: u32) -> &str {
        let (Some(at), Some(to)) = (self.bytes.get(line as usize), self.bytes.get(line as usize + 1)) else {
            return "";
        };
        let line = &self.text[*at..*to];
        let line = line.strip_suffix('\n').unwrap_or(line);
        line.strip_suffix('\r').unwrap_or(line)
    }

    /// The offset of `line`'s start in units; the text's end past it.
    fn start(&self, line: u32) -> u32 {
        self.units
            .get(line as usize)
            .copied()
            .unwrap_or_else(|| self.units.last().copied().unwrap_or(0))
    }
}

fn utf16_len(s: &str) -> u32 {
    sat_u32(s.encode_utf16().count())
}

/// Where a place on `side`, at `line` of `file`, stands against the changeset: in a changed
/// file's hunk, in a changed file, or not in the changeset's files at all.
fn standing(files: &[FileDiff], side: Side, file: &RepoPath, line: u32) -> Standing {
    let on_side = |f: &&FileDiff| match side {
        Side::Base => f.old_path.as_ref() == Some(file),
        Side::Target => f.new_path.as_ref() == Some(file),
    };
    let Some(diff) = files.iter().find(on_side) else {
        return Standing::Unchanged;
    };
    let at = LineRange {
        start: line,
        end: line + 1,
    };
    let hunk = diff.hunks.iter().position(|h| {
        at.intersects(match side {
            Side::Base => h.old,
            Side::Target => h.new,
        })
    });
    match hunk {
        Some(i) => Standing::Hunk(sat_u32(i)),
        None => Standing::File,
    }
}

/// A row for the reference `r`, whose line's text `line` is (the whole line, if it could be
/// read). `change` says where it stands. `highlights` gives the highlighting of a range of the
/// line, in units from its start.
fn row(r: &Reference, line_text: &str, change: Standing, highlights: impl FnOnce(u32, u32) -> Vec<u32>) -> Row {
    let span = r.location.span;
    let trimmed = line_text.trim_start();
    let lead = utf16_len(&line_text[..line_text.len() - trimmed.len()]);
    let mut shown = 0;
    let text: String = trimmed
        .trim_end()
        .chars()
        .take_while(|c| {
            shown += c.len_utf16() as u32;
            shown <= MAX_ROW_TEXT
        })
        .collect();
    let len = utf16_len(&text);
    let match_start = span.start.col.saturating_sub(lead).min(len);
    let mut match_end = if span.end.line == span.start.line {
        span.end.col.saturating_sub(lead).clamp(match_start, len)
    } else {
        len
    };
    if match_end == match_start {
        // A server may point at a name without giving its extent.
        let word = text
            .encode_utf16()
            .skip(match_start as usize)
            .take_while(|&u| char::from_u32(u.into()).is_some_and(|c| c.is_alphanumeric() || c == '_'))
            .count();
        match_end += sat_u32(word);
    }
    Row {
        path: r.location.file.clone(),
        line: span.start.line + 1,
        col: span.start.col + 1,
        within: r.within.clone(),
        highlights: highlights(lead, lead + len),
        text,
        match_start,
        match_end,
        change,
        external: r.location.external,
    }
}

/// The rows by path and line, those in the repository before the others, cut to `limit`
/// ([`MAX_ROWS`] by default); with how many there were. A cut keeps the places in changed
/// hunks, then those in changed files, before the others.
fn finish(mut rows: Vec<Row>, limit: usize) -> (Vec<Row>, u32) {
    let total = sat_u32(rows.len());
    if rows.len() > limit {
        let rank = |r: &Row| match r.change {
            Standing::Hunk(_) => 0,
            Standing::File => 1,
            Standing::Unchanged => 2,
        };
        rows.sort_by_key(|r| (rank(r), r.external));
        rows.truncate(limit);
    }
    rows.sort_by(|a, b| (a.external, &a.path, a.line, a.col).cmp(&(b.external, &b.path, b.line, b.col)));
    (rows, total)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use vet_core::content::{Hunk, LineRange};
    use vet_semantic::ports::{Location, Pos, Span};

    use super::*;

    fn reference(file: &str, from: (u32, u32), to: (u32, u32), external: bool) -> Reference {
        Reference {
            location: Location {
                file: RepoPath::from(file),
                span: Span {
                    start: Pos {
                        line: from.0,
                        col: from.1,
                    },
                    end: Pos { line: to.0, col: to.1 },
                },
                external,
            },
            within: None,
        }
    }

    /// A row of an unhighlighted line.
    fn plain_row(r: &Reference, line: &str) -> Row {
        row(r, line, Standing::Unchanged, |_, _| Vec::new())
    }

    fn diff(path: &str, hunks: &[(u32, u32, u32, u32)]) -> FileDiff {
        FileDiff {
            old_path: Some(RepoPath::from(path)),
            new_path: Some(RepoPath::from(path)),
            old_blob: None,
            new_blob: None,
            hunks: hunks
                .iter()
                .map(|&(os, oe, ns, ne)| Hunk {
                    old: LineRange { start: os, end: oe },
                    new: LineRange { start: ns, end: ne },
                })
                .collect(),
        }
    }

    /// The sources a row list asks the outside questions of: whether they let a place's file be
    /// read, and what they were asked to.
    struct Allowed(bool, Mutex<usize>);

    impl Allowed {
        /// How many files were read.
        fn reads(&self) -> std::sync::MutexGuard<'_, usize> {
            self.1.lock().expect("not poisoned")
        }
    }

    impl LineSources for Allowed {
        fn serves(&self, _language: &str, _file: &RepoPath) -> bool {
            self.0
        }

        async fn outside(&self, _file: &RepoPath, max_bytes: u64) -> Option<String> {
            assert_eq!(max_bytes, MAX_OUTSIDE_FILE, "the size the use case allows");
            *self.1.lock().unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            Some("the file's text".into())
        }

        async fn highlights(&self, _path: &str, _text: &str) -> Vec<u32> {
            Vec::new()
        }

        fn cut(&self, _highlights: &[u32], _from: u32, _to: u32) -> Vec<u32> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn a_place_outside_the_repository_is_read_only_for_a_source_of_the_answerers_language() {
        let file = RepoPath::from("/dep/lib.rs");
        let allowed = Allowed(false, Mutex::default());
        // No answerer: nothing is read.
        assert_eq!(outside(&allowed, None, &file).await, None);
        // A file that is not one of the answerer's sources: nothing is read.
        assert_eq!(outside(&allowed, Some("rust"), &file).await, None);
        assert_eq!(*allowed.reads(), 0);
        // A source of the answerer's: read once, within the size the answer shows.
        let sources = Allowed(true, Mutex::default());
        assert_eq!(
            outside(&sources, Some("rust"), &file).await.as_deref(),
            Some("the file's text")
        );
        assert_eq!(*sources.reads(), 1);
    }
    #[test]
    fn a_row_shows_the_trimmed_line_with_the_match_inside_it() {
        let r = reference("a.cs", (3, 12), (3, 15), false);
        let row = plain_row(&r, "        x = Foo();  ");
        assert_eq!((row.line, row.col), (4, 13));
        assert_eq!(row.text, "x = Foo();");
        assert_eq!(&row.text[row.match_start as usize..row.match_end as usize], "Foo");
    }

    #[test]
    fn a_rows_highlights_are_asked_for_the_trimmed_text_in_units_of_its_line() {
        let r = reference("a.rs", (0, 4), (0, 5), false);
        let asked = std::cell::Cell::new((0, 0));
        // Three units of indentation, then "é😀 f" (five units) and trailing blanks.
        row(&r, "   é😀 f  ", Standing::Unchanged, |from, to| {
            asked.set((from, to));
            vec![1, 2, 3]
        });
        assert_eq!(asked.get(), (3, 3 + 5));
    }

    #[test]
    fn a_lines_offsets_take_its_text_and_place_its_highlighting() {
        let text = LineText::split("first\r\nsecond line\n".into(), Vec::new());
        assert_eq!(text.start(0), 0);
        // The `\r\n` counts as its line's last unit.
        assert_eq!(text.start(1), 6);
        // The last offset is the text's end, whatever line beyond it asks for it.
        assert_eq!((text.start(2), text.start(9)), (18, 18));
        assert_eq!(text.line(0), "first");
        assert_eq!(text.line(1), "second line");
        assert_eq!(text.line(2), "");
    }

    #[test]
    fn a_place_without_extent_marks_the_name_it_points_at() {
        let r = reference("Program.cs", (9, 15), (9, 15), false);
        let row = plain_row(&r, "    public int Area() => 4;");
        assert_eq!(&row.text[row.match_start as usize..row.match_end as usize], "Area");
    }

    #[test]
    fn match_columns_count_utf16_units_not_bytes() {
        // "é" is two bytes, the emoji one scalar of two UTF-16 units.
        let r = reference("a.rs", (0, 4), (0, 7), false);
        let row = plain_row(&r, "é😀 f(x)");
        let units: Vec<u16> = row.text.encode_utf16().collect();
        let matched = String::from_utf16(&units[row.match_start as usize..row.match_end as usize]).unwrap();
        assert_eq!(matched, "f(x");
    }

    #[test]
    fn a_match_over_several_lines_ends_with_the_first_line() {
        let r = reference("a.rs", (1, 4), (3, 1), false);
        let row = plain_row(&r, "    fn run() {");
        assert_eq!(
            &row.text[row.match_start as usize..row.match_end as usize],
            "fn run() {"
        );
    }

    #[test]
    fn a_long_line_is_cut_and_a_match_beyond_the_cut_is_clamped_to_its_end() {
        let line = "x".repeat(1000);
        let matched = |from, to| {
            let row = plain_row(&reference("a.rs", (0, from), (0, to), false), &line);
            assert_eq!(row.text.len(), 300);
            (row.match_start, row.match_end)
        };
        assert_eq!(matched(100, 110), (100, 110));
        assert_eq!(matched(295, 310), (295, 300));
        assert_eq!(matched(900, 910), (300, 300));
    }

    #[test]
    fn a_line_that_could_not_be_read_leaves_an_empty_text() {
        let r = reference("/dep/lib.rs", (7, 2), (7, 5), true);
        let row = plain_row(&r, "");
        assert_eq!((row.text.as_str(), row.match_start, row.match_end), ("", 0, 0));
        assert!(row.external);
    }

    #[test]
    fn the_line_of_a_text_is_found_whatever_its_line_ending() {
        let text = LineText::split("a\r\nb\nc".into(), Vec::new());
        assert_eq!(text.line(1), "b");
        assert_eq!(text.line(0), "a");
        assert_eq!(text.line(5), "");
    }

    #[test]
    fn a_place_is_in_a_hunk_in_a_changed_file_or_unchanged() {
        let files = [diff("a.rs", &[(10, 12, 10, 15)])];
        let a = RepoPath::from("a.rs");
        assert_eq!(standing(&files, Side::Target, &a, 14), Standing::Hunk(0));
        assert_eq!(standing(&files, Side::Target, &a, 15), Standing::File);
        assert_eq!(standing(&files, Side::Base, &a, 12), Standing::File);
        assert_eq!(standing(&files, Side::Base, &a, 11), Standing::Hunk(0));
        assert_eq!(
            standing(&files, Side::Target, &RepoPath::from("b.rs"), 14),
            Standing::Unchanged
        );
    }

    #[test]
    fn a_place_in_a_hunk_names_the_hunk_by_its_index_in_the_file() {
        let files = [diff("a.rs", &[(1, 2, 1, 2), (10, 12, 10, 15), (30, 31, 33, 34)])];
        let a = RepoPath::from("a.rs");
        assert_eq!(standing(&files, Side::Target, &a, 11), Standing::Hunk(1));
        assert_eq!(standing(&files, Side::Target, &a, 33), Standing::Hunk(2));
        assert_eq!(standing(&files, Side::Base, &a, 30), Standing::Hunk(2));
    }

    #[test]
    fn a_renamed_file_is_changed_under_the_name_of_the_side() {
        let mut renamed = diff("new.rs", &[(0, 1, 0, 1)]);
        renamed.old_path = Some(RepoPath::from("old.rs"));
        let files = [renamed];
        assert_eq!(
            standing(&files, Side::Base, &RepoPath::from("old.rs"), 0),
            Standing::Hunk(0)
        );
        assert_eq!(
            standing(&files, Side::Target, &RepoPath::from("old.rs"), 0),
            Standing::Unchanged
        );
    }

    #[test]
    fn rows_are_ordered_by_path_and_line_with_outside_places_last_and_cut_with_the_total_kept() {
        let at =
            |path: &str, line: u32, external: bool| plain_row(&reference(path, (line, 0), (line, 1), external), "");
        let rows = vec![
            at("/dep/a.rs", 1, true),
            at("src/b.rs", 9, false),
            at("src/a.rs", 20, false),
            at("src/a.rs", 3, false),
        ];
        let (sorted, total) = finish(rows, MAX_ROWS);
        let order: Vec<_> = sorted
            .iter()
            .map(|r| (r.path.to_string_lossy().to_owned(), r.line))
            .collect();
        assert_eq!(
            order,
            [
                ("src/a.rs".to_owned(), 4),
                ("src/a.rs".to_owned(), 21),
                ("src/b.rs".to_owned(), 10),
                ("/dep/a.rs".to_owned(), 2)
            ]
        );
        assert_eq!(total, 4);

        let many = (0..MAX_ROWS as u32 + 7)
            .map(|n| {
                let mut r = at("src/a.rs", n, false);
                if n >= MAX_ROWS as u32 {
                    r.change = Standing::Hunk(0);
                }
                r
            })
            .collect();
        let (cut, total) = finish(many, MAX_ROWS);
        assert_eq!((cut.len(), total as usize), (MAX_ROWS, MAX_ROWS + 7));
        let hunks = cut.iter().filter(|r| matches!(r.change, Standing::Hunk(_))).count();
        assert_eq!(hunks, 7, "a cut keeps the places in changed hunks");
        assert!(cut.windows(2).all(|w| w[0].line < w[1].line));
    }

    #[test]
    fn a_larger_limit_lists_every_row_and_a_smaller_one_cuts_sooner() {
        let rows = |n: u32| -> Vec<_> {
            (0..n)
                .map(|n| plain_row(&reference("src/a.rs", (n, 0), (n, 1), false), ""))
                .collect()
        };
        let n = MAX_ROWS as u32 + 50;
        let (all, total) = finish(rows(n), n as usize);
        assert_eq!((all.len(), total), (n as usize, n));
        let (few, total) = finish(rows(n), 10);
        assert_eq!((few.len(), total), (10, n));
        let (few, total) = finish(rows(20), 10);
        assert_eq!(
            (few.len(), total),
            (10, 20),
            "a limit below the default cuts a short list too"
        );
    }
}
