#!/usr/bin/env bash
# usage: ci/prune-incremental.sh <incremental dir> [--dry-run]
#
# Deletes the incremental folders of earlier builds. rustc names each folder <crate>-<hash>, a hash
# nothing else in the target directory carries, so cargo-sweep cannot tell which build a folder
# belongs to and never deletes one. One build of a crate's variants (lib, tests, clippy, binaries)
# touches all of their folders within minutes, so a folder last touched well before its crate's
# newest one belongs to build settings that are no longer built: it is stale.
set -euo pipefail
inc=$1 dry_run=${2:-}
# How much older than its crate's newest folder a folder may be and still count as current.
window=$((60 * 60))
[[ -d $inc ]] || exit 0

mtime() { stat -c %Y "$1" 2>/dev/null || stat -f %m "$1"; }

n=0 crate="" newest=0
while read -r name time path; do
    if [[ $name != "$crate" ]]; then
        crate=$name newest=$time
    elif ((time < newest - window)); then
        n=$((n + 1))
        if [[ $dry_run == --dry-run ]]; then echo "$path"; else rm -rf "$path"; fi
    fi
done < <(
    for d in "$inc"/*/; do
        d=${d%/} name=${d##*/}
        echo "${name%-*} $(mtime "$d") $d"
    done | sort -k1,1 -k2,2nr
)
echo "stale incremental folders in $inc: $n" >&2
