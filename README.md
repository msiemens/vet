# Vet

A review-first IDE for the agentic coding age. See [SPEC.md](SPEC.md); the POC's
progress is in [notes/poc-plan.md](notes/poc-plan.md).

## Layout

| Path | What |
|---|---|
| `crates/vet-core` | Pure domain model: snapshots, anchors, review state |
| `crates/vet-protocol` | Shell ↔ backend wire types; generates the TS types |
| `crates/vet-transport` | Authenticated loopback WebSocket server |
| `crates/vet-semantic` | Server pool, queries, derived analyses (depends only on `vet-core`) |
| `crates/vet-app` | Use cases and ports |
| `crates/vet-infra` | Adapters (git, watcher, store, LSP, ...) |
| `crates/vet-backend` | Composition root, the `vet-backend` binary |
| `ui/` | Svelte + CodeMirror shell ([README](ui/README.md)) |
| `ui/src-tauri/` | Tauri host, outside the Cargo workspace ([README](ui/src-tauri/README.md)) |
| `ci/` | Checks and browser tooling ([README](ci/README.md)) |

## Build, check, run

```sh
cargo build --workspace
./ci/check.sh            # all checks; ./ci/quick.sh for what changed
cd ui && pnpm install && pnpm tauri dev
```

The backend keeps its state in the per-platform data directory
(`~/.local/share/vet` on Linux). Running the shell on Windows with the backend in
WSL is described in [ui/src-tauri/README.md](ui/src-tauri/README.md#wsl-split).
