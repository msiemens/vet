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
  /** Waits until `js` evaluates to something true in the page. */
  until(js: string, what: string): Promise<void>;
  /** Presses and releases a key. */
  press(key: string, code: string): Promise<void>;
  /** Clicks the middle of the element `selector` names. */
  click(selector: string): Promise<void>;
  /** Another window of the same project marks or flags a hunk of the changeset. */
  otherMark(path: string, hunk: number): Promise<unknown>;
  otherFlag(path: string, hunk: number): Promise<unknown>;
  /**
   * Runs `run`, waits until the page has settled, then compares what the views drew with the backend.
   * The step also fails unless the page sent `marks` review requests (default none), the backend answered
   * every request, and the page shows no notice.
   * With `stale` the views are expected to differ (they were not told of a change): the step fails if they do not.
   */
  step(name: string, run: () => Promise<unknown>, expected?: { marks?: number; stale?: boolean }): Promise<void>;
  /** Double-clicks the row of `path` in the file tree and waits for its text. */
  openText(path: string): Promise<void>;
  /** Another window comments on the changeset `count` times, each comment long enough to fill a side panel. */
  manyThreads(count: number): Promise<void>;
  /** An agent edits a file of the working tree; resolves once the page has a version for it. */
  edit(path: string, tag: string, edited: number[]): Promise<void>;
  /** A push to the branch on view: a commit of `path`; resolves once the page has a new revision. */
  pushBranch(path: string, tag: string, edited: number[]): Promise<void>;
  /** An agent moves `HEAD` to `ref` and keeps the files; resolves once the page says it left the base. */
  leaveBase(ref: string): Promise<void>;
  /** The working tree is committed and the page opens the branch, reviewed by its four commits. */
  openBranch(): Promise<void>;
  /** Where the active file tab's first line in view below the pinned header is, as `lineAt` of the probe. */
  probe(): Promise<Probe>;
  /** The errors the page logged or threw since the session started, oldest first. */
  errors(): Promise<string[]>;
}

/** The first line whose number is in view below the pinned header, and where its top is below the pane's top. */
interface Probe {
  line: number;
  y: number;
  /** The hunk bar pinned under the file's header, if one is. */
  bar: string | null;
}

/** Below the pinned header and hunk bar. */
const PROBE_Y = 100;

/** A step that sends one review request. */
const ONE = { marks: 1 };

const call = (method: string, path: string, hunk?: number) =>
  `__t.changeset.${method}(__t.changeset.currentShown(), ${JSON.stringify(path)}${hunk === undefined ? "" : `, ${hunk}`})`;

const sessions: Record<string, (d: Driver) => Promise<void>> = {
  /** The Changes tab and a file's text follow marks made here and in another window. */
  async views(d) {
    await d.step("opened", () => Promise.resolve());
    await d.step("hunk seen", () => d.page(call("toggleHunk", "b.txt", 0)), ONE);
    await d.step("hunk flagged", () => d.page(call("toggleFlag", "b.txt", 1)), ONE);
    await d.step("file seen", () => d.page(call("toggleFileSeen", "b.txt")), ONE);
    await d.step("hunk unseen", () => d.page(call("toggleHunk", "b.txt", 0)), ONE);
    await d.step("text of a file opened", () => d.openText("b.txt"));
    await d.step("hunk seen with the text open", () => d.page(call("toggleHunk", "b.txt", 1)), ONE);
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
    await d.step("hunk seen after reconnecting", () => d.page(call("toggleHunk", "b.txt", 1)), ONE);
  },

  /** A new version changes lines that were seen: the editor draws them unseen, not as they were. */
  async push(d) {
    await d.step("a file seen", () => d.page(call("toggleFileSeen", "b.txt")), ONE);
    await d.step("a version that changes its lines", () => d.edit("b.txt", "p1", [5, 7, 25]));
  },

  /** A push to the branch on view changes lines that were seen: the editor draws them unseen. */
  async pushed(d) {
    await d.openBranch();
    await d.openText("a.txt");
    await d.step("a file seen", () => d.page(call("toggleFileSeen", "a.txt")), ONE);
    await d.step("the branch is pushed to", () => d.pushBranch("a.txt", "p1", [2, 10]));
  },

  /** A change that no notification announced is still drawn once the view is looked at again. */
  async healing(d) {
    await d.step("opened", () => Promise.resolve());
    await d.page(`(() => { const l = __t.changeset.cells.listeners; window.__muted = new Set(l); l.clear(); })()`);
    await d.step("another window marks while the views are not told", () => d.otherMark("b.txt", 0), { stale: true });
    await d.page(`window.__muted.forEach((l) => __t.changeset.cells.listeners.add(l))`);
    await d.step("the view is scrolled", () =>
      d.page(`document.querySelector(".pane.active .scroll").dispatchEvent(new Event("scroll"))`),
    );
  },

  /** An "N unchanged lines" bar expands on click without an error from the editor's update cycle. */
  async collapse(d) {
    await d.step("a collapsed region is expanded", async () => {
      await d.until(`document.querySelector(".pane.active .cm-collapsedLines") !== null`, "a collapsed region");
      await d.page(
        `(() => { window.__bar = document.querySelector(".pane.active .cm-collapsedLines"); return true; })()`,
      );
      const before = (await d.errors()).length;
      await d.click(".pane.active .cm-collapsedLines");
      await sleep(200);
      const gone = (await d.page(`window.__bar.isConnected !== true`)) as boolean;
      const thrown = (await d.errors()).slice(before);
      if (thrown.length) throw new Error(`expanding a collapsed region threw: ${thrown.join("; ")}`);
      if (!gone) throw new Error("the collapsed region did not expand");
      await d.page(`(() => { delete window.__bar; return true; })()`);
    });
    await d.step("a collapsed region is expanded with the sides side by side", async () => {
      await d.page(`__t.commands.run("view.mode.split")`);
      await d.until(`document.querySelector(".pane.active .cm-mergeView") !== null`, "the side by side view");
      await d.until(`document.querySelector(".pane.active .cm-collapsedLines") !== null`, "a collapsed region");
      await d.page(
        `(() => { window.__bar = document.querySelector(".pane.active .cm-collapsedLines"); return true; })()`,
      );
      const before = (await d.errors()).length;
      await d.click(".pane.active .cm-collapsedLines");
      await sleep(200);
      const gone = (await d.page(`window.__bar.isConnected !== true`)) as boolean;
      const thrown = (await d.errors()).slice(before);
      if (thrown.length) throw new Error(`expanding a collapsed region threw: ${thrown.join("; ")}`);
      if (!gone) throw new Error("the collapsed region did not expand");
      await d.page(`(() => { delete window.__bar; return true; })()`);
    });
  },

  /** Search Everywhere gives the focus back to where it was after Escape and a choice, never after a click elsewhere. */
  async palette(d) {
    await d.page(`(() => {
      for (const [id, left] of [["probe-a", 4], ["probe-b", 220]]) {
        const field = document.createElement("input");
        field.id = id;
        field.style.cssText = "position: fixed; bottom: 4px; left: " + left + "px; z-index: 1;";
        document.body.append(field);
      }
    })()`);
    const open = async (command: string) => {
      await d.page(
        `(() => { document.getElementById("probe-a").focus(); __t.commands.run(${JSON.stringify(command)}); })()`,
      );
      await d.until(`document.activeElement?.closest(".popup") !== null`, "the popup to take the focus");
    };
    const closed = (focus: string) =>
      d.until(
        `document.querySelector(".popup") === null && document.activeElement?.id === ${JSON.stringify(focus)}`,
        `the popup closed with the focus on ${focus}`,
      );
    await d.step("Escape gives the focus back", async () => {
      await open("palette.all");
      await d.press("Escape", "Escape");
      await closed("probe-a");
    });
    await d.step("a choice gives the focus back before it is carried out", async () => {
      await open("palette.actions");
      await d.page(`(() => {
        const field = document.querySelector(".popup input");
        field.value = "Soft Wrap";
        field.dispatchEvent(new Event("input", { bubbles: true }));
      })()`);
      await d.until(`document.querySelector(".popup .r")?.textContent.includes("Soft Wrap")`, "the action to be found");
      await d.press("Enter", "Enter");
      await closed("probe-a");
      await d.page(`__t.commands.run("view.softWrap")`);
    });
    await d.step("a click elsewhere leaves the focus where it went", async () => {
      await open("palette.all");
      await d.click("#probe-b");
      await closed("probe-b");
    });
  },

  /** A file's tab keeps its place, to the pixel, across All changes, a commit and a push range. */
  async place(d) {
    const scrolledTo = async () => {
      // A stretch of the file without a change in any view, at a position that is no multiple of a line.
      await d.page(`document.querySelector(".pane.active .scroll").scrollTop = 487`);
      await sleep(500);
      const at = await d.probe();
      process.stderr.write(`  scrolled to ${JSON.stringify(at)}\n`);
      return at;
    };
    const keeps = async (step: string, before: Probe, run: () => Promise<unknown>) => {
      await d.step(step, run);
      const after = await d.probe();
      process.stderr.write(`  ${step}: was at ${JSON.stringify(before)}, is at ${JSON.stringify(after)}\n`);
      if (after.line !== before.line || Math.abs(after.y - before.y) > 1.5 || after.bar !== before.bar)
        throw new Error(`${step}: the file tab was at ${JSON.stringify(before)} and is at ${JSON.stringify(after)}`);
    };
    const show = async (mode: string) => {
      await d.page(`__t.commands.run("view.mode.${mode}")`);
      await sleep(500);
    };

    await d.openText("a.txt");
    await show("unified");
    // A review's first version is made by its first mark.
    await d.step("a hunk is seen", () => d.page(call("toggleHunk", "b.txt", 0)), ONE);
    await d.step("the timeline is open", () => d.page(`__t.commands.run("view.tool.timeline")`));
    await d.edit("a.txt", "agent", [2, 10, 18, 30, 35]);
    const all = await scrolledTo();
    await keeps("a push is chosen", all, () => d.page(`__t.changeset.setRange({ from: 1, to: 2 })`));
    await keeps("all changes again", all, () => d.page(`__t.changeset.showWhole()`));

    await d.openBranch();
    await d.openText("a.txt");
    await show("unified");
    const whole = await scrolledTo();
    await keeps("a commit is shown", whole, () => d.page(`__t.changeset.showLens("perCommit")`));
    await keeps("all changes after the commit", whole, () => d.page(`__t.changeset.showLens("squash")`));
  },

  /** Panels fuller than the window scroll inside themselves: the page never does (checked after every step). */
  async layout(d) {
    await d.manyThreads(40);
    for (const tool of ["all", "commits", "comments", "changes", "symbol"])
      await d.step(`the ${tool} tool window is open`, () => d.page(`__t.commands.run("view.tool.${tool}")`));
    await d.step("the timeline is open", () => d.page(`__t.commands.run("view.tool.timeline")`));
    await d.step("a push is chosen: the scope chip is narrowed", async () => {
      await d.edit("b.txt", "agent", [7, 8, 25]);
      await d.page(`__t.changeset.setRange({ from: 1, to: 2 })`);
    });
    await d.step("HEAD left the base: the banner and the base note show", () => d.leaveBase("main"));
  },
};

interface StepResult {
  step: string;
  differences: Difference[];
}

async function runSession(
  tab: Tab,
  other: Backend,
  fixture: Fixture,
  url: string,
  script: (d: Driver) => Promise<void>,
) {
  await tab.navigate(url);
  const page = (js: string) => tab.evaluate(`(async () => { return await (${js}); })()`);
  const ready = async (js: string) => (await page(js)) as boolean;
  await until(() => ready(`!!document.querySelector(".review")`), "the app");
  await page(`(async () => {
    const [changeset, session, commands, versions, notices] = await Promise.all([
      import("/src/state/changeset.svelte.ts"),
      import("/src/state/session.svelte.ts"),
      import("/src/state/commands.ts"),
      import("/src/state/versions.svelte.ts"),
      import("/src/state/notices.svelte.ts"),
    ]);
    window.__t = {
      changeset: changeset.changeset,
      session: session.session,
      notices: notices.notices,
      commands: commands.commands,
      versions: versions.versions,
    };
    // What the page asked of the backend: the review requests it sent, and the requests that were rejected.
    const marks = new Set(["toggleHunk", "toggleFlag", "toggleItem", "toggleFileSeen", "setFilesSeen", "createThread"]);
    const conn = session.session.backend.conn;
    const request = conn.request.bind(conn);
    window.__requests = { marks: 0, failed: [] };
    conn.request = (r, signal) => {
      if (marks.has(r.type)) window.__requests.marks++;
      const answer = request(r, signal);
      answer.catch((error) => {
        if (!signal?.aborted) window.__requests.failed.push(r.type + ": " + (error?.message ?? error));
      });
      return answer;
    };
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
    until: (js, what) => until(() => ready(js), what),
    async press(key, code) {
      await tab.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key, code });
      await tab.send("Input.dispatchKeyEvent", { type: "keyUp", key, code });
    },
    async click(selector) {
      const at = (await page(`(() => {
        const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
      })()`)) as { x: number; y: number };
      await tab.mouse("mouseMoved", at.x, at.y);
      await tab.mouse("mousePressed", at.x, at.y, { button: "left", clickCount: 1 });
      await tab.mouse("mouseReleased", at.x, at.y, { button: "left", clickCount: 1 });
    },
    async otherMark(path, hunk) {
      return other.toggleHunk((await other.manifest(CHANGESET)).revision, path, hunk, CHANGESET);
    },
    async otherFlag(path, hunk) {
      return other.toggleFlag((await other.manifest(CHANGESET)).revision, path, hunk, CHANGESET);
    },
    async step(name, run, expected = {}) {
      await page(`(() => { window.__requests.marks = 0; window.__requests.failed = []; __t.notices.close(); })()`);
      await run();
      await settle();
      const found = await check();
      const { marks, failed, notice } = (await page(`({ ...window.__requests, notice: __t.notices.text })`)) as {
        marks: number;
        failed: string[];
        notice?: string;
      };
      const problems: Difference[] = [];
      const wantMarks = expected.marks ?? 0;
      if (marks !== wantMarks)
        problems.push({ what: "the number of review requests the step sent", page: marks, backend: wantMarks });
      if (failed.length) problems.push({ what: "requests the backend rejected", page: failed, backend: [] });
      if (notice !== undefined) problems.push({ what: "the notice after the step", page: notice, backend: undefined });
      const overflow = (await page(
        `document.scrollingElement.scrollHeight - document.scrollingElement.clientHeight`,
      )) as number;
      if (overflow > 0) problems.push({ what: "how far the page scrolls (px)", page: overflow, backend: 0 });
      if (expected.stale) {
        if (!found.length)
          throw new Error(`${name}: the views followed a change they were not told of; the test cannot fail`);
        process.stderr.write(`  ${name}: stale, as wanted\n`);
        results.push({ step: name, differences: problems });
        return;
      }
      const differences = [...found, ...problems];
      process.stderr.write(`  ${name}: ${differences.length ? `${differences.length} differences` : "ok"}\n`);
      results.push({ step: name, differences });
    },
    async manyThreads(count) {
      const body = Array.from({ length: 12 }, (_, i) => `Line ${i} of a long comment that wraps in a narrow panel.`);
      for (let i = 0; i < count; i++)
        await other.createThread(
          (await other.manifest(CHANGESET)).revision,
          { type: "changeset" },
          `Comment ${i}\n\n${body.join(" ")}`,
          CHANGESET,
        );
    },
    async edit(path, tag, edited) {
      fixture.write(path, tag, edited);
      await until(() => ready(`__t.versions.items.length >= 2`), "a version for the edit");
    },
    async pushBranch(path, tag, edited) {
      const before = (await page(`__t.changeset.layout?.shown.revision ?? -1`)) as number;
      fixture.push(path, tag, edited);
      await until(() => ready(`(__t.changeset.layout?.shown.revision ?? -1) > ${before}`), "a revision for the push");
    },
    async leaveBase(ref) {
      fixture.resetHeadTo(ref);
      await until(
        () => ready(`document.body.innerText.includes("no longer builds on the review")`),
        "the banner for a HEAD off the base",
      );
    },
    async openBranch() {
      fixture.commitWorkingTree();
      await page(`__t.changeset.openChangeset({ type: "branch", name: "feature" })`);
      await until(() => ready(`__t.changeset.facts?.commits === 4`), "the branch's commits");
    },
    async probe() {
      return (await page(`(() => {
        const scroller = document.querySelector(".pane.active .scroll");
        const top = scroller.getBoundingClientRect().top;
        const pinned = scroller.querySelector(".vet-pin-bars");
        const content = document.createRange();
        if (pinned) content.selectNodeContents(pinned);
        const at = content.getBoundingClientRect();
        const bar = pinned && pinned.textContent.trim() + "@" + Math.round(at.left) + "," + Math.round(at.top - top);
        for (const el of scroller.querySelectorAll(".cm-lineNumbers .cm-gutterElement")) {
          const y = el.getBoundingClientRect().top - top;
          if (/^[0-9]+$/.test(el.textContent) && y >= ${PROBE_Y}) return { line: Number(el.textContent), y, bar };
        }
      })()`)) as Probe;
    },
    errors() {
      const logged = tab.console.filter((c) => c.type === "error").map((c) => c.text);
      return Promise.resolve([...tab.exceptions, ...logged]);
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
          (tab) => runSession(tab, other, fixture, `http://localhost:${vitePort}/#${hash}`, sessions[name]),
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
