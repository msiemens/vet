//! The rows of one file's diff in the approval prompt: its hunks with a little context.

use vet_core::content::Hunk;

use super::check::Change;

/// Unchanged lines shown around a hunk.
pub const CONTEXT: u32 = 2;
/// A diff with more rows is cut.
pub const MAX_ROWS: usize = 400;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Row {
    /// An unchanged line; `number` is its 1-based line number in the new text.
    Context { number: u32, text: String },
    /// A line of the new text; `number` is 1-based.
    Added { number: u32, text: String },
    /// A line of the old text; `number` is 1-based.
    Removed { number: u32, text: String },
    /// Unchanged lines left out.
    Gap,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiffRows {
    pub rows: Vec<Row>,
    /// The diff had more than [`MAX_ROWS`] rows; the rest is left out.
    pub truncated: bool,
}

/// The rows of the diff from `old` to `new` made of `hunks`. `None` if a side that should have
/// text has none (binary, over the capture cap): nothing to compare. A removed file has no new
/// side and an added one no old side.
pub fn diff_rows(old: Option<&str>, new: Option<&str>, change: Change, hunks: &[Hunk]) -> Option<DiffRows> {
    if (old.is_none() && change != Change::Added) || (new.is_none() && change != Change::Removed) {
        return None;
    }
    let old: Vec<&str> = old.map_or_else(Vec::new, |t| t.lines().collect());
    let new: Vec<&str> = new.map_or_else(Vec::new, |t| t.lines().collect());
    let mut out = Builder::default();
    let context = |out: &mut Builder, from: u32, to: u32| {
        for i in from..to.min(new.len() as u32) {
            out.push(Row::Context {
                number: i + 1,
                text: new[i as usize].to_owned(),
            });
        }
    };

    let mut cursor = 0;
    for (index, hunk) in hunks.iter().enumerate() {
        let start = hunk.new.start;
        if index == 0 {
            if start > CONTEXT {
                out.push(Row::Gap);
            }
            context(&mut out, start.saturating_sub(CONTEXT), start);
        } else if start.saturating_sub(cursor) <= 2 * CONTEXT {
            context(&mut out, cursor, start);
        } else {
            context(&mut out, cursor, cursor + CONTEXT);
            out.push(Row::Gap);
            context(&mut out, start - CONTEXT, start);
        }
        for i in hunk.old.start..hunk.old.end {
            let text = old.get(i as usize).copied().unwrap_or_default();
            out.push(Row::Removed {
                number: i + 1,
                text: text.to_owned(),
            });
        }
        for i in hunk.new.start..hunk.new.end {
            let text = new.get(i as usize).copied().unwrap_or_default();
            out.push(Row::Added {
                number: i + 1,
                text: text.to_owned(),
            });
        }
        cursor = hunk.new.end;
        if out.full() {
            return Some(out.finish());
        }
    }
    if !hunks.is_empty() {
        let tail = (new.len() as u32).min(cursor + CONTEXT);
        context(&mut out, cursor, tail);
        if (tail as usize) < new.len() {
            out.push(Row::Gap);
        }
    }
    Some(out.finish())
}

/// Collects rows up to one past the limit, which is all it takes to know the diff is cut.
#[derive(Default)]
struct Builder {
    rows: Vec<Row>,
}

impl Builder {
    fn push(&mut self, row: Row) {
        if !self.full() {
            self.rows.push(row);
        }
    }

    fn full(&self) -> bool {
        self.rows.len() > MAX_ROWS
    }

    fn finish(mut self) -> DiffRows {
        let truncated = self.full();
        self.rows.truncate(MAX_ROWS);
        DiffRows {
            rows: self.rows,
            truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use vet_core::content::LineRange;

    use super::*;

    fn hunk(old: (u32, u32), new: (u32, u32)) -> Hunk {
        Hunk {
            old: LineRange {
                start: old.0,
                end: old.1,
            },
            new: LineRange {
                start: new.0,
                end: new.1,
            },
        }
    }

    fn text(n: u32) -> String {
        (1..=n).map(|i| format!("l{i}\n")).collect()
    }

    /// `c3` a context line numbered 3, `a`/`d` added and removed ones, `~` a gap.
    fn show(rows: &DiffRows) -> String {
        let parts: Vec<String> = rows
            .rows
            .iter()
            .map(|r| match r {
                Row::Context { number, .. } => format!("c{number}"),
                Row::Added { number, .. } => format!("a{number}"),
                Row::Removed { number, .. } => format!("d{number}"),
                Row::Gap => "~".into(),
            })
            .collect();
        parts.join(" ")
    }

    fn modified(old: &str, new: &str, hunks: &[Hunk]) -> DiffRows {
        diff_rows(Some(old), Some(new), Change::Modified, hunks).unwrap()
    }

    #[test]
    fn a_changed_line_shows_its_context_and_the_old_line() {
        let old = text(10);
        let new = old.replace("l5", "L5");
        let rows = modified(&old, &new, &[hunk((4, 5), (4, 5))]);
        assert_eq!(show(&rows), "~ c3 c4 d5 a5 c6 c7 ~");
        assert_eq!(
            rows.rows[3..5],
            [
                Row::Removed {
                    number: 5,
                    text: "l5".into()
                },
                Row::Added {
                    number: 5,
                    text: "L5".into()
                }
            ]
        );
        assert!(!rows.truncated);
    }

    #[test]
    fn a_change_at_the_edges_has_no_gap_there() {
        let rows = modified(&text(4), &text(4), &[hunk((0, 1), (0, 1)), hunk((3, 4), (3, 4))]);
        assert_eq!(show(&rows), "d1 a1 c2 c3 d4 a4");
    }

    #[test]
    fn hunks_close_together_share_their_context_and_distant_ones_are_separated() {
        let near = modified(&text(12), &text(12), &[hunk((1, 2), (1, 2)), hunk((5, 6), (5, 6))]);
        assert_eq!(show(&near), "c1 d2 a2 c3 c4 c5 d6 a6 c7 c8 ~");
        let far = modified(&text(30), &text(30), &[hunk((1, 2), (1, 2)), hunk((20, 21), (20, 21))]);
        assert_eq!(show(&far), "c1 d2 a2 c3 c4 ~ c19 c20 d21 a21 c22 c23 ~");
    }

    #[test]
    fn deletions_and_insertions_number_each_side_by_its_own_lines() {
        let old = text(8);
        let new = "l1\nl2\nl3\nnew\nl7\nl8\n";
        // l4..l6 replaced by one line.
        let rows = modified(&old, new, &[hunk((3, 6), (3, 4))]);
        assert_eq!(show(&rows), "~ c2 c3 d4 d5 d6 a4 c5 c6");
        // A pure deletion has no added row.
        let rows = modified(&old, "l1\nl2\nl3\nl6\nl7\nl8\n", &[hunk((3, 5), (3, 3))]);
        assert_eq!(show(&rows), "~ c2 c3 d4 d5 c4 c5 ~");
    }

    #[test]
    fn an_added_file_is_all_additions() {
        let rows = diff_rows(None, Some(&text(2)), Change::Added, &[hunk((0, 0), (0, 2))]).unwrap();
        assert_eq!(show(&rows), "a1 a2");
    }

    #[test]
    fn a_removed_file_is_all_removals() {
        let rows = diff_rows(Some(&text(2)), None, Change::Removed, &[hunk((0, 2), (0, 0))]).unwrap();
        assert_eq!(show(&rows), "d1 d2");
    }

    #[test]
    fn a_side_without_text_only_matters_for_a_modified_file() {
        assert_eq!(diff_rows(Some("x\n"), None, Change::Modified, &[]), None);
        assert_eq!(diff_rows(None, Some("x\n"), Change::Modified, &[]), None);
        let moved = diff_rows(Some("x\n"), Some("x\n"), Change::Added, &[]).unwrap();
        assert_eq!(
            moved,
            DiffRows {
                rows: vec![],
                truncated: false
            }
        );
    }

    #[test]
    fn long_diffs_are_cut_after_the_limit() {
        let n = MAX_ROWS as u32 + 50;
        let rows = diff_rows(None, Some(&text(n)), Change::Added, &[hunk((0, 0), (0, n))]).unwrap();
        assert_eq!(rows.rows.len(), MAX_ROWS);
        assert!(rows.truncated);
        assert_eq!(
            rows.rows.last(),
            Some(&Row::Added {
                number: MAX_ROWS as u32,
                text: format!("l{MAX_ROWS}")
            })
        );
    }

    #[test]
    fn exactly_the_limit_is_not_cut() {
        let n = MAX_ROWS as u32;
        let rows = diff_rows(None, Some(&text(n)), Change::Added, &[hunk((0, 0), (0, n))]).unwrap();
        assert_eq!((rows.rows.len(), rows.truncated), (MAX_ROWS, false));
    }

    #[test]
    fn crlf_line_ends_are_not_part_of_the_text() {
        let rows = modified("a\r\nb\r\n", "a\r\nB\r\n", &[hunk((1, 2), (1, 2))]);
        assert_eq!(
            rows.rows[2],
            Row::Added {
                number: 2,
                text: "B".into()
            }
        );
    }
}
