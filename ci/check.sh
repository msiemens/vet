#!/usr/bin/env bash
# Runs every check: Rust (format, lint, tests, architecture) and UI (types, lint, format, tests, build).
set -euo pipefail
cd "$(dirname "$0")/.."
# Share compiled dependencies between checkouts (worktrees) when sccache is installed.
if [[ -z ${RUSTC_WRAPPER:-} ]] && command -v sccache >/dev/null; then export RUSTC_WRAPPER=sccache; fi

step() { printf '\n== %s\n' "$*"; }

step "cargo fmt"
cargo fmt --all --check
cargo fmt --manifest-path ui/src-tauri/Cargo.toml --check
step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings
step "cargo clippy (ui/src-tauri)"
tauri_clippy=(cargo clippy --manifest-path ui/src-tauri/Cargo.toml --all-targets -- -D warnings)
if [[ $(uname) != Linux ]] || pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    "${tauri_clippy[@]}"
elif command -v nix-shell >/dev/null; then
    nix-shell --run "${tauri_clippy[*]}"
else
    echo "SKIPPED: the GTK/webkit libraries are missing and nix-shell is not available"
fi
step "cargo nextest"
cargo nextest run --workspace
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
cd ..

# cargo never removes the artifacts of earlier builds; keep each target directory under a cap.
step "cargo sweep"
if command -v cargo-sweep >/dev/null; then
    cargo sweep --maxsize 10GB
    cargo sweep --maxsize 10GB ui/src-tauri
else
    echo "SKIPPED: cargo-sweep is not installed (cargo install cargo-sweep)"
fi

printf '\nall checks passed\n'
