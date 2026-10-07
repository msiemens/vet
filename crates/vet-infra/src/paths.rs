//! Conversions between `RepoPath` bytes (git's: `/`-separated, relative to the
//! repository root) and filesystem paths. A name the platform cannot represent
//! (non-UTF-8 on Windows) is an error: nothing is replaced or dropped.

use std::fmt;
use std::path::{Component, Path};

use vet_core::content::RepoPath;

#[derive(Debug, PartialEq, Eq)]
pub struct PathError(String);

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PathError {}

fn unrepresentable(name: &[u8]) -> PathError {
    PathError(format!(
        "the file name {:?} cannot be represented on this platform",
        String::from_utf8_lossy(name)
    ))
}

/// The filesystem path of `path`, a `RepoPath`'s bytes, below the root.
pub fn to_path(path: &[u8]) -> Result<&Path, PathError> {
    gix::path::try_from_byte_slice(path).map_err(|_| unrepresentable(path))
}

/// The `RepoPath` of `relative`, a path below the root.
pub fn from_relative(relative: &Path) -> Result<RepoPath, PathError> {
    let mut parts: Vec<&[u8]> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                let bytes = gix::path::os_str_into_bstr(name)
                    .map_err(|_| PathError(format!("the file name {name:?} cannot be represented here")))?;
                parts.push(bytes);
            }
            Component::CurDir => {}
            _ => return Err(PathError(format!("{} is not below the root", relative.display()))),
        }
    }
    Ok(RepoPath(parts.join(&b'/')))
}

/// An absolute path in the `/`-separated form `RepoPath` uses for places outside the root.
pub fn from_absolute(path: &Path) -> Result<RepoPath, PathError> {
    let bytes = gix::path::try_into_bstr(path)
        .map_err(|_| PathError(format!("the path {} cannot be represented here", path.display())))?;
    Ok(RepoPath(
        gix::path::to_unix_separators_on_windows(bytes).into_owned().into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_round_trips_through_repo_bytes() {
        let path = from_relative(Path::new("src/./a b/lib.rs")).unwrap();
        assert_eq!(path, RepoPath(b"src/a b/lib.rs".to_vec()));
        assert_eq!(to_path(&path.0).unwrap(), Path::new("src").join("a b").join("lib.rs"));
        assert_eq!(from_relative(Path::new("")).unwrap(), RepoPath(Vec::new()));
    }

    #[test]
    fn a_path_outside_the_root_is_refused() {
        assert!(from_relative(Path::new("../x")).is_err());
        assert!(from_relative(Path::new("/x")).is_err());
    }

    #[test]
    fn an_absolute_path_keeps_its_separators_as_slashes() {
        let absolute = std::env::temp_dir().join("a").join("b");
        let path = from_absolute(&absolute).unwrap();
        assert!(path.0.ends_with(b"/a/b"), "{path:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_utf8_survives_on_unix() {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::OsStr::from_bytes(b"d\xff/f");
        let path = from_relative(Path::new(name)).unwrap();
        assert_eq!(path.0, b"d\xff/f");
        assert_eq!(to_path(&path.0).unwrap(), Path::new(name));
    }
}
