# Vet

A review-first IDE for the agentic coding age. See [SPEC.md](SPEC.md).

## Layout

| Path | What |
|---|---|
| `crates/vet-core` | Pure domain model: snapshots, anchors, review state |
| `crates/vet-protocol` | Shell ↔ backend wire types; generates the TS types |
| `crates/vet-transport` | Authenticated loopback WebSocket server |
| `crates/vet-mock` | Mock orchestrator for the shell spike |
| `crates/vet-semantic` | Server pool, queries, derived analyses (depends only on `vet-core`) |
| `crates/vet-app` | Use cases and ports |
| `crates/vet-infra` | Adapters (git, watcher, store, LSP, ...) |
| `crates/vet-backend` | Composition root, the `vet-backend` binary |
| `ui/` | Svelte + CodeMirror shell |
| `ui/src-tauri/` | Tauri host (outside the Cargo workspace) |

## Rust

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all --check     # rustfmt.toml; also covers ui/src-tauri (see ci/check.sh)
./ci/check-deps.sh          # enforces the dependency direction between crates
./ci/check-module-cycles.sh # fails on a module cycle inside a crate
```

`ci/check-deps.sh` fails if a crate depends on one outside its allowlist, or if
a pure crate pulls in an async runtime, git or IO.

`ci/check-module-cycles.sh` needs `cargo install cargo-modules --locked`. It
fails if the module graph of a library or binary has a cycle, ignoring edges to
a module's own ancestors (`use super::X`). `cargo modules --acyclic` is not used
directly: it counts every type and its methods as a cycle.

## All checks

```sh
./ci/check.sh
```

Runs everything: rustfmt (both workspaces), clippy (`-D warnings`), tests,
`check-deps.sh`, `check-module-cycles.sh`, then in `ui/` `pnpm check`, `lint`,
`format:check`, `test` and `build`. Run it before merging.

`./ci/quick.sh [BASE]` checks only what changed since `BASE` (default: the merge
base with `main`): rustfmt on changed files, clippy/tests/module cycles for the
changed crates and their dependents, and svelte-check, eslint, prettier and unit
tests for changed UI files.

`./ci/chromium.sh [args…]` runs headless Chromium for screenshots or CDP: it is
killed after `VET_CHROMIUM_TIMEOUT` seconds (default 600) or when the script
exits, and its profile is deleted.

Formatting-only commits are listed in `.git-blame-ignore-revs`; enable them
with `git config blame.ignoreRevsFile .git-blame-ignore-revs`.

## Protocol types

The TS types in `ui/src/generated` are generated from `vet-protocol`. After
changing it:

```sh
cargo test -p vet-protocol
```

## Running the backend

```sh
cargo run -p vet-backend
```

It prints one JSON line, `{"port":...,"token":"..."}`, and serves the
authenticated WebSocket on that port. It reviews the dirty tree of the
repository a client opens (`OpenSession{repo}`: any folder inside it, a
relative path resolves against the backend's working directory), keeps its state in the per-platform
data directory (`~/.local/share/vet` on Linux) and pushes `SnapshotSettled` and
`ReviewChanged` events. `vet-mock` stays for the shell benchmark.

## UI

```sh
cd ui
pnpm install
pnpm dev        # http://localhost:1420
pnpm check      # svelte-check
pnpm lint       # eslint, incl. the architecture boundaries of ui/ARCHITECTURE.md (lint:fix)
pnpm format     # prettier (format:check in CI)
pnpm test       # keybinding tests
pnpm build
```

Outside Tauri, pass the backend's address and the repository in the URL hash:
`http://localhost:1420/#port=<port>&token=<token>&repo=<path>`. Without `repo`
the start screen asks for a path.

## Tauri shell

```sh
cd ui
pnpm tauri dev
```

On NixOS, the GTK and webkit libraries come from `shell.nix`:
`nix-shell --run "cd ui && pnpm tauri dev"`.

The shell spawns the backend (`VET_BACKEND`, default
`target/{debug,release}/vet-backend`; `vet-mock` when `VET_BENCH=1`) and kills it
on exit; the backend also exits when the shell dies (`--exit-on-stdin-close`).
`pnpm backend` (run by `pnpm tauri dev`) builds it.

The repository comes from the command line or `VET_REPO`:
`pnpm tauri dev -- -- /path/to/repo`. Without one the start screen offers
"Open repository…" (a native folder dialog) and the recent repositories, which
the shell keeps in its app config directory. "Open repository…" is also in the
header's project menu.

### WSL split

Shell on Windows, backend in WSL (§4.1). In WSL, with the repo on the Linux
filesystem (not `/mnt/*`):

```sh
cargo build -p vet-backend
```

(For the benchmark: `cargo build -p vet-mock && cargo run -p vet-mock -- --prepare`,
and `VET_BENCH=1` with `VET_BACKEND` pointing at `vet-mock`.)

Then in PowerShell on Windows, from `ui\` of a Windows checkout of the same
revision (the protocol must match):

```powershell
$env:VET_WSL_DISTRO = "NixOS"                                     # wsl -l -q
$env:VET_BACKEND = "/home/nixos/tmp/vet/target/debug/vet-backend" # Linux path inside WSL
pnpm tauri dev
```

The host runs the backend through a login shell (`wsl.exe --exec /bin/sh -lc`,
so PATH includes e.g. NixOS's per-user profile) and connects over WSL2 localhost
forwarding. In the split, pass the repository as a Linux path (`VET_REPO` or the
argument) or pick it under `\\wsl.localhost\<distro>\…` in the folder dialog; the
shell translates it and refuses folders outside that distro and Windows drives.
With `VET_WSL_DISTRO` set, nothing is built on Windows. If the shell
cannot connect, check that `localhostForwarding` is not disabled in
`%UserProfile%\.wslconfig`.

The shell spike (§12.0: corpus, benchmark, input checklist) is documented in
[notes/shell-spike.md](notes/shell-spike.md).
