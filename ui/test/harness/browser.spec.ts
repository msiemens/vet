// The browser tier's tests: one per scripted session, on each engine the config names.

import { test } from "./browserApp.ts";
import { runBrowserSession } from "./browserHarness.ts";
import { sessions, startOnQueue } from "./browserSessions.ts";

/** Sessions known to fail, with the backlog item that tracks each: they pass once it is fixed, which fails them here. */
const KNOWN_FAILURES: Record<string, string> = {};

for (const [name, script] of Object.entries(sessions)) {
  test(name, async ({ app }) => {
    test.fail(name in KNOWN_FAILURES, KNOWN_FAILURES[name]);
    await runBrowserSession(app, name, script, startOnQueue.has(name));
  });
}
