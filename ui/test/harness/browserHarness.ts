// The invariant harness, tier 2, on Playwright Test: after each step of a scripted session, what the
// editor views drew (`window.__vet.rendered()`) is compared with a fresh read of the backend
// (invariant I8). The steps and the comparison are the same as ever; the page is driven with
// Playwright's API and its waits, which retry until the predicate holds.

import type { Rendered } from "../../src/features/review/editor/rendered.ts";
import type { View } from "../../src/generated/View.ts";
import { CHANGESET } from "../../src/state/view.ts";
import type { BrowserApp } from "./browserApp.ts";
import { type Difference, compareText, compareView } from "./rendered.ts";

/** What the session scripts act on: the page, the backend as another window, and the checks. */
export interface Driver {
  /** Evaluates `js` in the page (a promise is awaited). */
  page(js: string): Promise<unknown>;
  /** Waits until `js` evaluates to something true in the page. */
  until(js: string, what: string): Promise<void>;
  /** Presses and releases a key. */
  press(key: string): Promise<void>;
  /** Clicks the element `selector` names. */
  click(selector: string): Promise<void>;
  /** Waits `ms`; for steps built around a deliberate pause rather than a condition. */
  sleep(ms: number): Promise<void>;
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
  /** Writes `count` new files into the working tree and waits until the page has them in the changeset. */
  manyFiles(count: number): Promise<void>;
  /** Another window comments on the changeset `count` times, each comment long enough to fill a side panel. */
  manyThreads(count: number): Promise<void>;
  /** An agent edits a file of the working tree; resolves once the page has a version for it. */
  edit(path: string, tag: string, edited: number[]): Promise<void>;
  /** An agent edits a file of the working tree; resolves once the page has a newer revision. */
  revise(path: string, tag: string, edited: number[]): Promise<void>;
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
export interface Probe {
  line: number;
  y: number;
  /** The hunk bar pinned under the file's header, if one is. */
  bar: string | null;
}

/** Below the pinned header and hunk bar. */
const PROBE_Y = 100;

interface StepResult {
  step: string;
  differences: Difference[];
}

/** The page-side globals the harness sets, read back with a type. */
interface HarnessWindow {
  __settle?: { at: number; snap: string };
  __settleStart: number;
  __vet: { rendered(): Rendered };
}

export async function runBrowserSession(
  app: BrowserApp,
  name: string,
  script: (d: Driver) => Promise<void>,
  onQueue = false,
): Promise<void> {
  const { page, other, fixture, url } = app;
  const exceptions: string[] = [];
  const logged: string[] = [];
  page.on("pageerror", (error) => exceptions.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") logged.push(message.text());
  });

  // Counts the requests that reach the backend from the very first one, before the app can send any.
  await page.addInitScript(`(() => {
    const wire = { fileContents: 0, fileText: 0, itemBlob: 0, targetLine: 0, listDirectory: 0, filePaths: [] };
    window.__wire = wire;
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      try {
        const message = JSON.parse(data);
        if (message && message.type === "request" && message.request && wire[message.request.type] !== undefined) {
          wire[message.request.type]++;
          if (message.request.type === "fileContents") wire.filePaths.push(message.request.path);
        }
      } catch {}
      return send.call(this, data);
    };
  })()`);

  const evaluate = (js: string) => page.evaluate(`(async () => { return await (${js}); })()`);
  const waitFor = async (js: string, what: string): Promise<void> => {
    try {
      await page.waitForFunction(js, undefined, { polling: 100, timeout: 60_000 });
    } catch (error) {
      throw new Error(`timed out waiting for ${what}: ${error instanceof Error ? error.message : String(error)}`, {
        cause: error,
      });
    }
  };
  const until = (js: string, what: string) => waitFor(js, what);
  const read = async () => (await evaluate("window.__vet.rendered()")) as Rendered;

  await page.goto(url);
  await until(`!!document.querySelector(".review")`, "the app");
  await evaluate(`(async () => {
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
  await until(`!!__t.changeset.layout`, "the changeset");
  if (!onQueue) {
    await evaluate(`__t.commands.run("view.review")`);
    await until(`!!window.__vet && document.querySelectorAll(".cm-editor").length > 0`, "the editors");
  }

  const results: StepResult[] = [];
  // Events from another window arrive after a step ends; nothing is left to come when the views stop changing.
  const settle = async () => {
    const done = page.waitForFunction(
      () => {
        const w = window as unknown as HarnessWindow;
        const now = { at: Date.now(), snap: JSON.stringify(w.__vet.rendered()) };
        const prev = w.__settle;
        w.__settle = now;
        if (prev === undefined) return false;
        return prev.snap === now.snap && now.at - prev.at >= 100 && now.at - w.__settleStart >= 500;
      },
      undefined,
      { polling: 100, timeout: 60_000 },
    );
    await done.catch(() => {
      throw new Error("the page did not settle");
    });
  };
  const check = async (): Promise<Difference[]> => {
    const view = (await evaluate(`JSON.parse(JSON.stringify(__t.changeset.layout.shown.view))`)) as View;
    const manifest = await other.manifest(view);
    const rendered = await read();
    const found = compareView(rendered.views.changes, manifest);
    for (const [id, text] of Object.entries(rendered.texts))
      found.push(...compareText(text, id.replace(/^file:/, ""), manifest));
    return found;
  };

  const driver: Driver = {
    page: evaluate,
    until,
    async press(key) {
      await page.keyboard.press(key);
    },
    async click(selector) {
      // The selector names what `document.querySelector` would: its first match.
      await page.locator(selector).first().click();
    },
    sleep: (ms) => page.waitForTimeout(ms),
    async otherMark(path, hunk) {
      return other.toggleHunk((await other.manifest(CHANGESET)).revision, path, hunk, CHANGESET);
    },
    async otherFlag(path, hunk) {
      return other.toggleFlag((await other.manifest(CHANGESET)).revision, path, hunk, CHANGESET);
    },
    async step(stepName, run, expected = {}) {
      await evaluate(`(() => { window.__requests.marks = 0; window.__requests.failed = []; __t.notices.close(); })()`);
      await page.evaluate(() => {
        const w = window as unknown as HarnessWindow;
        w.__settleStart = Date.now();
        w.__settle = undefined;
      });
      await run();
      await settle();
      const found = await check();
      const { marks, failed, notice } = (await evaluate(`({ ...window.__requests, notice: __t.notices.text })`)) as {
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
      const overflow = (await evaluate(
        `document.scrollingElement.scrollHeight - document.scrollingElement.clientHeight`,
      )) as number;
      if (overflow > 0) problems.push({ what: "how far the page scrolls (px)", page: overflow, backend: 0 });
      if (expected.stale) {
        if (!found.length)
          throw new Error(`${stepName}: the views followed a change they were not told of; the test cannot fail`);
        process.stderr.write(`  ${stepName}: stale, as wanted\n`);
        results.push({ step: stepName, differences: problems });
        return;
      }
      const differences = [...found, ...problems];
      process.stderr.write(`  ${stepName}: ${differences.length ? `${differences.length} differences` : "ok"}\n`);
      results.push({ step: stepName, differences });
    },
    async manyFiles(count) {
      fixture.manyFiles(count);
      await until(`__t.changeset.paths.length >= ${count}`, "the many files in the changeset");
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
      await until(`__t.versions.items.length >= 2`, "a version for the edit");
    },
    async revise(path, tag, edited) {
      const before = (await evaluate(`__t.changeset.layout?.shown.revision ?? -1`)) as number;
      fixture.write(path, tag, edited);
      await until(`(__t.changeset.layout?.shown.revision ?? -1) > ${before}`, "a newer revision for the edit");
    },
    async pushBranch(path, tag, edited) {
      const before = (await evaluate(`__t.changeset.layout?.shown.revision ?? -1`)) as number;
      fixture.push(path, tag, edited);
      await until(`(__t.changeset.layout?.shown.revision ?? -1) > ${before}`, "a revision for the push");
    },
    async leaveBase(ref) {
      fixture.resetHeadTo(ref);
      await until(
        `document.body.innerText.includes("no longer builds on the review")`,
        "the banner for a HEAD off the base",
      );
    },
    async openBranch() {
      fixture.commitWorkingTree();
      await evaluate(`__t.changeset.openChangeset({ type: "branch", name: "feature" })`);
      await until(`__t.changeset.facts?.commits === 4`, "the branch's commits");
    },
    async probe() {
      return (await evaluate(`(() => {
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
      return Promise.resolve([...exceptions, ...logged]);
    },
    async openText(path) {
      await page.locator(`nav button[title^=${JSON.stringify(path)}]`).dblclick();
      await until(`Object.values(window.__vet.rendered().texts).some((t) => t.hunks)`, "the text of the file");
    },
  };

  await script(driver);
  for (const exception of exceptions) process.stderr.write(`page exception: ${exception}\n`);

  const lines: string[] = [];
  for (const { step, differences } of results) {
    for (const d of differences)
      lines.push(
        `session ${name} / ${step}: ${d.what}\n    page:    ${JSON.stringify(d.page)}\n    backend: ${JSON.stringify(d.backend)}`,
      );
  }
  if (lines.length) throw new Error(`${lines.length} difference(s):\n${lines.join("\n")}`);
}
