use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath, Snapshot};
use vet_core::merkle;

use super::{files, linguist_generated};

fn entry(kind: EntryKind, content: &[u8]) -> FileEntry {
    FileEntry {
        blob: BlobId::of(content),
        mode: 0o100644,
        kind,
        unfiltered: false,
    }
}

fn path(s: &str) -> RepoPath {
    s.into()
}

fn generated(files: &[(&str, &str)], paths: &[&str]) -> Vec<(String, bool)> {
    let files: Vec<_> = files.iter().map(|(p, c)| (path(p), c.as_bytes().to_vec())).collect();
    let found = linguist_generated(&files, paths.iter().map(|p| path(p)).collect());
    found.into_iter().map(|(p, set)| (p.to_string_lossy(), set)).collect()
}

#[test]
fn only_files_named_gitattributes_are_found_shallowest_first() {
    let entries = [
        ("b/.gitattributes", EntryKind::File),
        ("a/x/.gitattributes", EntryKind::File),
        (".gitattributes", EntryKind::File),
        ("a/.gitattributes", EntryKind::File),
        ("old.gitattributes", EntryKind::File),
        ("link/.gitattributes", EntryKind::Symlink),
        ("sub/.gitattributes", EntryKind::Submodule),
        ("dir.gitattributes/inner.txt", EntryKind::File),
    ]
    .into_iter()
    .map(|(p, kind)| (path(p), entry(kind, p.as_bytes())))
    .collect();
    let snapshot = Snapshot {
        id: merkle::snapshot_id(&entries),
        entries,
    };

    let found: Vec<_> = files(&snapshot).into_iter().map(|(p, _)| p).collect();

    assert_eq!(
        found,
        [
            path(".gitattributes"),
            path("a/.gitattributes"),
            path("b/.gitattributes"),
            path("a/x/.gitattributes"),
        ]
    );
}

#[test]
fn set_unset_and_the_boolean_values_are_told_apart_from_other_values_and_silence() {
    let attributes = "\
*.pb.go linguist-generated
/vendor/** -linguist-generated
docs/api.md linguist-generated=true
keep.txt linguist-generated=false
odd.txt linguist-generated=maybe
unrelated.txt linguist-vendored
";
    let found = generated(
        &[(".gitattributes", attributes)],
        &[
            "api/x.pb.go",
            "vendor/lib.js",
            "docs/api.md",
            "keep.txt",
            "odd.txt",
            "unrelated.txt",
            "plain.rs",
        ],
    );

    assert_eq!(
        found,
        [
            ("api/x.pb.go".to_owned(), true),
            ("docs/api.md".to_owned(), true),
            ("keep.txt".to_owned(), false),
            ("vendor/lib.js".to_owned(), false),
        ]
    );
}

#[test]
fn a_deeper_file_overrides_the_ones_above_and_reaches_only_its_own_directory() {
    let found = generated(
        &[
            (".gitattributes", "*.gen.rs linguist-generated\n"),
            (
                "sub/.gitattributes",
                "*.gen.rs -linguist-generated\nlocal.rs linguist-generated\n",
            ),
        ],
        &[
            "a.gen.rs",
            "sub/a.gen.rs",
            "sub/local.rs",
            "local.rs",
            "other/sub/local.rs",
        ],
    );

    assert_eq!(
        found,
        [
            ("a.gen.rs".to_owned(), true),
            ("sub/a.gen.rs".to_owned(), false),
            ("sub/local.rs".to_owned(), true),
        ]
    );
}

#[test]
fn a_path_that_is_not_utf8_is_judged_like_any_other() {
    let files = vec![
        (path(".gitattributes"), b"*.gen linguist-generated\n".to_vec()),
        (
            RepoPath(b"\xff/.gitattributes".to_vec()),
            b"* -linguist-generated\n".to_vec(),
        ),
    ];
    let odd = RepoPath(b"d\xffir/a.gen".to_vec());

    let found = linguist_generated(&files, vec![odd.clone()]);

    assert_eq!(found.get(&odd), Some(&true));
}
