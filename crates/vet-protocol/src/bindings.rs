//! Generation of the few TS modules that are not a type `ts-rs` can export (the protocol
//! version, the request/response pairing), next to the generated bindings.
//!
//! `cargo test -p vet-protocol` runs this, the same step that writes the `ts-rs` bindings, into
//! `TS_RS_EXPORT_DIR` (`ui/src/generated`). The check scripts compare the directory against git
//! afterwards, so a committed module that does not match the protocol fails the build.

use std::path::PathBuf;

/// Writes `contents` as `name` in the directory the bindings are exported to. Does nothing when
/// `TS_RS_EXPORT_DIR` is unset, which is not a place a check looks at.
pub(crate) fn write(name: &str, contents: &str) {
    let Some(dir) = std::env::var_os("TS_RS_EXPORT_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("could not create the generated directory");
    std::fs::write(dir.join(name), contents).expect("could not write a generated module");
}
