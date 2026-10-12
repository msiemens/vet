// The invariant harness, tier 2: the real app in a headless browser against a real backend. After each
// step of a scripted session, what the editor views drew (`window.__vet.rendered()`) is compared with a
// fresh read of the backend (invariant I8). Run it with `pnpm harness:browser [session…]`; it needs
// Chrome (`VET_CHROME` names the binary), the vite dev server and `target/debug/vet-backend`.
//
// Everything it starts is its own: a backend and its data directory for each session, a dev server and a
// browser on ports it picks, a scratch browser profile without keychain access. All are stopped at the end.

import { type ChildProcess, spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Tab, withTab } from "../../../ci/cdp.mjs";
import type { Rendered } from "../../src/features/review/editor/rendered.ts";
import type { View } from "../../src/generated/View.ts";
import { Backend } from "../../src/state/backend.ts";
import { CHANGESET } from "../../src/state/view.ts";
import { BackendProcess, Fixture, binary } from "./harness.ts";
import { type Difference, compareText, compareView } from "./rendered.ts";

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const uiDir = join(import.meta.dirname, "../..");

const freePort = () =>
  new Promise<number>((resolve) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address() as net.AddressInfo;
      server.close(() => resolve(port));
    });
  });

const chromeBinary = () =>
  process.env.VET_CHROME ??
  (process.platform === "darwin" ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" : "chromium");

async function until(condition: () => Promise<boolean>, what: string, ms = 60_000) {
  const start = Date.now();
  while (!(await condition().catch(() => false))) {
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

/** What the session scripts act on: the page, the backend as another window, and the checks. */
interface Driver {
  /** Evaluates `js` in the page (a promise is awaited). */
  page(js: string): Promise<unknown>;
  /** Another window of the same project marks or flags a hunk of the changeset. */
  otherMark(path: string, hunk: number): Promise<unknown>;
  otherFlag(path: string, hunk: number): Promise<unknown>;
  /**
   * Runs `run`, waits until the page has settled, then compares what the views drew with the backend.
   * With `stale` the views are expected to differ (they were not told of a change): the step fails if they do not.
   */
  step(name: string, run: () => Promise<unknown>, stale?: "stale"): Promise<void>;
  /** Double-clicks the row of `path` in the file tree and waits for its text. */
  openText(path: string): Promise<void>;
}

const call = (method: string, path: string, hunk?: number) =>
  `__t.changeset.${method}(__t.changeset.currentShown(), ${JSON.stringify(path)}${hunk === undefined ? "" : `, ${hunk}`})`;

const sessions: Record<string, (d: Driver) => Promise<void>> = {
  /** The Changes tab and a file's text follow marks made here and in another window. */
  async views(d) {
    await d.step("opened", () => Promise.resolve());
    await d.step("hunk seen", () => d.page(call("toggleHunk", "b.txt", 0)));
    await d.step("hunk flagged", () => d.page(call("toggleFlag", "b.txt", 1)));
    await d.step("file seen", () => d.page(call("toggleFileSeen", "b.txt")));
    await d.step("hunk unseen", () => d.page(call("toggleHunk", "b.txt", 0)));
    await d.step("text of a file opened", () => d.openText("b.txt"));
    await d.step("hunk seen with the text open", () => d.page(call("toggleHunk", "b.txt", 1)));
    await d.step("another window marks", () => d.otherMark("b.txt", 0));
    await d.step("another window flags", () => d.otherFlag("b.txt", 1));
  },

  /** The socket drops while another window marks; the views show the state read after reconnecting. */
  async reconnect(d) {
    await d.step("text of a file opened", () => d.openText("b.txt"));
    await d.step("socket dropped while another window marks", async () => {
      const back = d.page(
        `new Promise((resolve) => { const b = __t.session.backend; b.onReconnect(resolve); b.conn.socket.close(); })`,
      );
      await d.otherMark("b.txt", 0);
      await d.otherFlag("b.txt", 1);
      await back;
    });
    await d.step("hunk seen after reconnecting", () => d.page(call("toggleHunk", "b.txt", 1)));
  },

  /** A change that no notification announced is still drawn once the view is looked at again. */
  async healing(d) {
    await d.step("opened", () => Promise.resolve());
    await d.page(`(() => { window.__muted = new Set(__t.changeset.listeners); __t.changeset.listeners.clear(); })()`);
    await d.step("another window marks while the views are not told", () => d.otherMark("b.txt", 0), "stale");
    await d.page(`window.__muted.forEach((l) => __t.changeset.listeners.add(l))`);
    await d.step("the view is scrolled", () =>
      d.page(`document.querySelector(".pane.active .scroll").dispatchEvent(new Event("scroll"))`),
    );
  },
};

interface StepResult {
  step: string;
  differences: Difference[];
}

async function runSession(tab: Tab, other: Backend, url: string, script: (d: Driver) => Promise<void>) {
  await tab.navigate(url);
  const page = (js: string) => tab.evaluate(`(async () => { return await (${js}); })()`);
  const ready = async (js: string) => (await page(js)) as boolean;
  await until(() => ready(`!!document.querySelector(".review")`), "the app");
  await page(`(async () => {
    const [changeset, session, commands] = await Promise.all([
      import("/src/state/changeset.svelte.ts"),
      import("/src/state/session.svelte.ts"),
      import("/src/state/commands.ts"),
    ]);
    window.__t = { changeset: changeset.changeset, session: session.session, commands: commands.commands };
  })()`);
  await until(() => ready(`!!__t.changeset.layout`), "the changeset");
  await page(`__t.commands.run("view.review")`);
  await until(() => ready(`!!window.__vet && document.querySelectorAll(".cm-editor").length > 0`), "the editors");

  const results: StepResult[] = [];
  const read = async () => (await page(`window.__vet.rendered()`)) as Rendered;
  // Events from another window arrive after a step ends; nothing is left to come when the views stop changing.
  const settle = async () => {
    await sleep(500);
    let before = JSON.stringify(await read());
    for (let i = 0; i < 100; i++) {
      await sleep(200);
      const now = JSON.stringify(await read());
      if (now === before) return;
      before = now;
    }
    throw new Error("the page did not settle");
  };
  const check = async (): Promise<Difference[]> => {
    const view = (await page(`JSON.parse(JSON.stringify(__t.changeset.layout.shown.view))`)) as View;
    const manifest = await other.manifest(view);
    const rendered = await read();
    const found = compareView(rendered.views.changes, manifest);
    for (const [id, text] of Object.entries(rendered.texts))
      found.push(...compareText(text, id.replace(/^file:/, ""), manifest));
    return found;
  };
  const driver: Driver = {
    page,
    async otherMark(path, hunk) {
      return other.toggleHunk((await other.manifest(CHANGESET)).revision, path, hunk, CHANGESET);
    },
    async otherFlag(path, hunk) {
      return other.toggleFlag((await other.manifest(CHANGESET)).revision, path, hunk, CHANGESET);
    },
    async step(name, run, stale) {
      await run();
      await settle();
      const found = await check();
      if (stale) {
        if (!found.length)
          throw new Error(`${name}: the views followed a change they were not told of; the test cannot fail`);
        process.stderr.write(`  ${name}: stale, as wanted\n`);
        results.push({ step: name, differences: [] });
        return;
      }
      process.stderr.write(`  ${name}: ${found.length ? `${found.length} differences` : "ok"}\n`);
      results.push({ step: name, differences: found });
    },
    async openText(path) {
      const at = (await page(`(() => {
        const row = document.querySelector('nav button[title^=' + JSON.stringify(${JSON.stringify(path)}) + ']');
        const r = row.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
      })()`)) as { x: number; y: number };
      await tab.mouse("mouseMoved", at.x, at.y);
      for (const clickCount of [1, 2]) {
        await tab.mouse("mousePressed", at.x, at.y, { button: "left", clickCount });
        await tab.mouse("mouseReleased", at.x, at.y, { button: "left", clickCount });
      }
      await until(
        () => ready(`Object.values(window.__vet.rendered().texts).some((t) => t.hunks)`),
        "the text of the file",
      );
    },
  };
  await script(driver);
  for (const exception of tab.exceptions) process.stderr.write(`page exception: ${exception}\n`);
  return results;
}

/** Ends a process this harness started and resolves once it is gone. */
function stop(child: ChildProcess): Promise<void> {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) {
      resolve();
      return;
    }
    child.once("exit", () => resolve());
    child.kill();
    setTimeout(() => child.kill("SIGKILL"), 5000).unref();
  });
}

async function main() {
  const requested = process.argv.slice(2);
  const names = requested.length ? requested : Object.keys(sessions);
  for (const name of names) if (!Object.hasOwn(sessions, name)) throw new Error(`no session named ${name}`);

  const profile = mkdtempSync(join(tmpdir(), "vet-harness-browser-"));
  const children: ChildProcess[] = [];
  let failures = 0;
  try {
    const vitePort = await freePort();
    const chromePort = await freePort();
    const vite = spawn(
      join(uiDir, "node_modules/.bin/vite"),
      ["--port", String(vitePort), "--strictPort", "--host", "127.0.0.1"],
      { cwd: uiDir, env: { ...process.env, HOME: profile }, stdio: ["ignore", "ignore", "inherit"] },
    );
    children.push(vite);
    mkdirSync(join(profile, "chrome"));
    const chrome = spawn(
      chromeBinary(),
      [
        "--headless=new",
        `--remote-debugging-port=${chromePort}`,
        `--user-data-dir=${join(profile, "chrome")}`,
        "--use-mock-keychain",
        "--password-store=basic",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-gpu",
        "about:blank",
      ],
      { stdio: "ignore" },
    );
    children.push(chrome);
    process.stderr.write(`started: vite pid ${vite.pid} on ${vitePort}, chrome pid ${chrome.pid} on ${chromePort}\n`);
    await until(async () => (await fetch(`http://127.0.0.1:${vitePort}/`)).ok, "the dev server");
    await until(async () => (await fetch(`http://127.0.0.1:${chromePort}/json/version`)).ok, "the browser");

    for (const name of names) {
      process.stderr.write(`session ${name}\n`);
      const fixture = new Fixture();
      const backend = new BackendProcess(binary(), fixture.home);
      try {
        const { port, token } = await backend.address;
        const hash = `port=${port}&token=${token}&repo=${encodeURIComponent(fixture.repo)}`;
        globalThis.location = new URL(`http://localhost/#${hash}`) as never;
        const other = await Backend.connect();
        await other.open(fixture.repo);
        const results = await withTab(
          (tab) => runSession(tab, other, `http://localhost:${vitePort}/#${hash}`, sessions[name]),
          {
            port: chromePort,
          },
        );
        for (const { step, differences } of results) {
          for (const d of differences) {
            failures++;
            process.stderr.write(
              `UNEXPECTED ${name} / ${step}: ${d.what}\n    page:    ${JSON.stringify(d.page)}\n    backend: ${JSON.stringify(d.backend)}\n`,
            );
          }
        }
      } finally {
        backend.stop();
        fixture.remove();
      }
    }
  } finally {
    await Promise.all(children.map(stop));
    rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
  process.stderr.write(`${names.length} sessions: ${failures} differences\n`);
  process.exit(failures ? 1 : 0);
}

main().catch((error: unknown) => {
  console.error(error);
  process.exit(1);
});
