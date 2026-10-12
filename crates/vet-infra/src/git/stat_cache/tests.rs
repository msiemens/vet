use std::collections::HashMap;

use vet_core::content::{BlobId, EntryKind, FileEntry, RepoPath};

use super::{Record, StatCache, StatKey, file_stamp};

fn key() -> StatKey {
    StatKey {
        size: 10,
        mtime: (1_000, 5),
        ctime: (1_000, 7),
        ino: 42,
        index_mode: Some(0o100644),
    }
}

fn record(key: StatKey, recorded_secs: i64) -> Record {
    Record {
        key,
        entry: FileEntry {
            blob: BlobId::of(b"content"),
            mode: 0o100644,
            kind: EntryKind::File,
            unfiltered: false,
        },
        unavailable: false,
        recorded_secs,
    }
}

fn records(paths: &[&str]) -> HashMap<RepoPath, Record> {
    paths.iter().map(|p| ((*p).into(), record(key(), 2_000))).collect()
}

fn paths(records: HashMap<RepoPath, Record>) -> Vec<String> {
    let mut found: Vec<_> = records.keys().map(|p| p.to_string_lossy()).collect();
    found.sort();
    found
}

#[test]
fn a_record_answers_for_a_file_with_the_same_stat_that_was_read_after_it_last_changed() {
    let record = record(key(), 1_001);

    assert!(record.matches(&key()));
}

#[test]
fn a_file_modified_in_the_second_it_was_read_could_have_changed_without_the_stat_showing_it() {
    for recorded in [1_000, 999] {
        assert!(!record(key(), recorded).matches(&key()), "recorded at {recorded}");
    }
}

#[test]
fn a_record_answers_for_no_file_whose_stat_differs_in_any_part() {
    type Change = fn(&mut StatKey);
    let changed: [(&str, Change); 6] = [
        ("size", |k| k.size += 1),
        ("mtime seconds", |k| k.mtime.0 -= 1),
        ("mtime nanoseconds", |k| k.mtime.1 += 1),
        ("ctime", |k| k.ctime.1 += 1),
        ("inode", |k| k.ino += 1),
        ("index mode", |k| k.index_mode = None),
    ];
    for (part, change) in changed {
        let mut other = key();
        change(&mut other);
        assert!(!record(key(), 2_000).matches(&other), "{part}");
    }
}

#[test]
fn records_are_handed_out_once_and_what_a_scan_puts_back_replaces_them() {
    let cache = StatCache::default();
    let config = vec![Some((1, (1, 0)))];
    assert!(cache.take(config.clone()).is_empty());

    cache.put(records(&["a", "b"]));
    assert_eq!(paths(cache.take(config.clone())), ["a", "b"]);
    assert!(cache.take(config.clone()).is_empty(), "taken, not copied");

    cache.put(records(&["a", "b"]));
    cache.put(records(&["b", "c"]));
    assert_eq!(paths(cache.take(config)), ["b", "c"], "a path that is gone is dropped");
}

#[test]
fn records_made_under_other_configuration_are_dropped() {
    let cache = StatCache::default();
    let before = vec![Some((1, (1, 0))), None];
    cache.take(before.clone());
    cache.put(records(&["a"]));

    assert!(
        cache.take(vec![Some((2, (1, 0))), None]).is_empty(),
        "the file changed in size"
    );

    cache.put(records(&["a"]));
    assert!(
        cache.take(before).is_empty(),
        "and the old configuration is not the current one"
    );
}

#[test]
fn a_config_files_stamp_follows_its_content_and_a_missing_file_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config");
    assert_eq!(file_stamp(&file), None);

    std::fs::write(&file, "[core]\n").unwrap();
    let short = file_stamp(&file).unwrap();
    std::fs::write(&file, "[core]\n\tbare = false\n").unwrap();
    let longer = file_stamp(&file).unwrap();

    assert_eq!((short.0, longer.0), (7, 21), "the sizes");
    assert_eq!(longer, file_stamp(&file).unwrap());
}
