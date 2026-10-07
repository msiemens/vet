#!/usr/bin/env bash
# Enforces the onion's dependency direction (I1, I5). Fails if any crate depends
# on a workspace crate outside its allowlist, or if the pure crates pull in IO,
# git, or an async runtime.
set -euo pipefail
cd "$(dirname "$0")/.."

# crate: space-separated workspace crates it may depend on (normal dependencies).
declare -A ALLOWED=(
    [vet-core]=""
    [vet-protocol]=""
    [vet-transport]="vet-protocol"
    [vet-highlight]="vet-core"
    [vet-mock]="vet-core vet-protocol vet-transport vet-highlight vet-app"
    [vet-semantic]="vet-core"
    [vet-app]="vet-core vet-semantic"
    [vet-infra]="vet-core vet-semantic vet-app"
    [vet-backend]="vet-core vet-protocol vet-transport vet-semantic vet-app vet-infra vet-highlight"
)

# External crates the pure layers must never depend on.
DENY='^(tokio|async-std|smol|futures|gix|gix-.*|git2|rusqlite|sqlx|notify|reqwest|hyper|tungstenite|tokio-tungstenite)$'
PURE="vet-core vet-semantic vet-protocol"

metadata=$(cargo metadata --format-version 1 --no-deps)
status=0

for crate in "${!ALLOWED[@]}"; do
    # Workspace dependencies are the ones declared with a path.
    deps=$(jq -r --arg c "$crate" \
        '.packages[] | select(.name == $c) | .dependencies[] | select(.kind == null and .path != null) | .name' \
        <<<"$metadata")
    for dep in $deps; do
        if [[ " ${ALLOWED[$crate]} " != *" $dep "* ]]; then
            echo "FORBIDDEN: $crate depends on $dep (allowed: ${ALLOWED[$crate]:-none})"
            status=1
        fi
    done
done

for crate in $PURE; do
    jq -r --arg c "$crate" \
        '.packages[] | select(.name == $c) | .dependencies[] | select(.path == null) | .name' \
        <<<"$metadata" | while read -r dep; do
        if [[ "$dep" =~ $DENY ]]; then
            echo "FORBIDDEN: $crate depends on $dep (IO, git, or async runtime in a pure crate)"
            exit 1
        fi
    done || status=1
done

[[ $status -eq 0 ]] && echo "dependency direction OK"
exit $status
