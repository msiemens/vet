# Shared by ci/check.sh and ci/quick.sh; sourced, not run.

step() { printf '\n== %s\n' "$*"; }

# Steps that could not run on this machine, said again at the end so that a pass is never read as
# more than it covered.
skipped=()

skip() {
    skipped+=("$1")
    printf 'SKIPPED: %s\n' "$1"
}

# `tauri_step LABEL COMMAND...` runs COMMAND where the Tauri host builds (it needs the system's
# webview libraries): here, in a nix shell, or not at all, in which case the step is a skipped one.
tauri_step() {
    local label=$1 status=0
    shift
    if [[ $(uname) != Linux ]] || pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
        "$@" || status=$?
    elif command -v nix-shell >/dev/null; then
        nix-shell --run "$*" || status=$?
    else
        skip "$label: the GTK/webkit libraries are missing and nix-shell is not available"
        return 0
    fi
    return "$status"
}

# Says what was skipped, last.
finish() {
    if ((${#skipped[@]})); then
        printf '\n%s\n' "$1, but ${#skipped[@]} step(s) did not run:"
        printf '  - %s\n' ${skipped[@]+"${skipped[@]}"}
    else
        printf '\n%s\n' "$1"
    fi
}
