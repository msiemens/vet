# Checks and browser tooling

## Checks

```sh
./ci/check.sh            # everything; run it before merging
./ci/quick.sh [BASE]     # only what changed since BASE (default: the merge base with main)
```

`check.sh` runs rustfmt (both workspaces), clippy (`-D warnings`), tests,
`check-deps.sh`, `check-module-cycles.sh`, then in `ui/` `pnpm check`, `lint`,
`format:check`, `test` and `build`.

`quick.sh` runs rustfmt on changed files, clippy, tests and module cycles for the
changed crates and their dependents, and svelte-check, eslint, prettier and unit
tests for changed UI files.

The Rust steps on their own:

```sh
cargo clippy --workspace --all-targets
cargo fmt --all --check     # rustfmt.toml; ui/src-tauri is checked by check.sh
./ci/check-deps.sh          # dependency direction between crates
./ci/check-module-cycles.sh # module cycles inside a crate
```

`check-deps.sh` fails if a crate depends on one outside its allowlist, or if a
pure crate pulls in an async runtime, git or IO.

`check-module-cycles.sh` needs `cargo install cargo-modules --locked`. It fails if
the module graph of a library or binary has a cycle, ignoring edges to a module's
own ancestors (`use super::X`). `cargo modules --acyclic` is not used directly: it
counts every type and its methods as a cycle.

Formatting-only commits are listed in `.git-blame-ignore-revs`; enable them with
`git config blame.ignoreRevsFile .git-blame-ignore-revs`.

## Browser

Screenshots and benchmarks use one shared headless browser (Windows Edge in WSL,
else Chromium; `VET_BROWSER=linux` forces Chromium). `browser.sh` starts it if
needed and prints its CDP port (`VET_BROWSER_PORT`, default 9222); a watchdog
closes it after `VET_BROWSER_IDLE` seconds (default 600) without a page. Clients
use `cdp.mjs`, which gives each run an isolated browser context:

```sh
node ci/cdp.mjs screenshot 'http://localhost:<port>/#port=…&token=…&repo=/path' out.png [--wait ms] [--theme light|dark]
```

Scripts import `withTab(fn)` (screenshots, `evaluate`, `mouse`, console capture)
and `withBenchLock(fn)` (exclusive, so parallel benchmarks do not skew each other).

`chromium.sh [args…]` is for one-off use only: it runs its own headless Chromium,
killed after `VET_CHROMIUM_TIMEOUT` seconds (default 600) or when the script
exits, and deletes its profile.
