// Playwright configuration for tier 1 of the invariant harness. Each scripted session is a test that
// spawns the session's own child process (`session-runner.ts`, with the rune loader); the stores never
// load here. No browser, no dev server, no retries.

import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./test/harness",
  testMatch: "stores.spec.ts",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  reporter: [["list"]],
  // A session waits for its backend to settle, which can take longer than Playwright's 30 s default.
  timeout: 120_000,
});
