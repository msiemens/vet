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
set -euo pipefail
cd "$(dirname "$0")/.."

if ! cargo modules --help >/dev/null 2>&1; then
    echo "cargo-modules is not installed: cargo install cargo-modules --locked" >&2
    exit 2
fi

FILTERS=(--no-externs --no-sysroot --no-fns --no-types --no-traits --no-owns)

status=0
while IFS=$'\t' read -r crate kind target; do
    if [[ $kind == lib ]]; then
        target_args=(--lib)
    else
        target_args=(--bin "$target")
    fi
    edges=$(cargo modules dependencies --package "$crate" "${target_args[@]}" "${FILTERS[@]}" |
        awk -F'"' '
            / -> .*"uses"/ {
                from = $2; to = $4
                if (index(to, from "::") == 1 || index(from, to "::") == 1) next
                print from, to
            }')
    if [[ -n $edges ]] && ! cycle=$(tsort <<<"$edges" 2>&1 >/dev/null); then
        echo "CYCLE in $crate ($kind $target):"
        echo "$cycle" | grep -v '^tsort: -: input contains a loop' | sed 's/^tsort: /  /'
        status=1
    fi
done < <(cargo metadata --format-version 1 --no-deps | jq -r '
    . as $m | .packages[] | select(.id as $id | $m.workspace_members | index($id)) | .name as $p
    | .targets[] | select(.kind[0] == "lib" or .kind[0] == "bin")
    | [$p, .kind[0], .name] | @tsv')

[[ $status -eq 0 ]] && echo "module graphs acyclic"
exit $status
