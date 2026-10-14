//! Which changed files the background analysis (symbols, problems) reads, and in what order.

use vet_app::session::{FileView, View};

use crate::Open;
use crate::open::Booted;

/// The changed files of the changeset on view that `keep` accepts, as two batches: the files the
/// viewer has open, in the order the window shows them, then the rest in the changeset's order.
/// The open files are read first.
pub(crate) async fn changed_files(
    open: &Open,
    booted: &Booted,
    keep: fn(&FileView) -> bool,
) -> (Vec<String>, Vec<String>) {
    let Ok(manifest) = booted.session.manifest(View::Changeset).await else {
        return (Vec::new(), Vec::new());
    };
    let paths: Vec<String> = manifest
        .files
        .iter()
        .filter(|file| keep(file))
        .map(|file| file.path.to_string_lossy())
        .collect();
    open_first(paths, &open.open_files())
}

/// `paths` as the open ones and the rest: those in `first` (each once, in `first`'s order) that the
/// changeset has, then the others in their order.
fn open_first(paths: Vec<String>, first: &[String]) -> (Vec<String>, Vec<String>) {
    let mut opened: Vec<String> = Vec::new();
    for path in first {
        if paths.contains(path) && !opened.contains(path) {
            opened.push(path.clone());
        }
    }
    let rest = paths.into_iter().filter(|path| !opened.contains(path)).collect();
    (opened, rest)
}

#[cfg(test)]
mod tests {
    use super::open_first;

    #[test]
    fn the_open_files_come_first_in_their_order_and_the_rest_keep_theirs() {
        let paths = vec![
            "a.rs".to_owned(),
            "b.rs".to_owned(),
            "c.rs".to_owned(),
            "d.rs".to_owned(),
        ];
        let (opened, rest) = open_first(paths, &["c.rs".to_owned(), "a.rs".to_owned()]);
        assert_eq!(opened, ["c.rs", "a.rs"]);
        assert_eq!(rest, ["b.rs", "d.rs"]);
    }

    #[test]
    fn an_open_file_not_in_the_changeset_is_ignored() {
        let paths = vec!["a.rs".to_owned(), "b.rs".to_owned()];
        let (opened, rest) = open_first(paths, &["gone.rs".to_owned(), "b.rs".to_owned()]);
        assert_eq!(opened, ["b.rs"]);
        assert_eq!(rest, ["a.rs"]);
    }
}
