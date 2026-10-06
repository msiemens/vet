#!/usr/bin/env bash
# Fast check scoped to what changed since BASE (default: the merge base with main),
# including uncommitted and untracked files. Rust: format the changed files; clippy,
# test and module cycles for the changed crates and the workspace crates depending on
# them. UI: svelte-check, lint and format for the changed files, unit tests. Skips
# `pnpm build`; ci/check.sh remains the full check before merging.
#
# Usage: ci/quick.sh [BASE]
set -euo pipefail
cd "$(dirname "$0")/.."

base=$(git merge-base HEAD "${1:-main}")
mapfile -t changed < <({ git diff --name-only "$base"; git ls-files --others --exclude-standard; } | sort -u)
exists() { [[ -e $1 ]]; }

step() { printf '\n== %s\n' "$*"; }

rust_files=() crates=() tauri=false protocol=false deps=false ui_files=()
for f in "${changed[@]}"; do
    case $f in
        ui/src-tauri/*) tauri=true ;;
        ui/src/generated/*) ;;
        ui/*) exists "$f" && ui_files+=("${f#ui/}") ;;
        crates/*)
            crates+=("$(cut -d/ -f2 <<<"$f")")
            [[ $f == *.rs ]] && exists "$f" && rust_files+=("$f")
            [[ $f == */Cargo.toml ]] && deps=true
            [[ $f == crates/vet-protocol/* ]] && protocol=true
            ;;
        Cargo.toml | Cargo.lock) deps=true ;;
        ci/check-deps.sh) deps=true ;;
    esac
done

if ((${#crates[@]})); then
    mapfile -t crates < <(printf '%s\n' "${crates[@]}" | sort -u)
    # The changed crates plus every workspace crate that depends on them, transitively.
    mapfile -t affected < <(cargo metadata --format-version 1 --no-deps | jq -r --args '
        [.packages[] | {name, deps: [.dependencies[] | select(.path != null) | .name]}] as $p
        | def up($set): ($set + [$p[] | select(any(.deps[]; . as $d | $set | index($d))) | .name] | unique)
              as $next | if $next == $set then $set else up($next) end;
        up($ARGS.positional | unique)[]' "${crates[@]}")
    pkgs=()
    for c in "${affected[@]}"; do pkgs+=(-p "$c"); done

    if ((${#rust_files[@]})); then
        step "rustfmt (${#rust_files[@]} files)"
        rustfmt --check --edition 2024 "${rust_files[@]}"
    fi
    step "cargo clippy (${affected[*]})"
    cargo clippy "${pkgs[@]}" --all-targets -- -D warnings
    step "cargo test (${affected[*]})"
    cargo test "${pkgs[@]}"
    step "module cycles (${crates[*]})"
    ci/check-module-cycles.sh "${crates[@]}"
fi
if $deps; then
    step "check-deps"
    ci/check-deps.sh
fi
if $tauri; then
    step "ui/src-tauri: run ci/check.sh for its fmt and clippy (needs the webkit toolchain)"
fi

if ((${#ui_files[@]})) || $protocol; then
    cd ui
    step "pnpm check"
    pnpm check
    mapfile -t lintable < <(printf '%s\n' "${ui_files[@]}" | grep -E '\.(ts|js|mjs|svelte)$' || true)
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
fi

printf '\nquick checks passed (run ci/check.sh before merging)\n'
