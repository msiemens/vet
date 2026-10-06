#!/usr/bin/env bash
# Runs every check: Rust (format, lint, tests, architecture) and UI (types, lint, format, tests, build).
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n== %s\n' "$*"; }

step "cargo fmt"
cargo fmt --all --check
# ui/src-tauri is outside the workspace; rustfmt follows its modules from the entry points.
rustfmt --check ui/src-tauri/build.rs ui/src-tauri/src/main.rs
step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings
step "cargo test"
cargo test --workspace
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
step "pnpm build"
pnpm build

printf '\nall checks passed\n'
