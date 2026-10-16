//! The scan when the tree changes under it: an entry that is gone is dropped the way a
//! deletion is, the scan goes on, and nothing is reported about it.

use gix::bstr::{BStr, ByteSlice};

use super::super::BlobCache;
use super::super::stat_cache::StatCache;
use super::scan_with;
use crate::git::DEFAULT_CAP;

fn repo() -> (tempfile::TempDir, gix::Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = gix::init(dir.path()).unwrap();
    (dir, repo)
}

fn names(scan: &vet_app::ports::Scan) -> Vec<String> {
    scan.snapshot
        .entries
        .keys()
        .map(vet_core::content::RepoPath::to_string_lossy)
        .collect()
}

#[test]
fn a_directory_that_vanishes_during_the_walk_is_left_out_and_the_scan_succeeds() {
    let (_dir, repo) = repo();
    let root = repo.workdir().unwrap().to_owned();
    std::fs::create_dir_all(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/inner.txt"), "inner\n").unwrap();
    std::fs::write(root.join("kept.txt"), "kept\n").unwrap();

    let capture = vet_log::capture::Capture::default();
    let _log = capture.install(vet_log::filter(Some("warn")).0);
    let vanish = |path: &BStr| {
        if path == b"dir".as_bstr() {
            let _ = std::fs::remove_dir_all(root.join("dir"));
        }
    };

    let result = scan_with(&repo, &BlobCache::default(), &StatCache::default(), DEFAULT_CAP, vanish).unwrap();

    assert_eq!(names(&result), ["kept.txt"]);
    assert!(
        !capture.text().contains("skipping a directory"),
        "a vanished directory is not worth a warning: {}",
        capture.text()
    );
}

#[test]
fn a_file_that_vanishes_during_the_walk_is_left_out_without_a_warning() {
    let (_dir, repo) = repo();
    let root = repo.workdir().unwrap().to_owned();
    std::fs::write(root.join("gone.txt"), "gone\n").unwrap();
    std::fs::write(root.join("kept.txt"), "kept\n").unwrap();

    let capture = vet_log::capture::Capture::default();
    let _log = capture.install(vet_log::filter(Some("warn")).0);
    let vanish = |path: &BStr| {
        if path == b"gone.txt".as_bstr() {
            let _ = std::fs::remove_file(root.join("gone.txt"));
        }
    };

    let result = scan_with(&repo, &BlobCache::default(), &StatCache::default(), DEFAULT_CAP, vanish).unwrap();

    assert_eq!(names(&result), ["kept.txt"]);
    assert!(
        !capture.text().contains("cannot be read"),
        "a vanished file is not worth a warning: {}",
        capture.text()
    );
}
