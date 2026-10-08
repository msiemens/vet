# Tauri shell

```sh
cd ui
pnpm tauri dev
```

On NixOS, the GTK and webkit libraries come from `shell.nix`:
`nix-shell --run "cd ui && pnpm tauri dev"`.

The shell spawns the backend (`VET_BACKEND`, default
`target/{debug,release}/vet-backend`; `vet-mock` when `VET_BENCH` is set) and kills it
on exit; the backend also exits when the shell dies (`--exit-on-stdin-close`).
`pnpm backend` (run by `pnpm tauri dev`) builds it.

The repository comes from the command line or `VET_REPO`:
`pnpm tauri dev -- -- /path/to/repo`. Without one the welcome screen offers
"Open…" (a native folder dialog) and the recent projects, which the backend keeps
in its store.

The benchmark: `VET_BENCH=1` runs it on startup and quits with the verdict;
`VET_BENCH=manual` opens its corpus and waits for "Run benchmark" in the Dev panel
(the footer's "Dev"), e.g. to record a profile first. `VET_DEVTOOLS=1` opens the
webview's inspector with the window (debug builds only):
`VET_DEVTOOLS=1 VET_BENCH=manual pnpm tauri dev`.

## Log

The shell and the backend write one log: a daily file `vet.log.<date>` in the
`logs` folder of Vet's data directory, five days kept (macOS
`~/Library/Application Support/vet/logs`, Windows `%APPDATA%\vet\logs`, Linux and
the backend in WSL `~/.local/share/vet/logs`). Debug builds print it to stderr too.
Errors of the UI and failed requests land there under the target `ui`.
Help > Show Log opens the folder; with the backend in WSL that is the folder in
the distro, while the shell's own lines are in the Windows folder.

`VET_LOG` sets the level in `RUST_LOG` syntax, `info` by default:
`VET_LOG=debug`, `VET_LOG=vet_infra=debug,info`. `VET_LOG=timing=debug,info` adds
how long each request from the UI, each language-server start and each
language-server symbol request took (with how much of it went to making the
server see the file, including the wait after opening it). Tokens, credentials
and file contents are not logged at `info` or above. The backend reads
`VET_LOG` from its own environment; in WSL list it in `WSLENV`.

## WSL split

Shell on Windows, backend in WSL (SPEC §4.1). In WSL, with the repo on the Linux
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

The host runs the backend through a login shell (`wsl.exe --exec /bin/sh -lc`, so
PATH includes e.g. NixOS's per-user profile) and connects over WSL2 localhost
forwarding. In the split, pass the repository as a Linux path (`VET_REPO` or the
argument) or pick it under `\\wsl.localhost\<distro>\…` in the folder dialog; the
shell translates it and refuses folders outside that distro and Windows drives.
With `VET_WSL_DISTRO` set, nothing is built on Windows. If the shell cannot
connect, check that `localhostForwarding` is not disabled in
`%UserProfile%\.wslconfig`.
