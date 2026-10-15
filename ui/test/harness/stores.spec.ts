// The tier-1 tests: one per scripted session. The stores are process-wide singletons, so the session
// runs in a child process of its own (`session-runner.ts`, with the rune loader); this side only
// judges what it found against `known.ts`. The parent never loads the stores.

import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { test } from "@playwright/test";
import type { StepResult } from "./harness.ts";
import { known } from "./known.ts";
import { sessionNames, type SessionName } from "./sessionNames.ts";

const register = fileURLToPath(new URL("./register.mjs", import.meta.url));
const runner = fileURLToPath(new URL("./session-runner.ts", import.meta.url));

/** Runs one session in a process of its own and resolves with what its steps found. */
function runInProcess(name: string): Promise<StepResult[]> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["--import", register, runner, name], {
      stdio: ["ignore", "pipe", "inherit"],
    });
    let out = "";
    child.stdout.on("data", (chunk: Buffer) => (out += chunk.toString()));
    child.on("exit", (code) => {
      const line = out.split("\n").find((l) => l.startsWith("RESULT "));
      if (code === 0 && line) resolve(JSON.parse(line.slice("RESULT ".length)) as StepResult[]);
      else reject(new Error(`session ${name} failed (exit ${code}):\n${out}`));
    });
  });
}

/**
 * The messages for a session's violations that `known.ts` does not explain, and for its entries that did
 * not happen. Each entry names the session and step it happens in, so it hides nothing else.
 */
function judge(name: SessionName, steps: StepResult[]): string[] {
  const matched = new Set<number>();
  const messages: string[] = [];
  for (const step of steps) {
    for (const violation of step.violations) {
      const index = known.findIndex(
        (k) =>
          k.session === name &&
          k.step === step.name &&
          k.invariant === violation.invariant &&
          k.view === violation.view,
      );
      if (index >= 0) matched.add(index);
      else
        messages.push(
          `UNEXPECTED ${name} / ${step.name}: ${violation.invariant} (${violation.view} view), ${violation.what}\n    window:  ${JSON.stringify(violation.ui)}\n    backend: ${JSON.stringify(violation.backend)}`,
        );
    }
  }
  known.forEach((k, index) => {
    if (k.session === name && !matched.has(index))
      messages.push(
        `STALE ${k.bug}: ${k.invariant} (${k.view} view) in ${k.session} / ${k.step} did not happen; delete it from known.ts`,
      );
  });
  return messages;
}

for (const name of sessionNames) {
  test(name, async () => {
    const messages = judge(name, await runInProcess(name));
    if (messages.length) throw new Error(`${messages.length} difference(s):\n${messages.join("\n")}`);
  });
}
