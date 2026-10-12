use vet_core::content::{BlobId, Hash, SnapshotId};
use vet_semantic::policy::AnalysisPolicy;
use vet_semantic::ports::BackendId;
use vet_semantic::reuse::Basis;

use super::*;

fn span() -> Span {
    Span {
        start: Pos { line: 1, col: 2 },
        end: Pos { line: 3, col: 4 },
    }
}

fn symbols() -> Value {
    Value::Symbols(vec![SymbolInfo {
        sym: Sym {
            file: "a".into(),
            name: vec!["X".into(), "y".into()],
        },
        span: span(),
        at: Pos { line: 5, col: 6 },
        detail: Some("d".into()),
        container: true,
    }])
}

fn locations() -> Value {
    Value::Locations(vec![Location {
        file: "a".into(),
        span: span(),
        external: true,
    }])
}

fn diagnostics() -> Value {
    Value::Diagnostics(vec![Diagnostic {
        span: span(),
        severity: Severity::Warning,
        message: "m".into(),
        source: Some("s".into()),
        code: None,
    }])
}

fn hover() -> Value {
    Value::Hover(Some("hi".into()))
}

/// The bytes rows already written hold, spelled out: numbers are four bytes little-endian, text
/// and byte strings carry their length, an optional text a flag byte.
fn stored() -> [(Value, Vec<u8>); 5] {
    let span = [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0];
    let join = |parts: &[&[u8]]| parts.concat();
    [
        (
            symbols(),
            join(&[
                &[1, 0, 0, 0],       // one symbol
                &[1, 0, 0, 0, b'a'], // file
                &[2, 0, 0, 0],       // two name parts
                &[1, 0, 0, 0, b'X'],
                &[1, 0, 0, 0, b'y'],
                &span,
                &[5, 0, 0, 0, 6, 0, 0, 0], // at
                &[1, 1, 0, 0, 0, b'd'],    // detail
                &[1],                      // container
            ]),
        ),
        (locations(), join(&[&[1, 0, 0, 0], &[1, 0, 0, 0, b'a'], &span, &[1]])),
        (
            diagnostics(),
            join(&[
                &[1, 0, 0, 0],
                &span,
                &[1], // warning
                &[1, 0, 0, 0, b'm'],
                &[1, 1, 0, 0, 0, b's'],
                &[0],
            ]),
        ),
        (hover(), vec![1, 2, 0, 0, 0, b'h', b'i']),
        (Value::Hover(None), vec![0]),
    ]
}

#[test]
fn the_stored_form_of_each_kind_is_the_one_rows_already_written_hold() {
    for (value, bytes) in stored() {
        let (kind, encoded) = encode(&value);

        assert_eq!(encoded, bytes, "{value:?}");
        assert_eq!(format!("{:?}", decode(kind, &bytes)), format!("{:?}", Some(&value)));
    }
}

#[test]
fn every_severity_is_stored_as_its_own_byte_and_no_other_byte_is_a_severity() {
    let with = |severity: u8| {
        let (_, mut bytes) = encode(&diagnostics());
        bytes[4 + 16] = severity;
        decode(Kind::Diagnostics, &bytes)
    };
    let read: Vec<_> = (0..4)
        .map(|b| match with(b) {
            Some(Value::Diagnostics(d)) => d[0].severity,
            other => panic!("{other:?}"),
        })
        .collect();

    assert_eq!(
        read,
        [Severity::Error, Severity::Warning, Severity::Info, Severity::Hint]
    );
    assert!(with(4).is_none());
    assert!(with(255).is_none());
}

#[test]
fn a_value_cut_short_or_followed_by_more_is_refused_at_every_length() {
    for (value, bytes) in stored() {
        let kind = kind_of_value(&value);
        for cut in 0..bytes.len() {
            assert!(decode(kind, &bytes[..cut]).is_none(), "{value:?} cut to {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(kind, &longer).is_none(), "{value:?} with a byte more");
    }
}

#[test]
fn a_value_is_refused_as_a_kind_it_is_not() {
    let values = [symbols(), locations(), diagnostics(), hover()];
    for value in &values {
        let (own, bytes) = encode(value);
        for other in [Kind::Symbols, Kind::Locations, Kind::Hover, Kind::Diagnostics] {
            if other != own {
                assert!(decode(other, &bytes).is_none(), "{value:?} as {}", other.name());
            }
        }
    }
}

#[test]
fn bytes_that_are_not_text_or_not_a_flag_are_refused() {
    let (_, hover) = encode(&hover());
    let mut not_utf8 = hover.clone();
    *not_utf8.last_mut().unwrap() = 0xff;
    let mut not_a_flag = hover;
    not_a_flag[0] = 2;

    assert!(decode(Kind::Hover, &not_utf8).is_none());
    assert!(decode(Kind::Hover, &not_a_flag).is_none());

    let (_, mut location) = encode(&locations());
    *location.last_mut().unwrap() = 2;
    assert!(decode(Kind::Locations, &location).is_none());
}

#[test]
fn a_count_that_outruns_the_input_is_refused() {
    for kind in [Kind::Symbols, Kind::Locations, Kind::Diagnostics] {
        assert!(decode(kind, &u32::MAX.to_le_bytes()).is_none());
    }
    let mut name_count = vec![1, 0, 0, 0, 1, 0, 0, 0, b'a'];
    name_count.extend(u32::MAX.to_le_bytes());
    assert!(decode(Kind::Symbols, &name_count).is_none());
}

#[test]
fn text_and_paths_that_are_not_ascii_come_back_byte_for_byte() {
    let value = Value::Symbols(vec![SymbolInfo {
        sym: Sym {
            file: RepoPath(vec![0xff, b'/', 0xfe]),
            name: vec!["ü".into(), "日本".into(), String::new()],
        },
        span: span(),
        at: Pos { line: u32::MAX, col: 0 },
        detail: Some(String::new()),
        container: false,
    }]);
    let (kind, bytes) = encode(&value);

    assert_eq!(format!("{:?}", decode(kind, &bytes)), format!("{:?}", Some(value)));
}

fn server() -> ServerIdentity {
    ServerIdentity {
        backend: BackendId("rust".into()),
        fingerprint: Hash([1; 32]),
        label: "ra".into(),
    }
}

fn project(query: ProjectQuery) -> Key {
    Key::Project {
        server: server(),
        basis: Basis {
            snapshot: SnapshotId(Hash([2; 32])),
            policy: AnalysisPolicy { allow_executing: false },
            restore: Restore::Recorded(Hash([3; 32])),
        },
        query,
    }
}

fn at() -> At {
    At {
        file: "a.rs".into(),
        pos: Pos { line: 1, col: 1 },
    }
}

#[test]
fn a_question_is_kept_under_the_kind_of_the_answer_it_gets() {
    let text = Key::Text {
        server: server(),
        extension: b"rs".to_vec(),
        blob: BlobId(Hash([4; 32])),
    };
    let questions = [
        (text, symbols()),
        (project(ProjectQuery::DocumentSymbols("a.rs".into())), symbols()),
        (project(ProjectQuery::Incoming(at())), symbols()),
        (project(ProjectQuery::Outgoing(at())), symbols()),
        (project(ProjectQuery::Definition(at())), locations()),
        (project(ProjectQuery::References(at())), locations()),
        (project(ProjectQuery::Implementations(at())), locations()),
        (project(ProjectQuery::Hover(at())), hover()),
        (project(ProjectQuery::Diagnostics("a.rs".into())), diagnostics()),
    ];
    for (key, answer) in questions {
        let (_, kind) = address(&key).unwrap();

        assert_eq!(kind, encode(&answer).0, "{key:?}");
    }
}

#[test]
fn the_kinds_have_ids_of_their_own() {
    let ids: std::collections::HashSet<_> = [Kind::Symbols, Kind::Locations, Kind::Hover, Kind::Diagnostics]
        .into_iter()
        .map(Kind::id)
        .collect();

    assert_eq!(ids.len(), 4);
}
