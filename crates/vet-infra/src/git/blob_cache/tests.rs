use vet_core::content::BlobId;

use super::BlobCache;

fn repo() -> (tempfile::TempDir, gix::Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = gix::init(dir.path()).unwrap();
    (dir, repo)
}

#[test]
fn a_git_blob_has_the_blob_id_of_its_content_and_is_then_found_by_it() {
    let (_dir, repo) = repo();
    let oid = repo.write_blob(b"hello\n").unwrap().detach();
    let cache = BlobCache::default();
    let blob = BlobId::of(b"hello\n");

    assert_eq!(
        cache.odb_content(&repo, blob, u64::MAX).unwrap(),
        None,
        "not yet mapped"
    );
    assert_eq!(cache.blob_id(&repo, oid).unwrap(), blob);

    assert_eq!(cache.odb_content(&repo, blob, u64::MAX).unwrap().unwrap(), b"hello\n");
    assert_eq!(cache.odb_size(&repo, blob).unwrap(), Some(6));
}

#[test]
fn content_over_the_cap_is_not_returned_and_content_at_it_is() {
    let (_dir, repo) = repo();
    let oid = repo.write_blob(b"12345").unwrap().detach();
    let cache = BlobCache::default();
    let blob = cache.blob_id(&repo, oid).unwrap();

    assert!(cache.odb_content(&repo, blob, 4).unwrap().is_none());
    assert!(cache.odb_content(&repo, blob, 5).unwrap().is_some());
}

#[test]
fn an_object_the_database_lacks_is_an_error_that_names_it() {
    let (_dir, repo) = repo();
    let missing = gix::ObjectId::from_hex(b"0123456789abcdef0123456789abcdef01234567").unwrap();

    let error = BlobCache::default().blob_id(&repo, missing).unwrap_err();

    assert_eq!(error.message(), format!("missing object {missing}"));
}

#[test]
fn a_submodule_entry_is_its_pinned_commit_and_nothing_else_answers_to_it() {
    let (_dir, repo) = repo();
    let commit = gix::ObjectId::from_hex(b"0123456789abcdef0123456789abcdef01234567").unwrap();
    let other = gix::ObjectId::from_hex(b"fedcba9876543210fedcba9876543210fedcba98").unwrap();
    let cache = BlobCache::default();

    let id = cache.submodule_id(commit);

    assert_ne!(id, cache.submodule_id(other));
    assert_eq!(cache.submodule_commit(id), Some(commit.to_string()));
    assert_eq!(cache.submodule_commit(BlobId::of(b"file")), None);
    assert_eq!(
        cache.odb_content(&repo, id, u64::MAX).unwrap(),
        None,
        "no content to read"
    );
}
