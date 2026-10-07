// Builds the backend the Tauri shell spawns, so a plain `pnpm tauri dev` works with no
// manual steps. Run from `ui/` by tauri.conf.json's before*Command.
//
// Default: `vet-backend`. With VET_BENCH=1 or VET_BENCH=manual the shell spawns the
// spike's `vet-mock` instead (the benchmark needs its corpus), so that is built, and the
// corpus fetched.
//
// With VET_WSL_DISTRO set the backend is built by hand inside WSL (see "WSL split" in
// the README), so this does nothing.

import { spawnSync } from "node:child_process";

if (process.env.VET_WSL_DISTRO) {
  console.log("VET_WSL_DISTRO is set: build the backend inside WSL (see 'WSL split' in the README); skipping.");
  process.exit(0);
}

const release = process.argv.includes("--release") ? ["--release"] : [];
/** @param {string[]} args */
const cargo = (args) => {
  const result = spawnSync("cargo", args, { cwd: new URL("../..", import.meta.url), stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
};

if (process.env.VET_BENCH === "1" || process.env.VET_BENCH === "manual") {
  cargo(["build", ...release, "-p", "vet-mock"]);
  // First run shallow-fetches the default corpus into .corpus/ (needs git and network).
  cargo(["run", "-q", ...release, "-p", "vet-mock", "--", "--prepare"]);
} else {
  cargo(["build", ...release, "-p", "vet-backend"]);
}
