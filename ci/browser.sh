#!/usr/bin/env bash
# Ensures one shared headless browser is running and prints its CDP port.
# Clients (ci/cdp.mjs) get isolation from browser contexts, not from separate browsers.
#
# Inside WSL it is Windows Edge (the WebView2 engine, with the Windows GPU; needs WSL's
# mirrored networking, so localhost is shared). VET_BROWSER=linux forces Linux Chromium.
# An idle watchdog closes the browser after VET_BROWSER_IDLE seconds (default 600)
# without a page other than about:blank.
#
# Usage: port=$(ci/browser.sh)      VET_BROWSER_PORT (default 9222)
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
port=${VET_BROWSER_PORT:-9222}
edge="/mnt/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"

up() { curl -sf --max-time 2 "http://127.0.0.1:$port/json/version" >/dev/null 2>&1; }

# Launched processes must not inherit the lock fds, or they would hold the locks forever.
watchdog() {
    VET_BROWSER_PORT=$port VET_BROWSER_IDLE=${VET_BROWSER_IDLE:-600} \
        setsid flock -n /tmp/vet-browser-watchdog.lock \
        node "$here/cdp.mjs" watchdog </dev/null >/dev/null 2>&1 9>&- &
}

if ! up; then
    exec 9>/tmp/vet-browser.lock
    flock 9
    # Another caller may have launched it while we waited for the lock.
    if ! up; then
        windows=false
        if [[ ${VET_BROWSER:-} != linux && -x $edge ]] && grep -qi microsoft /proc/sys/kernel/osrelease 2>/dev/null; then
            windows=true
        fi

        if $windows; then
            # From a Windows working directory, or cmd.exe complains about the UNC path.
            temp=$(cd /mnt/c && wslpath -u "$(/mnt/c/Windows/System32/cmd.exe /c 'echo %TEMP%' 2>/dev/null | tr -d '\r')")
            profile=$(wslpath -w "$temp/vet-browser-profile")
        else
            profile=/tmp/vet-browser-profile
        fi
        args=(--headless=new --no-first-run --no-default-browser-check --window-size=1440,900
            --remote-debugging-port="$port" --user-data-dir="$profile" about:blank)

        launched=false
        for attempt in 1 2 3 4; do
            if $windows; then
                setsid "$edge" "${args[@]}" </dev/null >/dev/null 2>&1 9>&- &
            elif command -v chromium >/dev/null; then
                setsid chromium "${args[@]}" </dev/null >/dev/null 2>&1 9>&- &
            else
                setsid nix-shell -p chromium --run "exec chromium $(printf '%q ' "${args[@]}")" </dev/null >/dev/null 2>&1 9>&- &
            fi
            # An interop failure shows as a process that never serves the port.
            for _ in $(seq 1 60); do
                up && { launched=true; break; }
                sleep 0.5
            done
            $launched && break
            echo "browser.sh: launch attempt $attempt failed, retrying" >&2
            sleep $((attempt * 2))
        done
        $launched || { echo "browser.sh: browser did not start on port $port" >&2; exit 1; }
    fi
fi

watchdog
echo "$port"
