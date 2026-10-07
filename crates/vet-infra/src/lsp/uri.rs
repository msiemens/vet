//! `file://` URIs of paths and back: the one mapping every part of the client uses.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

/// The URI of the absolute path `path`.
pub(super) fn file_uri(path: &Path) -> Option<Uri> {
    Uri::from_str(url::Url::from_file_path(path).ok()?.as_str()).ok()
}

/// The URI of the absolute directory `dir`, which ends in a slash.
pub(super) fn directory_uri(dir: &Path) -> Option<Uri> {
    Uri::from_str(url::Url::from_directory_path(dir).ok()?.as_str()).ok()
}

/// The path a `file://` URI names.
pub(super) fn path_of(uri: &Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_round_trips_through_its_uri() {
        let path = std::env::temp_dir().join("a b").join("é.rs");
        let uri = file_uri(&path).unwrap();
        assert!(
            uri.as_str().starts_with("file://") && !uri.as_str().contains(' '),
            "{}",
            uri.as_str()
        );
        assert_eq!(path_of(&uri), Some(path));
    }

    #[test]
    fn a_directory_uri_ends_in_a_slash_and_a_relative_path_has_none() {
        assert!(directory_uri(&std::env::temp_dir()).unwrap().as_str().ends_with('/'));
        assert!(file_uri(Path::new("relative/a.rs")).is_none());
        assert!(path_of(&Uri::from_str("https://example.com/a.rs").unwrap()).is_none());
    }
}
