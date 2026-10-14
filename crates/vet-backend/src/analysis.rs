//! Which changed files the background analysis (symbols, problems) reads, and in what order.

use vet_app::session::{FileView, View};

use crate::Open;
use crate::open::Booted;

/// The changed files of the changeset on view that `keep` accepts, the files the viewer has open
/// first, then the rest in the order the changeset lists them.
pub(crate) async fn changed_files(open: &Open, booted: &Booted, keep: fn(&FileView) -> bool) -> Vec<String> {
    let Ok(manifest) = booted.session.manifest(View::Changeset).await else {
        return Vec::new();
    };
    let paths: Vec<String> = manifest
        .files
        .iter()
        .filter(|file| keep(file))
        .map(|file| file.path.to_string_lossy())
        .collect();
    open_first(paths, &open.open_files())
}

/// `paths`, with those in `first` moved to the front in `first`'s order; the rest keep their order.
fn open_first(paths: Vec<String>, first: &[String]) -> Vec<String> {
    let mut ordered: Vec<String> = Vec::with_capacity(paths.len());
    for open in first {
        if paths.contains(open) && !ordered.contains(open) {
            ordered.push(open.clone());
        }
    }
    for path in paths {
        if !ordered.contains(&path) {
            ordered.push(path);
        }
    }
    ordered
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
        let ordered = open_first(paths, &["c.rs".to_owned(), "a.rs".to_owned()]);
        assert_eq!(ordered, ["c.rs", "a.rs", "b.rs", "d.rs"]);
    }

    #[test]
    fn an_open_file_not_in_the_changeset_is_ignored() {
        let paths = vec!["a.rs".to_owned(), "b.rs".to_owned()];
        let ordered = open_first(paths, &["gone.rs".to_owned(), "b.rs".to_owned()]);
        assert_eq!(ordered, ["b.rs", "a.rs"]);
    }
}
