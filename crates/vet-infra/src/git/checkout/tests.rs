use vet_core::content::RepoPath;

use super::checked;

fn bad(path: &[u8]) -> bool {
    checked(&RepoPath(path.to_vec())).is_err()
}

#[test]
fn a_path_that_could_leave_the_root_or_reach_into_git_s_own_directory_is_refused() {
    for path in [
        &b".."[..],
        b"../x",
        b"a/../x",
        b"a/..",
        b".",
        b"a/./b",
        b"",
        b"/abs",
        b"a//b",
        b"a/",
        b".git",
        b".git/config",
        b"a/.git/hooks/pre-commit",
        b".GIT/config",
        b"sub/.Git",
    ] {
        assert!(bad(path), "{:?}", String::from_utf8_lossy(path));
    }
}

#[test]
fn names_that_only_resemble_the_reserved_ones_are_written() {
    for path in [
        &b".gitignore"[..],
        b".github/workflows/ci.yml",
        b"a/.gitattributes",
        b"..x",
        b"x..",
        b"a/b.git",
        b"git/x",
    ] {
        assert!(!bad(path), "{:?}", String::from_utf8_lossy(path));
    }
}

#[cfg(unix)]
#[test]
fn a_name_that_is_not_utf8_is_a_path_like_any_other() {
    let path = RepoPath(b"d\xffir/f\xfe.txt".to_vec());

    let checked = checked(&path).unwrap();

    use std::os::unix::ffi::OsStrExt;
    assert_eq!(checked.as_os_str().as_bytes(), b"d\xffir/f\xfe.txt");
}
