//! Conversions between `RepoPath` bytes (git's: `/`-separated, relative to the
//! repository root) and filesystem paths. A name the platform cannot represent
//! (non-UTF-8 on Windows) is an error: nothing is replaced or dropped. Also the
//! plain form of a canonical path on Windows, which is the one shown and handed on.

use std::path::{Component, Path, PathBuf, Prefix};
use std::{fmt, io};

use vet_core::content::{Hash, RepoPath};

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
        RepoPath(name.to_vec()).to_string_lossy()
    ))
}

/// The filesystem path of `path`, a `RepoPath`'s bytes, below the root.
pub fn to_path(path: &[u8]) -> Result<&Path, PathError> {
    gix::path::try_from_byte_slice(path).map_err(|_| unrepresentable(path))
}

/// Whether `path` is a file in the checkout at `root`.
pub fn is_file_in(root: &Path, path: &RepoPath) -> bool {
    to_path(&path.0).is_ok_and(|relative| root.join(relative).is_file())
}

/// Whether `path` ends in one of `extensions`, in any case.
pub(crate) fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The hash of the content of the file at `path`, if it can be read.
pub(crate) fn hash_of_file(path: &Path) -> Option<Hash> {
    std::fs::read(path).ok().map(|bytes| Hash::of(&bytes))
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

/// `path` with links resolved, in the plain form wherever that names the same file.
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    std::fs::canonicalize(path).map(simplified)
}

/// `path` without the verbatim prefix `canonicalize` puts on every path on Windows
/// (`\\?\C:\…`, `\\?\UNC\server\share\…`), which few programs accept and nobody wants
/// to read. A path the plain form cannot name stays verbatim: one too long for
/// `MAX_PATH`, or with a name Windows reads differently without it (`CON`, `a.`).
pub fn simplified(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    match path.to_str().and_then(plain_form) {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

/// `MAX_PATH`, which counts the terminating NUL.
const MAX_PATH: usize = 260;

/// The plain form of `path`, a Windows verbatim path, if it names the same file.
fn plain_form(path: &str) -> Option<String> {
    let rest = path.strip_prefix(r"\\?\")?;
    let (plain, names) = match rest.strip_prefix(r"UNC\") {
        Some(unc) => {
            let mut parts = unc.splitn(3, '\\');
            let (server, share) = (parts.next()?, parts.next()?);
            if !is_plain_name(server) || !is_plain_name(share) {
                return None;
            }
            (format!(r"\\{unc}"), parts.next().unwrap_or(""))
        }
        None => {
            let bytes = rest.as_bytes();
            if !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':') {
                return None;
            }
            let names = match &rest[2..] {
                "" => "",
                after => after.strip_prefix('\\')?,
            };
            (rest.to_owned(), names)
        }
    };
    let names = names.strip_suffix('\\').unwrap_or(names);
    let fits = plain.encode_utf16().count() < MAX_PATH;
    (fits && (names.is_empty() || names.split('\\').all(is_plain_name))).then_some(plain)
}

/// Whether Windows reads `name` as itself outside a verbatim path: it does not trim
/// trailing dots and spaces, take a reserved device name for the device, or split at `/`.
fn is_plain_name(name: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "AUX", "NUL", "PRN", "CON", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    !name.is_empty()
        && name.encode_utf16().count() <= 255
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        && !RESERVED.iter().any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    #[test]
    fn a_verbatim_drive_path_loses_its_prefix() {
        assert_eq!(plain_form(r"\\?\D:\work\my repo").as_deref(), Some(r"D:\work\my repo"));
        assert_eq!(plain_form(r"\\?\D:\work\").as_deref(), Some(r"D:\work\"));
        assert_eq!(plain_form(r"\\?\D:\").as_deref(), Some(r"D:\"));
    }

    #[test]
    fn a_verbatim_share_path_becomes_a_plain_share_path() {
        assert_eq!(
            plain_form(r"\\?\UNC\server\share\repo").as_deref(),
            Some(r"\\server\share\repo")
        );
        assert_eq!(
            plain_form(r"\\?\UNC\wsl.localhost\Ubuntu\home\me").as_deref(),
            Some(r"\\wsl.localhost\Ubuntu\home\me")
        );
        assert_eq!(plain_form(r"\\?\UNC\server").as_deref(), None);
    }

    #[test]
    fn a_name_windows_reads_differently_keeps_the_path_verbatim() {
        for path in [
            r"\\?\D:\work\CON",
            r"\\?\D:\work\con.txt",
            r"\\?\D:\work\Lpt1 .rs",
            r"\\?\D:\work\trailing.",
            r"\\?\D:\work\trailing ",
            r"\\?\D:\work\..\x",
            r"\\?\D:\work\a/b",
            r"\\?\UNC\server\share\aux",
        ] {
            assert_eq!(plain_form(path), None, "{path}");
        }
        assert!(plain_form(r"\\?\D:\work\console\com10\a.con").is_some());
    }

    #[test]
    fn a_path_too_long_for_max_path_stays_verbatim() {
        let fits = format!(r"\\?\D:\{}\{}", "a".repeat(127), "b".repeat(128));
        assert_eq!(plain_form(&fits).map(|p| p.len()), Some(MAX_PATH - 1));
        let long = format!(r"\\?\D:\{}\{}", "a".repeat(128), "b".repeat(128));
        assert_eq!(plain_form(&long), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_canonical_path_on_windows_is_in_the_plain_form() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = canonical(dir.path()).unwrap();
        assert!(!path.to_str().unwrap().starts_with(r"\\?\"), "{}", path.display());
        assert_eq!(
            std::fs::canonicalize(&path).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn a_path_that_is_not_verbatim_is_left_alone() {
        for path in [
            r"D:\work",
            r"\\server\share",
            r"\\.\pipe\x",
            r"\\?\GLOBALROOT\Device\x",
            r"\\?\D:x",
        ] {
            assert_eq!(plain_form(path), None, "{path}");
        }
    }

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
        let name = std::ffi::OsStr::from_bytes(b"d\xff/f");
        let path = from_relative(Path::new(name)).unwrap();
        assert_eq!(path.0, b"d\xff/f");
        assert_eq!(to_path(&path.0).unwrap(), Path::new(name));
    }
}
