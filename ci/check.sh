#!/usr/bin/env bash
# Runs every check: Rust (format, lint, tests, architecture) and UI (types, lint, format, tests, build).
#
# With BASE, the tests that start real language servers or restores (the `language-servers` and
# `roslyn` groups of .config/nextest.toml and ci/tool-tests.sh) run only when a change since BASE
# (committed, uncommitted or untracked) touches what they exercise; the rest runs in full.
#
# Usage: ci/check.sh [BASE]
set -euo pipefail
cd "$(dirname "$0")/.."
# Share compiled dependencies between checkouts (worktrees) through sccache, if installed.
: "${RUSTC_WRAPPER:=$PWD/ci/rustc-wrapper.sh}"
export RUSTC_WRAPPER

source ci/lib.sh

language_servers=true
if (($#)); then
    base=$(git merge-base HEAD "$1")
    language_servers=false
    while IFS= read -r f; do
        case $f in
            # The servers' protocol layer, their installs and restores, and the processes they run in.
            crates/vet-semantic/* | crates/vet-infra/src/lsp/* | crates/vet-infra/src/servers.rs \
                | crates/vet-infra/src/restore* | crates/vet-infra/src/materialize* \
                | crates/vet-infra/src/exec* | crates/vet-infra/src/language.rs \
                | crates/vet-app/src/semantics* | crates/vet-app/src/servers.rs \
                | crates/vet-app/src/trust.rs | crates/vet-app/src/session* \
                | crates/vet-backend/src/semantic.rs | crates/vet-backend/src/servers.rs \
                | crates/vet-backend/src/symbols.rs | crates/vet-backend/src/problems.rs \
                | crates/vet-backend/src/supervisor.rs | crates/vet-backend/src/trust.rs \
                | crates/vet-backend/src/open.rs \
                | crates/vet-infra/tests/* | crates/vet-backend/tests/* \
                | Cargo.lock | .config/nextest.toml | ci/tool-tests.sh) language_servers=true ;;
        esac
    done < <({ git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u)
fi

step "cargo fmt"
cargo fmt --all --check
cargo fmt --manifest-path ui/src-tauri/Cargo.toml --check
step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings
step "cargo clippy (ui/src-tauri)"
tauri_step "clippy (ui/src-tauri)" cargo clippy --manifest-path ui/src-tauri/Cargo.toml --all-targets -- -D warnings
step "cargo nextest"
# The tests of the `language-servers` group, as .config/nextest.toml selects them.
language_server_tests='binary(it) & ((package(vet-infra) & test(/^(lsp|servers|restore)::/)) | (package(vet-backend) & test(/^(semantic|restore|python)::/)))'
if $language_servers; then
    cargo nextest run --workspace
else
    cargo nextest run --workspace -E "not ($language_server_tests)"
fi
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
if $language_servers; then
    tool_log=$(mktemp)
    ci/tool-tests.sh 2>&1 | tee "$tool_log"
    not_run=$(grep '^NOT RUN:' "$tool_log" || true)
    rm -f "$tool_log"
    [[ -z $not_run ]] || skip "tests that need tools: ${not_run#NOT RUN: }"
else
    skip "language-server and tool tests: nothing since $1 touches what they exercise"
fi
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
