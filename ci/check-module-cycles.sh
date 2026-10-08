#!/usr/bin/env bash
# Fails if the module graph of any workspace crate (library or binary) has a cycle.
#
# tools/module-cycles loads the workspace into rust-analyzer once and builds, per target,
# the module graph `cargo modules dependencies` reports; see its crate documentation.
# Edges between a module and its own ancestor (`use super::X`, `mod child; use child::X`)
# are ignored: that is how a Rust module tree is meant to read, not a layering cycle.
#
# Usage: check-module-cycles.sh [crate...]   restrict the check to these crates
#        check-module-cycles.sh --self-test  check that a known cycle is detected
set -euo pipefail
cd "$(dirname "$0")/.."
# Share compiled dependencies between checkouts (worktrees) when sccache is installed.
if [[ -z ${RUSTC_WRAPPER:-} ]] && command -v sccache >/dev/null; then export RUSTC_WRAPPER=sccache; fi

manifest=tools/module-cycles/Cargo.toml
cargo build --release --quiet --manifest-path "$manifest"
tool=tools/module-cycles/target/release/module-cycles

if [[ ${1:-} == --self-test ]]; then
    fixture=ci/fixtures/module-cycle
    status=0
    out=$("$tool" --manifest-path "$fixture/Cargo.toml") || status=$?
    if [[ $status -ne 1 ]]; then
        echo "self-test failed: the cycle in $fixture was not detected (exit $status)" >&2
        exit 1
    fi
    echo "self-test ok, detected:"
    echo "$out"
    exit 0
fi

status=0
"$tool" "$@" || status=$?
[[ $status -eq 0 ]] && echo "module graphs acyclic"
exit $status
