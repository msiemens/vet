//! The order the background analysis of a changeset's changed files reads them: the ones the
//! window has open first, then the rest in the changeset's order.

/// `paths` as the open ones and the rest: those in `open` (each once, in `open`'s order) that are
/// among `paths`, then the others in their order.
pub fn open_first(paths: Vec<String>, open: &[String]) -> (Vec<String>, Vec<String>) {
    let mut opened: Vec<String> = Vec::new();
    for path in open {
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

    #[test]
    fn an_open_file_named_twice_is_analysed_once() {
        let paths = vec!["a.rs".to_owned(), "b.rs".to_owned()];
        let (opened, rest) = open_first(paths, &["b.rs".to_owned(), "b.rs".to_owned()]);
        assert_eq!(opened, ["b.rs"]);
        assert_eq!(rest, ["a.rs"]);
    }
}
