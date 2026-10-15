# Checks and browser tooling

## Checks

```sh
./ci/check.sh            # everything; run it before merging
./ci/quick.sh [BASE]     # only what changed since BASE (default: the merge base with main)
```

`check.sh` runs rustfmt (the workspace and `ui/src-tauri`), clippy (`-D warnings`), tests (the
workspace's and the host's),
`check-deps.sh`, `check-module-cycles.sh`, then in `ui/` `pnpm check`, `lint`,
`format:check`, `test`, `harness` and `build`. After the workspace tests it fails if the
`ui/src/generated` bindings differ from what they regenerated from `vet-protocol`; `quick.sh`
does the same when `vet-protocol` changed. The module-cycle tool's own checks (rustfmt, clippy,
self-test) run only in `quick.sh`, when the tool changed.

`quick.sh` runs rustfmt on changed files, clippy, tests and module cycles for the
changed crates and their dependents, rustfmt and clippy for `ui/src-tauri` when it
changed (with its tests), rustfmt, clippy and the module-cycle self-test when `tools/module-cycles`
changed, and svelte-check, eslint, prettier and unit
tests for changed UI files.

### UI invariant harness

`pnpm harness` (in `ui/`) runs the UI's real stores, with their runes compiled by a
module hook (`ui/test/harness/loader.mjs`), under node against a real
`target/debug/vet-backend` (`VET_BACKEND` names another). Each scripted session
(`sessions.ts`) gets a process, a scratch `HOME` and data directory, and a fixture
repository of its own, and the backend is killed at the end. After every step, once the
window has been quiet, `invariants.ts` compares what the stores hold with fresh reads
through a second connection: file review states, view and changeset completion, base,
review info, the queue's on-view row, versions, commits, threads, and that every review
or comment request names the view of the element that sent it. `pnpm harness <session>…` runs some sessions.

`known.ts` lists the differences that are known bugs, by invariant and view, each with
its bug id. A difference not listed fails the run, and so does a listed one that did not
happen: fixing a bug means deleting its lines. A session with unlisted differences is
run once more, because requests sent without waiting for each other can be answered out
of order; only differences that stay fail.

`pnpm harness:browser` is the second tier, for local runs: the real app under
[Playwright Test](https://playwright.dev) (`ui/playwright.config.ts`) on the vite dev server that
Playwright starts, against a backend of its own. Each session (`browserSessions.ts`) is a test, run on
Playwright's WebKit (closest to the Tauri webview on macOS and Linux) and on the installed Chrome
(`channel: "chrome"`; WebKit needs `pnpm exec playwright install webkit`). After every step the
harness (`browserHarness.ts`) compares what the editor views drew, read through the dev build's
`window.__vet.rendered()` (header texts, cards, the editors' review fields, the scroll bar's marks, a
file's text), with a fresh read of the backend. Each test starts its own backend and fixture; a
failure names the session, the step and the difference, and keeps a trace. `pnpm exec playwright test
-g <name>` runs some sessions and `--project=webkit|chrome` one engine. It is not part of `check.sh`
or `quick.sh`.

`check.sh` builds the backend and runs the harness after the UI unit tests (about 20 s);
`quick.sh` runs it when `ui/src/state`, the harness, or the `vet-protocol`, `vet-backend`
or `vet-app` crates changed.

The Rust steps on their own:

```sh
cargo nextest run --workspace  # .config/nextest.toml
cargo clippy --workspace --all-targets
cargo fmt --all --check     # rustfmt.toml; ui/src-tauri is checked by check.sh
./ci/check-deps.sh          # dependency direction between crates
./ci/check-module-cycles.sh # module cycles inside a crate
```

Tests run with `cargo-nextest` (`cargo install cargo-nextest --locked`): each test
in its own process, test binaries in parallel. A test is reported as slow after
30 s and killed after 2 min. Tests that start real language servers or restores
form the `language-servers` group, of which at most two run at once.

Tests that need a tool that may be missing (`python3`, `rust-analyzer`,
`typescript-language-server`, `roslyn-language-server` with `dotnet`) are `#[ignore = "needs …"]`,
so a plain run lists them as skipped instead of passed. `tool-tests.sh` (run by `check.sh` and
`quick.sh` after nextest) runs the ignored tests of every tool that is installed, prints what it
leaves out for the others (and says so again in one `NOT RUN` line), and fails if an ignored test
belongs to no tool. Ignored tests that download packages or use the network (`servers`, `restore`,
`python`, `first_mark` and the benchmark in `vet-core`) are not part of any check; they run by hand
with `ci/tool-tests.sh --on-request [-p CRATE]...`, before a change to restoring, the server installs
or the python and first-mark paths is merged, and now and then besides, since registries and package
versions move under them. There is no scheduler in the repository to run them unattended.
`examples/platform_smoke.rs` of `vet-infra` reads the real GitHub and Azure DevOps with the `gh` and
`az` tokens.

A step that cannot run on the machine (the Tauri host without the webkit libraries and `nix-shell`,
`cargo-sweep`, an uninstalled tool) is printed as `SKIPPED` where it happens and listed again at the
end of `check.sh` and `quick.sh`; the scripts still pass, but the closing message counts what did not
run. The host's own tests (`ui/src-tauri`) run in `check.sh`, and in `quick.sh` when the host changed.

`check-deps.sh` fails if a crate depends on one outside its allowlist, or if a
pure crate pulls in an async runtime, git or IO.

`check-module-cycles.sh` builds and runs `tools/module-cycles`, a standalone
workspace (so Vet's builds never compile rust-analyzer) on the rust-analyzer
version of `cargo-modules` 0.27. It loads the workspace into rust-analyzer once,
without the sysroot, build scripts and proc macros, and primes the name resolution
of the dependency graph on all cores. For each library and binary it builds the
module graph that `cargo modules dependencies --no-externs --no-sysroot --no-fns
--no-types --no-traits --no-owns` reports and fails if it has a cycle, ignoring
edges to a module's own ancestors (`use super::X`). Those edges are the uses of
modules by modules: imports (also glob and `pub use`) and the types and traits
named in the fields and signatures of an item. The methods of an `impl` count for
the module declaring the implemented type. Like cargo-modules it treats items under
a `cfg` that holds when only `test` is set (such as `cfg(not(windows))`) as part of
the enclosing module, sees no `std` types (a `Vec<Foo>` does not mention `Foo`) and
sees no code generated by proc macros. `--dump-edges` prints the edges per target.
`--self-test` runs the check on `ci/fixtures/module-cycle`, which contains a cycle,
and fails if it is not reported; `quick.sh` runs it when the tool changed.

The scripts default `RUSTC_WRAPPER` to `rustc-wrapper.sh`, which runs rustc through
sccache (directly, if sccache is not installed) so that compiled dependencies are
shared between checkouts. Plain `RUSTC_WRAPPER=sccache` misses across checkouts:
crates reading `env!("OUT_DIR")` embed the checkout's build-script path, which the
wrapper replaces by a symlink with a stable name in `/tmp/sccache-out-dir-<uid>`, and
on macOS proc-macro libraries, which sccache cannot cache but hashes as inputs of
their dependents, differ per checkout by their install name, which the wrapper fixes.

Formatting-only commits are listed in `.git-blame-ignore-revs`; enable them with
`git config blame.ignoreRevsFile .git-blame-ignore-revs`.
