#!/usr/bin/env bash
# Headless Chromium for screenshots and the benchmark, never left running: the browser
# is closed after VET_CHROMIUM_TIMEOUT seconds (default 600) or when this script exits,
# and its throwaway profile is deleted.
#
# Inside WSL it runs Windows Edge (the WebView2 engine, with the Windows GPU) instead
# of a Linux Chromium; this needs WSL's mirrored networking, so localhost is shared.
# VET_BROWSER=linux forces the Linux Chromium.
#
# Usage: ci/chromium.sh [chromium args...]
#   ci/chromium.sh --screenshot=out.png --window-size=1440,900 'http://localhost:1420/#…'
#   ci/chromium.sh --remote-debugging-port=9333 about:blank   (drive via CDP)
set -euo pipefail

edge="/mnt/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"
windows=false
if [[ ${VET_BROWSER:-} != linux && -x $edge ]] && grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then
    windows=true
fi

if $windows; then
    # From a Windows working directory, or cmd.exe complains about the UNC path.
    temp=$(cd /mnt/c && wslpath -u "$(/mnt/c/Windows/System32/cmd.exe /c 'echo %TEMP%' 2>/dev/null | tr -d '\r')")
    # Profiles Edge still held when an earlier run tried to delete them.
    find "$temp" -maxdepth 1 -name 'vet-edge.*' -mmin +10 -exec rm -rf {} + 2>/dev/null || true
    profile=$(mktemp -d "$temp/vet-edge.XXXXXX")
    profile_arg=$(wslpath -w "$profile")
else
    profile=$(mktemp -d)
    profile_arg=$profile
fi

# The browser is closed through CDP, which also reaches a Windows process.
port=""
args=()
for arg in "$@"; do
    case $arg in
        --remote-debugging-port=*) port=${arg#*=} ;;
        --screenshot=*) $windows && arg="--screenshot=$(wslpath -w "$(realpath -m "${arg#*=}")")" ;;
    esac
    args+=("$arg")
done
if [[ -z $port ]]; then
    port=$((20000 + RANDOM % 20000))
    args=(--remote-debugging-port="$port" "${args[@]}")
fi

close() {
    node -e '
        const port = process.argv[1];
        fetch(`http://127.0.0.1:${port}/json/version`)
            .then((r) => r.json())
            .then(({ webSocketDebuggerUrl }) => {
                const ws = new WebSocket(webSocketDebuggerUrl);
                ws.onopen = () => ws.send(JSON.stringify({ id: 1, method: "Browser.close" }));
                ws.onmessage = ws.onerror = () => process.exit(0);
                setTimeout(() => process.exit(0), 3000);
            })
            .catch(() => process.exit(0));' "$port" 2>/dev/null || true
    [[ -n ${pid:-} ]] && kill -- "-$pid" 2>/dev/null
    # Edge's helper processes hold the profile for a few seconds after the browser exits.
    for _ in {1..20}; do
        rm -rf "$profile" 2>/dev/null && break
        sleep 0.5
    done
}
trap close EXIT INT TERM

args=(--headless=new --no-first-run --no-default-browser-check --user-data-dir="$profile_arg" "${args[@]}")
if $windows; then
    cmd=("$edge" "${args[@]}")
elif command -v chromium >/dev/null; then
    cmd=(chromium "${args[@]}")
else
    cmd=(nix-shell -p chromium --run "chromium $(printf '%q ' "${args[@]}")")
fi

setsid timeout --kill-after=10 "${VET_CHROMIUM_TIMEOUT:-600}" "${cmd[@]}" &
pid=$!
wait "$pid"
