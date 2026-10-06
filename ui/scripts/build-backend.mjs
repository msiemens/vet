// Builds the backend the Tauri shell spawns (the spike's `vet-mock`) and fetches
// the spike's corpus, so a plain `pnpm tauri dev` works with no manual steps. Run
// from `ui/` by tauri.conf.json's before*Command.
//
// With VET_WSL_DISTRO set the backend is built and fetched inside WSL by hand (see
// "WSL split" in the README), so this does nothing.

import { spawnSync } from "node:child_process";

if (process.env.VET_WSL_DISTRO) {
  console.log("VET_WSL_DISTRO is set: build vet-mock inside WSL (see "WSL split" in the README); skipping.");
  process.exit(0);
}

const release = process.argv.includes("--release") ? ["--release"] : [];
const cargo = (args) => {
  const result = spawnSync("cargo", args, { cwd: new URL("../..", import.meta.url), stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
};

cargo(["build", ...release, "-p", "vet-mock"]);
// First run shallow-fetches the default corpus into .corpus/ (needs git and network).
cargo(["run", "-q", ...release, "-p", "vet-mock", "--", "--prepare"]);
