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
./ci/check-deps.sh          # enforces the dependency direction between crates
```

`ci/check-deps.sh` fails if a crate depends on one outside its allowlist, or if
a pure crate pulls in an async runtime, git or IO.

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
authenticated WebSocket on that port.

## UI

```sh
cd ui
pnpm install
pnpm dev        # http://localhost:1420
pnpm check      # svelte-check
pnpm test       # keybinding tests
pnpm build
```

Outside Tauri, pass the backend's address in the URL hash:
`http://localhost:1420/#port=<port>&token=<token>`.

## Tauri shell

```sh
cd ui
pnpm tauri dev
```

On NixOS, the GTK and webkit libraries come from `shell.nix`:
`nix-shell --run "cd ui && pnpm tauri dev"`.

The shell spawns the backend (`VET_BACKEND`, default
`target/{debug,release}/vet-mock`) and kills it on exit; the backend also exits
when the shell dies (`--exit-on-stdin-close`).

### WSL split

Shell on Windows, backend in WSL (§4.1). In WSL, with the repo on the Linux
filesystem (not `/mnt/*`):

```sh
cargo build -p vet-mock && cargo run -p vet-mock -- --prepare
```

Then in PowerShell on Windows, from `ui\` of a Windows checkout of the same
revision (the protocol must match):

```powershell
$env:VET_WSL_DISTRO = "NixOS"                                     # wsl -l -q
$env:VET_BACKEND = "/home/nixos/tmp/vet/target/debug/vet-mock"    # Linux path inside WSL
pnpm tauri dev
```

The host runs the backend through a login shell (`wsl.exe --exec /bin/sh -lc`,
so PATH includes e.g. NixOS's per-user profile) and connects over WSL2 localhost
forwarding. With `VET_WSL_DISTRO` set, nothing is built on Windows. If the shell
cannot connect, check that `localhostForwarding` is not disabled in
`%UserProfile%\.wslconfig`.

The shell spike (§12.0: corpus, benchmark, input checklist) is documented in
[notes/shell-spike.md](notes/shell-spike.md).
