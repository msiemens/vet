#!/usr/bin/env bash
# Runs the tests that are ignored because they need a tool (`#[ignore = "needs python3"]`),
# for every tool that is installed, and says which tests it leaves out for the tools that are
# not. A test that needs a tool is never reported as passed without having run.
#
# Fails if an ignored test is claimed neither by a tool below nor by the tests that are run
# on request only (they download packages or talk to the network), so a new ignored test
# cannot go unrun.
#
# The tests that run on request are not part of ci/check.sh. They run by hand, before a change
# to restoring, the server installs or the python/first-mark paths is merged, and now and then
# (they depend on registries and package versions that move):
#
#     ci/tool-tests.sh --on-request
#
# Usage: ci/tool-tests.sh [--on-request] [-p CRATE]...   (default: the whole workspace)
set -euo pipefail
cd "$(dirname "$0")/.."
: "${RUSTC_WRAPPER:=$PWD/ci/rustc-wrapper.sh}"
export RUSTC_WRAPPER

mode=tools
if [[ ${1:-} == --on-request ]]; then
    mode=on-request
    shift
fi
scope=("${@:---workspace}")

# Tool, the command that tells it is installed, and the tests that need it.
names=(python3 rust-analyzer typescript-language-server pyright-langserver roslyn-language-server
    svelteserver ngserver yaml-language-server vscode-json-language-server bash-language-server
    "bash-language-server and shellcheck" docker-langserver)
probes=(
    "python3 --version"
    "rust-analyzer --version"
    "typescript-language-server --version"
    "command -v pyright-langserver"
    "command -v roslyn-language-server && dotnet --version"
    "command -v svelteserver"
    "command -v ngserver && command -v npm"
    "command -v yaml-language-server"
    "command -v vscode-json-language-server"
    "command -v bash-language-server"
    "command -v bash-language-server && command -v shellcheck"
    "command -v docker-langserver"
)
filters=(
    'package(vet-backend) & binary(it) & test(/^(restart|cancel|load)::/)'
    '(package(vet-infra) & binary(it) & test(/^lsp::rust_analyzer/)) | (package(vet-backend) & binary(it) & test(/^semantic::/))'
    'package(vet-infra) & binary(it) & test(/^lsp::typescript/)'
    'package(vet-infra) & binary(it) & test(/^lsp::pyright/)'
    'package(vet-infra) & binary(it) & test(/^lsp::roslyn/)'
    'package(vet-infra) & binary(it) & test(/^lsp::svelte/)'
    'package(vet-infra) & binary(it) & test(/^lsp::angular/)'
    'package(vet-infra) & binary(it) & test(/^lsp::yaml/)'
    'package(vet-infra) & binary(it) & test(/^lsp::json/)'
    'package(vet-infra) & binary(it) & test(/^lsp::bash_language_server_answers/)'
    'package(vet-infra) & binary(it) & test(/^lsp::bash_language_server_reports/)'
    'package(vet-infra) & binary(it) & test(/^lsp::dockerfile/)'
)
# Ignored tests that run on request: they download packages, talk to a network service or take
# long to measure.
on_request='binary(it) & ((package(vet-infra) & test(/^(servers|restore)::/)) | (package(vet-backend) & test(/^(python|restore|first_mark)::/)) | (package(vet-core) & test(/^review_poc::/)))'
# Ignored tests that another test starts as a child process; never run on their own.
children='test(crash_child)'

if [[ $mode == on-request ]]; then
    cargo nextest run "${scope[@]}" --run-ignored ignored-only --no-tests=pass -E "$on_request"
    exit
fi

union=""
for f in "${filters[@]}"; do union+="${union:+ | }($f)"; done
unclaimed=$(cargo nextest list "${scope[@]}" --run-ignored ignored-only --message-format oneline \
    -E "not (($union) | ($on_request) | ($children))")
if [[ -n $unclaimed ]]; then
    echo "ignored tests that no tool in ci/tool-tests.sh runs:" >&2
    echo "$unclaimed" >&2
    exit 1
fi

selected=()
missing=()
not_run=0
for i in "${!names[@]}"; do
    if bash -c "${probes[$i]}" >/dev/null 2>&1; then
        selected+=(-E "${filters[$i]}")
    else
        left=$(cargo nextest list "${scope[@]}" --run-ignored ignored-only --message-format oneline -E "${filters[$i]}")
        if [[ -n $left ]]; then
            echo "SKIPPED: not installed (${names[$i]}); not run:"
            sed 's/^/    /' <<<"$left"
            missing+=("${names[$i]}")
            not_run=$((not_run + $(wc -l <<<"$left")))
        fi
    fi
done
if ((${#selected[@]})); then
    cargo nextest run "${scope[@]}" --run-ignored ignored-only --no-tests=pass "${selected[@]}"
fi
if ((not_run)); then
    printf 'NOT RUN: %d tests for tools that are not installed: %s\n' "$not_run" "$(IFS=,; echo "${missing[*]}")"
fi
