#!/usr/bin/env bash
# Fails if the module graph of any workspace crate (library or binary) has a cycle.
# Needs cargo-modules: cargo install cargo-modules --locked
#
# `cargo modules dependencies --acyclic` checks the unfiltered item graph, where every
# type and its methods form a cycle (`Type -owns-> Type::method -uses-> Type`), so the
# flag cannot be used as is. Instead the graph is filtered down to modules, written as
# dot, and its `use` edges are checked with tsort. Edges between a module and its own
# ancestor (`use super::X`, `mod child; use child::X`) are ignored: that is how a Rust
# module tree is meant to read, not a layering cycle.
#
# Every cargo-modules run loads the whole dependency graph into rust-analyzer, which
# costs seconds per target, so the targets are analysed in parallel.
#
# Usage: check-module-cycles.sh [crate...]   restrict the check to these crates
#        check-module-cycles.sh --self-test  check that a known cycle is detected
set -euo pipefail
cd "$(dirname "$0")/.."

if ! cargo modules --help >/dev/null 2>&1; then
    echo "cargo-modules is not installed: cargo install cargo-modules --locked" >&2
    exit 2
fi

FILTERS=(--no-externs --no-sysroot --no-fns --no-types --no-traits --no-owns)

# Module dependency graph as dot on stdout. Args: cargo-modules target arguments.
module_graph() {
    cargo modules dependencies "$@" "${FILTERS[@]}"
}

# Reads dot on stdin; prints the cycle (if any) and returns 1 when there is one.
check_graph() {
    local edges cycle
    edges=$(awk -F'"' '
        / -> .*"uses"/ {
            from = $2; to = $4
            if (index(to, from "::") == 1 || index(from, to "::") == 1) next
            print from, to
        }')
    [[ -n $edges ]] || return 0
    # GNU tsort exits 1 on a loop, BSD tsort exits 0; both report it on stderr only.
    cycle=$(tsort <<<"$edges" 2>&1 >/dev/null || true)
    if [[ -n $cycle ]]; then
        echo "$cycle" | grep -v -e '^tsort: -: input contains a loop' -e '^tsort: cycle in data' |
            sed 's/^tsort: /  /'
        return 1
    fi
}

if [[ ${1:-} == --self-test ]]; then
    fixture=ci/fixtures/module-cycle
    if out=$(module_graph --manifest-path "$fixture/Cargo.toml" --lib | check_graph); then
        echo "self-test failed: the cycle in $fixture was not detected" >&2
        exit 1
    fi
    echo "self-test ok, detected:"
    echo "$out"
    exit 0
fi

only=" $* "
out_dir=$(mktemp -d)
trap 'rm -rf "$out_dir"' EXIT

targets=()
pids=()
while IFS=$'\t' read -r crate kind target; do
    [[ $# -eq 0 || $only == *" $crate "* ]] || continue
    if [[ $kind == lib ]]; then
        target_args=(--lib)
    else
        target_args=(--bin "$target")
    fi
    n=${#targets[@]}
    targets+=("$crate ($kind $target)")
    module_graph --package "$crate" "${target_args[@]}" >"$out_dir/$n.dot" 2>"$out_dir/$n.err" &
    pids+=($!)
done < <(cargo metadata --format-version 1 --no-deps | jq -r '
    . as $m | .packages[] | select(.id as $id | $m.workspace_members | index($id)) | .name as $p
    | .targets[] | select(.kind[0] == "lib" or .kind[0] == "bin")
    | [$p, .kind[0], .name] | @tsv')

status=0
for n in "${!targets[@]}"; do
    if ! wait "${pids[$n]}"; then
        echo "cargo-modules failed for ${targets[$n]}:" >&2
        cat "$out_dir/$n.err" >&2
        status=1
    elif ! cycle=$(check_graph <"$out_dir/$n.dot"); then
        echo "CYCLE in ${targets[$n]}:"
        echo "$cycle"
        status=1
    fi
done

[[ $status -eq 0 ]] && echo "module graphs acyclic"
exit $status
