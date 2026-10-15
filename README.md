# Vet

Vet is a desktop app for reviewing code changes, built for the case where most of the code is written by agents and you are the one who checks it. Its unit of work is the changeset, not the file: a working tree an agent is editing, a local branch, or a pull request on GitHub or Azure DevOps.

Vet doesn't write code and doesn't drive agents. It watches what they leave behind in git and on disk, and it helps you verify that before it is merged.

## What it does

- **One queue for everything to review.** Working trees, local branches and pull requests, ordered by what blocks you, each with how much of it you have reviewed.
- **Review state that survives rewrites.** What you marked as seen is tied to the content, not to line numbers. It survives amends, rebases, force-pushes and an agent's later edits. Content that changed is never silently left marked as seen.
- **Versions.** Every settled state of a working tree, and every push to a pull request, is a version. You can review just what changed since you last looked.
- **Real code intelligence on both sides of the diff.** Go to definition, references, hover, call hierarchy, symbols and diagnostics come from the languages' own servers (rust-analyzer, TypeScript, Svelte, Angular, Pyright, Roslyn, YAML, JSON, Bash, Dockerfile), for the base as well as the changed code.
- **Comments where your team reads them.** Threads are synced both ways with GitHub and Azure DevOps, and stay anchored to the right lines across rebases.
- **Safe with code you didn't write.** Pull requests from forks, and pull requests in projects you haven't marked trusted, are analysed without running any of their code until you trust them.
- **Keyboard first.** Hunk navigation, marking seen, commenting and Search Everywhere all work from the keyboard.

The full product definition, with its invariants, is in [SPEC.md](SPEC.md).

## Status

Vet is a personal tool in active development, not released. It runs on macOS, Windows and WSL2, including a setup where the window runs on Windows and the backend in WSL.

## Building and running

You need a Rust toolchain that supports edition 2024 (1.85 or newer), Node.js and pnpm. To use code intelligence for a language, its language server has to be installed.

```sh
cargo build --workspace
cd ui && pnpm install && pnpm tauri dev
```

Vet keeps its state in the platform's data directory (`~/Library/Application Support/vet` on macOS, `~/.local/share/vet` in WSL). Running the window on Windows with the backend in WSL is described in [ui/src-tauri/README.md](ui/src-tauri/README.md#wsl-split).

## Checks

```sh
./ci/check.sh          # everything; run it before merging
./ci/quick.sh [BASE]   # only what changed since BASE
```

Both use sccache when it is installed and plain rustc otherwise. [ci/README.md](ci/README.md) lists what each one runs.

## How it is built

A Rust backend does all the work: it watches the repository, keeps review state in SQLite, talks to GitHub and Azure DevOps, and runs the language servers. A Svelte window in a Tauri app only shows what the backend tells it, over a local WebSocket.

| Path            | What                                                                                    |
| --------------- | --------------------------------------------------------------------------------------- |
| `crates/`       | The backend, a Cargo workspace; [ARCHITECTURE.md](ARCHITECTURE.md) describes its crates |
| `ui/`           | The window, Svelte and CodeMirror; [ui/ARCHITECTURE.md](ui/ARCHITECTURE.md)             |
| `ui/src-tauri/` | The Tauri app, outside the Cargo workspace ([README](ui/src-tauri/README.md))           |
| `ci/`           | The checks ([README](ci/README.md))                                                     |
| `tools/`        | The module-cycle checker the checks run                                                 |
