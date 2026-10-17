#!/usr/bin/env bash
# Fast check scoped to what changed since BASE (default: the merge base with main),
# including uncommitted and untracked files. Rust: format the changed files; clippy,
# test and module cycles for the changed crates and the workspace crates depending on
# them. ui/src-tauri: rustfmt, clippy and its tests when it changed. UI: svelte-check, lint and format for the changed files, unit tests,
# and the invariant harness when ui/src/state, the harness or the backend's crates changed. Skips
# `pnpm build`; ci/check.sh remains the full check before merging.
#
# Usage: ci/quick.sh [BASE]
set -euo pipefail
cd "$(dirname "$0")/.."
# Share compiled dependencies between checkouts (worktrees) through sccache, if installed.
: "${RUSTC_WRAPPER:=$PWD/ci/rustc-wrapper.sh}"
export RUSTC_WRAPPER

base=$(git merge-base HEAD "${1:-main}")
changed=()
while IFS= read -r f; do changed+=("$f"); done \
    < <({ git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u)
exists() { [[ -e $1 ]]; }

source ci/lib.sh

rust_files=() crates=() tauri=false protocol=false deps=false cycle_tool=false harness=false ui_files=()
for f in ${changed[@]+"${changed[@]}"}; do
    case $f in
        tools/module-cycles/* | ci/check-module-cycles.sh | ci/fixtures/module-cycle*) cycle_tool=true ;;
        ui/src-tauri/*) tauri=true ;;
        ui/src/generated/*) ;;
        ui/*)
            exists "$f" && ui_files+=("${f#ui/}")
            [[ $f == ui/src/state/* || $f == ui/test/harness/* ]] && harness=true
            ;;
        # The lint configuration of every crate.
        crates/clippy.toml) for d in crates/*/; do crates+=("$(basename "$d")"); done ;;
        crates/*)
            crates+=("$(cut -d/ -f2 <<<"$f")")
            [[ $f == *.rs ]] && exists "$f" && rust_files+=("$f")
            [[ $f == */Cargo.toml ]] && deps=true
            [[ $f == crates/vet-protocol/* ]] && protocol=true
            # What the UI harness's real backend is made of.
            [[ $f == crates/vet-protocol/* || $f == crates/vet-backend/* || $f == crates/vet-app/* ]] && harness=true
            ;;
        Cargo.toml | Cargo.lock) deps=true ;;
        ci/check-deps.sh) deps=true ;;
    esac
done

if ((${#crates[@]})); then
    deduped=()
    while IFS= read -r c; do deduped+=("$c"); done \
        < <(printf '%s\n' ${crates[@]+"${crates[@]}"} | sort -u)
    crates=(${deduped[@]+"${deduped[@]}"})
    # The changed crates plus every workspace crate that depends on them, transitively.
    affected=()
    while IFS= read -r c; do affected+=("$c"); done \
        < <(cargo metadata --format-version 1 --no-deps | jq -r --args '
        [.packages[] | {name, deps: [.dependencies[] | select(.path != null) | .name]}] as $p
        | def up($set): ($set + [$p[] | select(any(.deps[]; . as $d | $set | index($d))) | .name] | unique)
              as $next | if $next == $set then $set else up($next) end;
        up($ARGS.positional | unique)[]' ${crates[@]+"${crates[@]}"})
    pkgs=()
    for c in ${affected[@]+"${affected[@]}"}; do pkgs+=(-p "$c"); done

    if ((${#rust_files[@]})); then
        step "rustfmt (${#rust_files[@]} files)"
        rustfmt --check --edition 2024 "${rust_files[@]}"
    fi
    step "cargo clippy (${affected[*]+"${affected[*]}"})"
    cargo clippy ${pkgs[@]+"${pkgs[@]}"} --all-targets -- -D warnings
    step "cargo nextest (${affected[*]+"${affected[*]}"})"
    cargo nextest run ${pkgs[@]+"${pkgs[@]}"}
    if $protocol; then
        step "generated bindings are fresh"
        dirty=$(git status --porcelain -- ui/src/generated)
        if [[ -n $dirty ]]; then
            printf '%s\n' "$dirty" >&2
            echo "ui/src/generated does not match vet-protocol; commit the bindings the tests regenerated" >&2
            exit 1
        fi
    fi
    step "tests that need tools (${affected[*]+"${affected[*]}"})"
    tool_tests ${pkgs[@]+"${pkgs[@]}"}
    step "module cycles (${crates[*]+"${crates[*]}"})"
    ci/check-module-cycles.sh ${crates[@]+"${crates[@]}"}
fi
if $deps; then
    step "check-deps"
    ci/check-deps.sh
fi
if $cycle_tool; then
    step "cargo fmt (tools/module-cycles)"
    cargo fmt --manifest-path tools/module-cycles/Cargo.toml --check
    step "cargo clippy (tools/module-cycles)"
    cargo clippy --manifest-path tools/module-cycles/Cargo.toml -- -D warnings
    step "module cycles (self-test, whole workspace)"
    ci/check-module-cycles.sh --self-test
    ci/check-module-cycles.sh
fi
if $tauri; then
    step "cargo fmt (ui/src-tauri)"
    cargo fmt --manifest-path ui/src-tauri/Cargo.toml --check
    step "cargo clippy (ui/src-tauri)"
    tauri_step "clippy (ui/src-tauri)" cargo clippy --manifest-path ui/src-tauri/Cargo.toml --all-targets -- -D warnings
    step "cargo nextest (ui/src-tauri)"
    tauri_step "tests (ui/src-tauri)" cargo nextest run --manifest-path ui/src-tauri/Cargo.toml
fi

if ((${#ui_files[@]})) || $protocol; then
    cd ui
    step "pnpm check"
    pnpm check
    lintable=()
    while IFS= read -r f; do lintable+=("$f"); done \
        < <(printf '%s\n' ${ui_files[@]+"${ui_files[@]}"} | grep -E '\.(ts|js|mjs|svelte)$' || true)
    if ((${#lintable[@]})); then
        step "eslint (${#lintable[@]} files)"
        pnpm exec eslint "${lintable[@]}"
    fi
    if ((${#ui_files[@]})); then
        step "prettier (${#ui_files[@]} files)"
        pnpm exec prettier --check --ignore-unknown "${ui_files[@]}"
    fi
    step "pnpm test"
    pnpm test
    cd ..
fi
if $harness; then
    step "pnpm harness (the stores against a real backend)"
    cargo build -p vet-backend
    (cd ui && pnpm harness)
fi

finish "quick checks passed (run ci/check.sh before merging)"
