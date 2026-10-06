#!/usr/bin/env bash
# Headless Chromium for screenshots and the benchmark, never left running: the
# process is killed after VET_CHROMIUM_TIMEOUT seconds (default 600) or when this
# script exits, and its throwaway profile is deleted.
#
# Usage: ci/chromium.sh [chromium args...]
#   ci/chromium.sh --screenshot=out.png --window-size=1440,900 'http://localhost:1420/#…'
#   ci/chromium.sh --remote-debugging-port=9222 about:blank   (drive via CDP)
set -euo pipefail

profile=$(mktemp -d)
cleanup() {
    [[ -n ${pid:-} ]] && kill -- "-$pid" 2>/dev/null
    rm -rf "$profile"
}
trap cleanup EXIT INT TERM

args=(--headless=new --no-first-run --no-default-browser-check --user-data-dir="$profile" "$@")
if command -v chromium >/dev/null; then
    cmd=(chromium "${args[@]}")
else
    cmd=(nix-shell -p chromium --run "chromium $(printf '%q ' "${args[@]}")")
fi

# Own process group, so the cleanup takes every renderer and helper with it.
setsid timeout --kill-after=10 "${VET_CHROMIUM_TIMEOUT:-600}" "${cmd[@]}" &
pid=$!
wait "$pid"
