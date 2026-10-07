//! Conversions between `RepoPath` bytes (git's: `/`-separated, relative to the
//! repository root) and filesystem paths. A name the platform cannot represent
//! (non-UTF-8 on Windows) is an error: nothing is replaced or dropped.

use std::fmt;
use std::path::{Component, Path, Prefix};

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

/// The part of `path` below `root`, both absolute. A Windows drive letter
/// matches in either case and with or without the `\\?\` prefix: language
/// servers report paths in their own spelling (`c:` for `C:`).
pub fn below<'a>(root: &Path, path: &'a Path) -> Option<&'a Path> {
    let mut rest = path.components();
    for want in root.components() {
        if !same_component(want, rest.next()?) {
            return None;
        }
    }
    Some(rest.as_path())
}

fn same_component(a: Component, b: Component) -> bool {
    let disk = |prefix: Prefix| match prefix {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => Some(letter.to_ascii_uppercase()),
        _ => None,
    };
    match (a, b) {
        (Component::Prefix(a), Component::Prefix(b)) => match (disk(a.kind()), disk(b.kind())) {
            (Some(a), Some(b)) => a == b,
            _ => a == b,
        },
        _ => a == b,
    }
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
    fn below_matches_whole_components() {
        let root = std::env::temp_dir().join("repo");
        let file = root.join("src").join("lib.rs");
        assert_eq!(below(&root, &file), Some(Path::new("src").join("lib.rs").as_path()));
        assert_eq!(below(&root, &root), Some(Path::new("")));
        assert_eq!(below(&root, &std::env::temp_dir().join("repo2").join("lib.rs")), None);
        assert_eq!(below(&root, &std::env::temp_dir()), None);
    }

    #[cfg(windows)]
    #[test]
    fn below_ignores_the_case_and_verbatim_form_of_a_drive_letter() {
        let file = Path::new(r"c:\work\repo\src\lib.rs");
        let expected = Some(Path::new(r"src\lib.rs"));
        assert_eq!(below(Path::new(r"C:\work\repo"), file), expected);
        assert_eq!(below(Path::new(r"\\?\C:\work\repo"), file), expected);
        assert_eq!(below(Path::new(r"D:\work\repo"), file), None);
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
