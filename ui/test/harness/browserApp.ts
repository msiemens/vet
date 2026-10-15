// The browser tier's Playwright fixture: a scratch repository, a real backend of its own, a second
// connection that is the "other window", and the page the app runs in.

import "./nodeGlobals.ts";
import { test as base, type Page } from "@playwright/test";
import { Backend } from "../../src/state/backend.ts";
import { BackendProcess, Fixture, binary } from "./fixture.ts";

export interface BrowserApp {
  readonly page: Page;
  readonly fixture: Fixture;
  /** A second connection to the same project: another window, and the source of fresh reads. */
  readonly other: Backend;
  /** The app's address, carrying the backend's port, token and repository in the hash. */
  readonly url: string;
}

export const test = base.extend<{ app: BrowserApp }>({
  app: async ({ page, baseURL }, use) => {
    if (!baseURL) throw new Error("the browser tier needs a baseURL");
    const fixture = new Fixture();
    const backend = new BackendProcess(binary(), fixture.home);
    try {
      const { port, token } = await backend.address;
      const hash = `port=${port}&token=${token}&repo=${encodeURIComponent(fixture.repo)}`;
      // The connection's dialer reads the hash for the backend to reach; the fixture's other
      // connection runs on this side, so it needs the location the page would have.
      globalThis.location = new URL(`http://localhost/#${hash}`) as never;
      const other = await Backend.connect();
      await other.open(fixture.repo);
      await use({ page, fixture, other, url: `${baseURL}/#${hash}` });
    } finally {
      backend.stop();
      fixture.remove();
    }
  },
});
