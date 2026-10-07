//! `file://` URIs of paths and back: the one mapping every part of the client uses.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

/// The URI of the absolute path `path`.
pub(super) fn file_uri(path: &Path) -> Option<Uri> {
    uri_of(&url::Url::from_file_path(path).ok()?)
}

/// The URI of the absolute directory `dir`, which ends in a slash.
pub(super) fn directory_uri(dir: &Path) -> Option<Uri> {
    uri_of(&url::Url::from_directory_path(dir).ok()?)
}

/// `Url` leaves brackets in a path as they are, which `Uri` rejects (a route
/// directory like `[id]` is common).
fn uri_of(url: &url::Url) -> Option<Uri> {
    Uri::from_str(&url.as_str().replace('[', "%5B").replace(']', "%5D")).ok()
}

/// The path a `file://` URI names.
pub(super) fn path_of(uri: &Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

/// Identifies the file a URI names, however the sender spelled it. Servers
/// re-encode URIs (`(` as `%28`, `C:` as `c%3A`), so URI strings of one file
/// differ between what Vet sent and what a server publishes.
pub(super) fn key(uri: &Uri) -> String {
    match path_of(uri) {
        Some(path) if cfg!(windows) => path.to_string_lossy().to_lowercase(),
        Some(path) => path.to_string_lossy().into_owned(),
        None => uri.as_str().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_of_one_files_uri_share_a_key() {
        let sent = Uri::from_str("file:///w/my%20proj%20(1)/%5Bid%5D+x/%C3%A9@a.ts").unwrap();
        let published = Uri::from_str("file:///w/my%20proj%20%281%29/%5Bid%5D%2Bx/%C3%A9%40a.ts").unwrap();
        assert_ne!(sent.as_str(), published.as_str());
        assert_eq!(key(&sent), key(&published));
        let other = Uri::from_str("file:///w/my%20proj%20(1)/%5Bid%5D+x/b.ts").unwrap();
        assert_ne!(key(&sent), key(&other));
    }

    #[test]
    fn a_path_round_trips_through_its_uri() {
        let path = std::env::temp_dir().join("a b").join("[id]").join("é.rs");
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
