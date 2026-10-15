#!/usr/bin/env bash
# Enforces the onion's dependency direction (I1, I5). Fails if any crate depends
# on a workspace crate outside its allowlist, or if the pure crates pull in IO,
# git, or an async runtime.
set -euo pipefail
cd "$(dirname "$0")/.."

# crate: space-separated workspace crates it may depend on (normal dependencies).
ALLOWED_CRATES="vet-core vet-protocol vet-log vet-engine vet-host vet-transport
    vet-highlight vet-semantic vet-app vet-infra vet-mapping vet-backend"
allowed() {
    case $1 in
        vet-core | vet-protocol | vet-log | vet-engine) echo "" ;;
        vet-host) echo "vet-protocol" ;;
        vet-transport) echo "vet-protocol" ;;
        vet-highlight) echo "vet-core" ;;
        vet-semantic) echo "vet-core vet-highlight vet-engine" ;;
        vet-app) echo "vet-core vet-semantic vet-engine" ;;
        vet-infra) echo "vet-core vet-semantic vet-app vet-log vet-engine" ;;
        vet-mapping) echo "vet-core vet-protocol vet-semantic vet-app" ;;
        vet-backend) echo "vet-core vet-protocol vet-transport vet-semantic vet-app vet-infra vet-highlight vet-mapping vet-log" ;;
    esac
}

# External crates the pure layers must never depend on.
DENY='^(tokio|async-std|smol|futures|gix|gix-.*|git2|rusqlite|sqlx|notify|reqwest|hyper|tungstenite|tokio-tungstenite)$'
PURE="vet-core vet-semantic vet-protocol vet-engine"

metadata=$(cargo metadata --format-version 1 --no-deps)
status=0

for crate in $ALLOWED_CRATES; do
    # Workspace dependencies are the ones declared with a path.
    deps=$(jq -r --arg c "$crate" \
        '.packages[] | select(.name == $c) | .dependencies[] | select(.kind == null and .path != null) | .name' \
        <<<"$metadata")
    allow=$(allowed "$crate")
    for dep in $deps; do
        if [[ " $allow " != *" $dep "* ]]; then
            echo "FORBIDDEN: $crate depends on $dep (allowed: ${allow:-none})"
            status=1
        fi
    done
done

for crate in $PURE; do
    jq -r --arg c "$crate" \
        '.packages[] | select(.name == $c) | .dependencies[] | select(.path == null) | .name' \
        <<<"$metadata" | while read -r dep; do
        # The line diff is pure computation; hunks are domain policy.
        if [[ "$dep" =~ $DENY && "$dep" != gix-imara-diff ]]; then
            echo "FORBIDDEN: $crate depends on $dep (IO, git, or async runtime in a pure crate)"
            exit 1
        fi
    done || status=1
done

[[ $status -eq 0 ]] && echo "dependency direction OK"
exit $status
