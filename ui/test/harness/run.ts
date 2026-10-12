// The invariant harness: runs every scripted session against a real backend and fails on a difference
// between what the stores hold and what the backend says that `known.ts` does not list, and on a listed
// one that did not happen. `pnpm harness [session…]` runs the named sessions only.
//
// The backend is `target/debug/vet-backend` (`VET_BACKEND` names another); build it first.

import { spawn } from "node:child_process";
import { known } from "./known.ts";
import { sessions } from "./sessions.ts";
import type { StepResult } from "./harness.ts";

function runInProcess(name: string): Promise<StepResult[]> {
  const args = [
    "--import",
    new URL("./register.mjs", import.meta.url).pathname,
    new URL("./session-runner.ts", import.meta.url).pathname,
    name,
  ];
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, { stdio: ["ignore", "pipe", "inherit"] });
    let out = "";
    child.stdout.on("data", (chunk: Buffer) => (out += chunk.toString()));
    child.on("exit", (code) => {
      const line = out.split("\n").find((l) => l.startsWith("RESULT "));
      if (code === 0 && line) resolve(JSON.parse(line.slice("RESULT ".length)) as StepResult[]);
      else reject(new Error(`session ${name} failed (exit ${code}):\n${out}`));
    });
  });
}

const requested = process.argv.slice(2);
const names = requested.length ? requested : Object.keys(sessions);
const matched = new Set<number>();
const unexpected: string[] = [];
const started = Date.now();

/** The violations of a session that no entry of `known.ts` explains; the entries that explain one are marked as seen. */
function judge(name: string, steps: StepResult[]): string[] {
  const found: string[] = [];
  for (const step of steps) {
    for (const v of step.violations) {
      const index = known.findIndex((k) => k.invariant === v.invariant && k.view === v.view);
      if (index >= 0) matched.add(index);
      else {
        found.push(
          `${name} / ${step.name}: ${v.invariant} (${v.view} view), ${v.what}\n    window:  ${JSON.stringify(v.ui)}\n    backend: ${JSON.stringify(v.backend)}`,
        );
      }
    }
  }
  return found;
}

for (const name of names) {
  process.stderr.write(`session ${name}\n`);
  unexpected.push(...judge(name, await runInProcess(name)));
}

const seconds = ((Date.now() - started) / 1000).toFixed(1);
// A partial run cannot tell whether an entry is stale.
const stale = requested.length ? [] : known.filter((_, i) => !matched.has(i));
for (const message of unexpected) console.error(`UNEXPECTED ${message}`);
for (const k of stale) {
  console.error(`STALE ${k.bug}: ${k.invariant} (${k.view} view) did not happen; delete it from known.ts`);
}
console.log(
  `${names.length} sessions in ${seconds}s: ${unexpected.length} unexpected, ${stale.length} stale, ${matched.size} known bugs seen`,
);
process.exit(unexpected.length || stale.length ? 1 : 0);
