#!/usr/bin/env bash
# Runs every check: Rust (format, lint, tests, architecture) and UI (types, lint, format, tests, build).
set -euo pipefail
cd "$(dirname "$0")/.."
# Share compiled dependencies between checkouts (worktrees) through sccache, if installed.
: "${RUSTC_WRAPPER:=$PWD/ci/rustc-wrapper.sh}"
export RUSTC_WRAPPER

source ci/lib.sh

step "cargo fmt"
cargo fmt --all --check
cargo fmt --manifest-path ui/src-tauri/Cargo.toml --check
step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings
step "cargo clippy (ui/src-tauri)"
tauri_step "clippy (ui/src-tauri)" cargo clippy --manifest-path ui/src-tauri/Cargo.toml --all-targets -- -D warnings
step "cargo nextest"
cargo nextest run --workspace
step "generated bindings are fresh"
dirty=$(git status --porcelain -- ui/src/generated)
if [[ -n $dirty ]]; then
    printf '%s\n' "$dirty" >&2
    echo "ui/src/generated does not match vet-protocol; commit the bindings the tests regenerated" >&2
    exit 1
fi
step "cargo nextest (ui/src-tauri)"
tauri_step "tests (ui/src-tauri)" cargo nextest run --manifest-path ui/src-tauri/Cargo.toml
step "tests that need tools"
tool_log=$(mktemp)
ci/tool-tests.sh 2>&1 | tee "$tool_log"
not_run=$(grep '^NOT RUN:' "$tool_log" || true)
rm -f "$tool_log"
[[ -z $not_run ]] || skip "tests that need tools: ${not_run#NOT RUN: }"
step "check-deps"
ci/check-deps.sh
step "module cycles"
ci/check-module-cycles.sh

cd ui
step "pnpm check"
pnpm check
step "pnpm lint"
pnpm lint
step "pnpm format:check"
pnpm format:check
step "pnpm test"
pnpm test
step "pnpm harness (the stores against a real backend)"
cargo build -p vet-backend
pnpm harness
step "pnpm build"
pnpm build
cd ..

# cargo never removes the artifacts of earlier builds. Stale incremental folders go first:
# cargo-sweep cannot see them but counts them against its cap, and would then delete every
# dependency build instead.
step "prune incremental folders"
for dir in . ui/src-tauri tools/module-cycles; do
    ci/prune-incremental.sh "$dir/target/debug/incremental"
done
step "cargo sweep"
if command -v cargo-sweep >/dev/null; then
    cargo sweep --maxsize 20GB
    cargo sweep --maxsize 20GB ui/src-tauri
    cargo sweep --maxsize 20GB tools/module-cycles
else
    skip "cargo sweep: cargo-sweep is not installed (cargo install cargo-sweep)"
fi

finish "all checks passed"
