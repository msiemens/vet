# Tauri shell

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
the shell keeps in its app config directory.

`VET_DEVTOOLS=1` opens the webview's inspector with the window (debug builds
only): `VET_DEVTOOLS=1 VET_BENCH=1 pnpm tauri dev`.

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
