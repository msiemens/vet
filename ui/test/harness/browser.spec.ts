// The browser tier's tests: one per scripted session, on each engine the config names.

import { test } from "./browserApp.ts";
import { runBrowserSession } from "./browserHarness.ts";
import { sessions, startOnQueue } from "./browserSessions.ts";

for (const [name, script] of Object.entries(sessions)) {
  test(name, async ({ app }) => {
    await runBrowserSession(app, name, script, startOnQueue.has(name));
  });
}
