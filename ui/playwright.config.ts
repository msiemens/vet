// Playwright configuration for the browser tier of the invariant harness. Each scripted session is
// a test, run on WebKit (closest to the Tauri webview on macOS and Linux) and on the installed
// Chrome. The vite dev server is Playwright's; the app finds its backend in the URL hash.

import { createServer } from "node:net";
import { defineConfig } from "@playwright/test";

/** A port for the dev server, so a run never clashes with a dev server of your own. */
async function freePort(): Promise<number> {
  return await new Promise((resolve) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address() as { port: number };
      server.close(() => resolve(port));
    });
  });
}

const vitePort = Number(process.env.VET_VITE_PORT ?? 0) || (await freePort());
// Workers evaluate this config again; the choice has to be the runner's, so it is shared through the
// environment the runner sets before it starts them.
process.env.VET_VITE_PORT = String(vitePort);
const baseURL = `http://127.0.0.1:${vitePort}`;

export default defineConfig({
  testDir: "./test/harness",
  testMatch: "browser.spec.ts",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  reporter: [["list"]],
  use: {
    baseURL,
    viewport: { width: 1440, height: 900 },
    // The steps compare what the page drew with the backend, so a trace is what is needed when one fails.
    trace: "retain-on-failure",
  },
  webServer: {
    command: `node_modules/.bin/vite --port ${vitePort} --strictPort --host 127.0.0.1`,
    url: baseURL,
    cwd: import.meta.dirname,
    reuseExistingServer: false,
    timeout: 60_000,
  },
  projects: [
    { name: "webkit", use: { browserName: "webkit" } },
    { name: "chrome", use: { channel: "chrome" } },
  ],
});
