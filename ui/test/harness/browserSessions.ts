// The browser tier's scripted sessions: each drives the real app in a page and, after every step,
// lets the harness compare what the views drew with the backend. Each is a Playwright test.

import type { Driver, Probe } from "./browserHarness.ts";

/** The sessions whose script starts on the queue, before the review screen is shown. */
export const startOnQueue = new Set<string>([
  "hidden",
  "many",
  "allFiles",
  "reviewOnly",
  "returning",
  "fromQueue",
  "findInFiles",
  "sinceLast",
]);

/** A step that sends one review request. */
const ONE = { marks: 1 };

const call = (method: string, path: string, hunk?: number) =>
  `__t.changeset.${method}(__t.changeset.currentShown(), ${JSON.stringify(path)}${hunk === undefined ? "" : `, ${hunk}`})`;

export const sessions: Record<string, (d: Driver) => Promise<void>> = {
  /**
   * The All files tree lists the target snapshot; while the review screen is away on the queue it
   * lists nothing, a revision that settles there waits, and the tree loads when the screen comes up.
   */
  async allFiles(d) {
    await d.until(`!!__t.changeset.layout`, "the changeset");
    await d.page(`__t.commands.run("view.tool.all")`);
    await d.page(`__t.session.backend.waitForQuiet()`);
    await d.sleep(500);
    const listed = async () => (await d.page(`__wire.listDirectory`)) as number;
    if ((await listed()) !== 0)
      throw new Error("the hidden All files tree listed a directory while the queue was on screen");
    await d.revise("b.txt", "wt2", [7, 25, 33]);
    await d.page(`__t.session.backend.waitForQuiet()`);
    await d.sleep(500);
    if ((await listed()) !== 0) throw new Error("a revision while the queue was on screen listed a directory");
    await d.page(`__t.commands.run("view.review")`);
    await d.until(`__wire.listDirectory > 0`, "the All files tree to load when the review screen is shown");
  },

  /**
   * The review screen stays mounted behind the queue and loads nothing there; a revision that settles
   * while it is away waits too, and the files load when the screen comes up.
   */
  async hidden(d) {
    await d.until(`!!__t.changeset.layout`, "the changeset");
    await d.page(`__t.session.backend.waitForQuiet()`);
    await d.sleep(500);
    const loaded = async () => (await d.page(`__wire.fileContents`)) as number;
    if ((await loaded()) !== 0) throw new Error("the hidden review screen loaded files while the queue was on screen");
    await d.revise("b.txt", "wt2", [7, 25, 33]);
    await d.page(`__t.session.backend.waitForQuiet()`);
    await d.sleep(500);
    if ((await loaded()) !== 0) throw new Error("a revision while the queue was on screen loaded files");
    await d.page(`__t.commands.run("view.review")`);
    await d.until(`document.querySelectorAll(".cm-editor").length > 0`, "the editors");
    await d.until(`__wire.fileContents > 0`, "the review screen's files to load");
  },

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
      await d.sleep(200);
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
      await d.sleep(200);
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
      await d.press("Escape");
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
      await d.press("Enter");
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
      await d.sleep(500);
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
      await d.sleep(500);
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

  /**
   * All changes loads a file's contents on demand: what is on screen plus the margin around it, and
   * anything a command needs. The rest loads a few at a time while the review screen is idle. A file
   * that fills in does not move the scroll position, because its height comes from the manifest. A new
   * revision reloads what is in view and leaves the rest to the trickle.
   */
  async many(d) {
    const TOTAL = 120;
    const far = `many/f100.dat`;
    const changed = `many/f118.dat`;
    const files = () => d.page(`__t.changeset.paths.length`) as Promise<number>;
    const requests = () => d.page(`__wire.fileContents`) as Promise<number>;
    const loaded = () => d.page(`new Set(__wire.filePaths).size`) as Promise<number>;

    await d.manyFiles(TOTAL);
    await d.page(`__t.session.backend.waitForQuiet()`);
    // Nothing loads while the queue is on screen; the review screen's files wait for its return.
    if ((await requests()) !== 0) throw new Error("the hidden review screen loaded files");

    await d.page(`__t.commands.run("view.review")`);
    await d.until(`document.querySelectorAll(".pane.active .cm-editor").length > 0`, "the first editors");

    // 1. Opening loads the visible part plus the margin around it, not the whole changeset.
    const opened = (await d.page(`(() => {
      const scroller = document.querySelector(".pane.active .scroll");
      const rect = scroller.getBoundingClientRect();
      const bottom = rect.top + scroller.clientHeight;
      const margin = scroller.clientHeight * 0.5;
      let visible = 0, mounted = 0, below = 0;
      for (const s of scroller.querySelectorAll("section.file")) {
        const at = s.getBoundingClientRect();
        if (at.top < bottom && at.bottom > rect.top) visible++;
        if (s.querySelector(".cm-editor")) {
          mounted++;
          if (at.top >= bottom + margin) below++;
        }
      }
      return { requests: window.__wire.fileContents, visible, mounted, below };
    })()`)) as { requests: number; visible: number; mounted: number; below: number };
    const total = await files();
    process.stderr.write(
      `  open: ${opened.requests} requests, ${opened.visible} visible, ${opened.mounted} mounted, ${opened.below} in the lower margin\n`,
    );
    if (opened.requests === 0) throw new Error("opening the review screen loaded nothing");
    if (opened.requests >= total) throw new Error(`opening loaded every file (${opened.requests} of ${total})`);
    if (opened.below === 0) throw new Error("opening loaded nothing beyond the visible part: the margin is gone");

    // 2. A far hunk the keyboard goes to is loaded on demand, and the view lands on it.
    await d.step("a far hunk is flagged by another window", () => d.otherFlag(far, 0));
    await d.step("the keyboard goes to the far flagged hunk", async () => {
      await d.press("Alt+f");
      await d.until(
        `document.querySelector(".pane.active .scroll section.file.current-file")?.querySelector(".cm-editor") !== null`,
        "the far file's editor",
      );
    });
    const landed = (await d.page(`(() => {
      const scroller = document.querySelector(".pane.active .scroll");
      const section = scroller.querySelector("section.file.current-file");
      const at = section.getBoundingClientRect();
      const rect = scroller.getBoundingClientRect();
      return {
        path: section.querySelector(".path").textContent,
        visible: at.bottom > rect.top && at.top < rect.top + scroller.clientHeight,
      };
    })()`)) as { path: string; visible: boolean };
    if (landed.path !== far) throw new Error(`the keyboard landed on ${landed.path}, not ${far}`);
    if (!landed.visible) throw new Error("the far file is not in view after the keyboard navigation");

    // 3. Scrolling to the end loads the files there; the position and the last file stay put as they fill in.
    const before = (await d.page(`(() => {
      const scroller = document.querySelector(".pane.active .scroll");
      scroller.scrollTop = scroller.scrollHeight;
      window.__last = [...scroller.querySelectorAll("section.file")].filter((s) => s.querySelector(".path").textContent.endsWith(".dat")).at(-1);
      window.__anchor = { top: scroller.scrollTop, y: window.__last.getBoundingClientRect().top - scroller.getBoundingClientRect().top };
      return window.__anchor;
    })()`)) as { top: number; y: number };
    await d.until(`window.__last.querySelector(".cm-editor") !== null`, "the last file's text");
    const after = (await d.page(`(() => {
      const scroller = document.querySelector(".pane.active .scroll");
      return { top: scroller.scrollTop, y: window.__last.getBoundingClientRect().top - scroller.getBoundingClientRect().top };
    })()`)) as { top: number; y: number };
    process.stderr.write(`  scroll: top ${before.top} -> ${after.top}, anchor ${before.y} -> ${after.y}\n`);
    if (Math.abs(after.top - before.top) > 1)
      throw new Error(`the scroll position moved when the end filled in (${before.top} -> ${after.top})`);
    if (Math.abs(after.y - before.y) > 1.5)
      throw new Error(`the file at the end moved when it filled in (${before.y} -> ${after.y})`);

    // 4. Left idle, the trickle loads every file.
    await d.until(`new Set(__wire.filePaths).size >= ${total}`, "the trickle to reach every file");
    process.stderr.write(`  trickle: ${await loaded()} of ${total} files loaded\n`);

    // 5. A revision reloads what is in view; the rest is left to the trickle.
    let reloaded: string[] = [];
    await d.step("a revision settles", async () => {
      const at = (await d.page(`__wire.filePaths.length`)) as number;
      await d.revise(changed, "r1", [20]);
      await d.until(`__wire.filePaths.length > ${at}`, "the revision's reloads");
      await d.sleep(60);
      reloaded = (await d.page(`__wire.filePaths.slice(${at})`)) as string[];
      if (!reloaded.length) throw new Error("the revision reloaded nothing");
    });
    process.stderr.write(
      `  revision: ${reloaded.length} requests, ${reloaded.includes(changed) ? "with" : "without"} the changed file\n`,
    );
    if (!reloaded.includes(changed)) throw new Error("the revision did not reload the file in view that changed");
    if (reloaded.length >= total / 2)
      throw new Error(`the revision reloaded ${reloaded.length} of ${total} files eagerly`);
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

  /**
   * The review screen asks the backend for nothing while the queue shows: its stores hold nothing,
   * its tools and views are gone, and a revision, a server change and new checks on the queue move
   * no review-only request. Showing it again loads what it shows, current, with its tabs and view as
   * they were left.
   */
  async reviewOnly(d) {
    const REVIEW_ONLY = [
      "versions",
      "commits",
      "mapLines",
      "pipelineSteps",
      "signature",
      "calls",
      "usages",
      "reachability",
      "removalImpact",
      "fileOutline",
      "fileContents",
      "fileText",
      "listDirectory",
    ] as const;
    const counts = async () => {
      const wire = (await d.page(`window.__wire`)) as Record<string, number>;
      return Object.fromEntries(REVIEW_ONLY.map((t) => [t, wire[t] ?? 0])) as Record<string, number>;
    };
    const quiet = async () => {
      await d.page(`__t.session.backend.waitForQuiet()`);
      await d.sleep(300);
    };
    const project = () => d.page(`__t.session.backend.project`) as Promise<number>;
    const fire = (event: Record<string, unknown>) =>
      d.page(
        `(() => { const c = __t.session.backend.conn; for (const l of [...c.listeners]) l(${JSON.stringify(event)}); return true; })()`,
      );
    const showReview = async () => {
      await d.page(`__t.commands.run("view.review")`);
      await d.until(
        `document.querySelector(".review") !== null && document.querySelectorAll(".cm-editor").length > 0`,
        "the review screen's editors",
      );
    };
    const showQueue = async () => {
      await d.page(`__t.commands.run("view.queue")`);
      await d.until(`document.querySelector(".review") === null`, "the review screen to go");
    };
    const tabs = () =>
      d.page(
        `(async () => { const { appState } = await import("/src/app/appState.svelte.ts"); const c = appState.controller; return c && { open: [...c.tabs.open], active: c.tabs.active, view: c.activeMode }; })()`,
      );
    const hidden = async (before: Record<string, number>, what: string) => {
      await quiet();
      const after = await counts();
      const moved = REVIEW_ONLY.filter((t) => after[t] !== before[t]);
      if (moved.length) throw new Error(`${what}: the queue asked for the review screen: ${moved.join(", ")}`);
      return after;
    };
    const loadedAgain = async (before: Record<string, number>, types: readonly string[], what: string) => {
      await quiet();
      const after = await counts();
      for (const t of types)
        if (after[t] <= before[t])
          throw new Error(`${what}: the review screen did not load ${t} again (${JSON.stringify({ before, after })})`);
      return after;
    };
    const checks = (job: string, at: number, commit: string) => ({
      type: "checksChanged",
      project: at,
      checks: [
        {
          commit,
          state: "failure",
          runs: [{ id: job, name: job, state: "failure", phase: "finished", pipeline: "p1", duration: 1, log: true }],
          pipelines: [{ id: "p1", name: "p1", detail: "#1", attempt: 1, state: "failure", phase: "finished" }],
          url: "https://example.test/checks",
        },
      ],
      error: null,
      limitedUntil: null,
    });

    // 1. On the queue from the first paint: a revision and a server change move no review-only request.
    await quiet();
    const atStart = await counts();
    for (const t of REVIEW_ONLY)
      if (atStart[t] !== 0) throw new Error(`the queue asked for ${t} before the review showed`);
    await d.revise("b.txt", "wt2", [7, 25, 33]);
    await fire({ type: "serversChanged", project: await project() });
    await hidden(atStart, "a revision and a server change on the queue");

    const openFile = (path: string) =>
      d.page(
        `(async () => { const { appState } = await import("/src/app/appState.svelte.ts"); await appState.controller?.openSource("screen", ${JSON.stringify(path)}); return true; })()`,
      );
    const chooseSymbol = () =>
      d.page(`(async () => {
        const { symbols } = await import("/src/state/symbols.svelte.ts");
        symbols.choose({ id: { path: "a.txt", name: ["foo"] }, modules: 0, kind: "function", span: { start: { line: 0, col: 0 }, end: { line: 0, col: 3 } }, at: { line: 0, col: 0 }, test: false });
        return true;
      })()`);
    const showBottom = (tool: string) =>
      d.page(
        `(async () => { const { shell } = await import("/src/features/chrome/shell.svelte.ts"); shell.showBottom(${JSON.stringify(tool)}); return true; })()`,
      );
    const keepSearch = () =>
      d.page(`(async () => {
        const { findSets } = await import("/src/state/findSets.svelte.ts");
        findSets.keepSearch({
          spec: {
            type: "search",
            request: { scope: { type: "changeset" }, pattern: { text: "line", matchCase: false, wholeWord: false, regex: false }, mask: "", side: "both", onlyChanges: false, notSeen: false, limit: 0 },
            options: { text: "line", matchCase: false, wholeWord: false, regex: false, side: "both", onlyChanges: false },
            scope: { label: "All changes" },
          },
          results: {
            revision: __t.changeset.layout.shown.revision,
            files: [{ path: "b.txt", hunks: [], states: [], hits: 1, lines: [{ kind: "context", line: 3, text: "line 3", textStart: 0, hits: [{ from: 0, to: 4 }] }] }],
            hits: 1,
            truncated: false,
            searched: 1,
          },
        });
        return true;
      })()`);

    // 2. Show the review and set up what leaks in the changeset's own view: the versions, an open
    //    file tab, a selected symbol, and a Find result that goes stale on the next revision.
    await showReview();
    await d.page(call("toggleHunk", "a.txt", 0));
    await d.until(`__t.versions.items.length > 0`, "the first version");
    await openFile("a.txt");
    await keepSearch();
    await chooseSymbol();
    await quiet();
    const beforeQueue = await counts();
    const beforeTabs = await tabs();

    // 3. On the queue again: a revision and a server change move no review-only request.
    await showQueue();
    await d.revise("b.txt", "wt3", [5, 9]);
    await fire({ type: "serversChanged", project: await project() });
    const afterQueue = await hidden(beforeQueue, "a revision and a server change on the queue");

    // 4. Show the review again: it loads what it shows, current, and keeps its tab. The stale Find
    //    result then reads where its places are now.
    await showReview();
    await loadedAgain(
      afterQueue,
      ["versions", "signature", "calls", "usages", "reachability", "fileOutline", "fileContents"],
      "returning to the review",
    );
    const tabsBack = await tabs();
    if (JSON.stringify(tabsBack) !== JSON.stringify(beforeTabs))
      throw new Error(`the tabs were ${JSON.stringify(beforeTabs)} and came back as ${JSON.stringify(tabsBack)}`);
    await showBottom("find");
    await d.until(`document.querySelector("section[aria-label=Find]") !== null`, "the Find tool");
    await loadedAgain(afterQueue, ["mapLines"], "the Find tool's stale result");

    // 5. The by-commit view: a tab pinned at a commit and the Pipelines tool.
    await d.openBranch();
    await d.until(`__t.changeset.facts?.commits === 4`, "the branch");
    await d.page(
      `(async () => { const { shell } = await import("/src/features/chrome/shell.svelte.ts"); shell.showLeft("commits"); return true; })()`,
    );
    await d.until(`__t.changeset.lens === "perCommit" && __t.changeset.commits.length > 0`, "the commits");
    const commit = (await d.page(`__t.changeset.commits[__t.changeset.shownCommit].commit.id`)) as string;
    await openFile("a.txt");
    await fire(checks("job-a", await project(), commit));
    await showBottom("pipelines");
    await d.until(`document.querySelector("section[aria-label=Pipelines]") !== null`, "the Pipelines tool");
    await quiet();
    const beforeCommit = await counts();
    const pinnedTabs = await tabs();

    // 6. On the queue once more: a push and new checks move no review-only request.
    await showQueue();
    await d.pushBranch("b.txt", "p2", [12, 30]);
    await fire(checks("job-b", await project(), commit));
    const afterCommit = await hidden(beforeCommit, "new checks on the queue");

    // 7. Returning reads the commits and the chosen job's steps again, and keeps the pinned tab.
    await showReview();
    await loadedAgain(afterCommit, ["commits", "pipelineSteps", "fileContents"], "returning to the by-commit view");
    const pinnedBack = await tabs();
    if (JSON.stringify(pinnedBack) !== JSON.stringify(pinnedTabs))
      throw new Error(`the tabs were ${JSON.stringify(pinnedTabs)} and came back as ${JSON.stringify(pinnedBack)}`);
    const view = await d.page(`__t.changeset.layout?.shown.view.type`);
    if (view !== "commit") throw new Error(`the view came back as ${String(view)}, not the commit it was left at`);
  },

  /**
   * Going to the queue and back keeps the review screen as it was: its open tabs, the active one,
   * where it was scrolled and scrolled to, and an unsent comment draft.
   */
  async returning(d) {
    const showReview = async () => {
      await d.page(`__t.commands.run("view.review")`);
      await d.until(
        `document.querySelector(".review") !== null && document.querySelectorAll(".pane.active .cm-editor").length > 0`,
        "the review screen's editors",
      );
    };
    const showQueue = async () => {
      await d.page(`__t.commands.run("view.queue")`);
      await d.until(`document.querySelector(".review") === null`, "the review screen to go");
    };
    const tabs = () =>
      d.page(
        `(async () => { const { appState } = await import("/src/app/appState.svelte.ts"); const c = appState.controller; return c && { open: [...c.tabs.open], active: c.tabs.active, view: c.activeMode }; })()`,
      );
    const draft = () =>
      d.page(
        `(async () => { const { commentsUi } = await import("/src/features/comments/index.ts"); return commentsUi.changesetComment.body; })()`,
      );

    await showReview();
    // The Changes tab is left scrolled to the middle of the changeset, in All changes.
    await d.until(`document.querySelector(".pane.active .scroll") !== null`, "the Changes tab");
    await d.page(`document.querySelector(".pane.active .scroll").scrollTop = 800`);
    await d.sleep(500);
    const beforeProbe = await d.probe();
    await d.openText("a.txt");
    await d.page(`document.querySelector(".pane.active .scroll").scrollTop = 240`);
    await d.sleep(500);
    const fileProbe = await d.probe();
    await d.page(
      `(async () => { const { commentsUi } = await import("/src/features/comments/index.ts"); commentsUi.composeChangesetComment(); commentsUi.changesetComment.body = "an unsent draft"; return true; })()`,
    );
    const beforeTabs = await tabs();

    await showQueue();
    await showReview();

    const afterTabs = await tabs();
    if (JSON.stringify(afterTabs) !== JSON.stringify(beforeTabs))
      throw new Error(`the tabs were ${JSON.stringify(beforeTabs)} and came back as ${JSON.stringify(afterTabs)}`);
    const active = (afterTabs as { active: string }).active;
    const afterProbe = await d.probe();
    const wanted = active === "changes" ? beforeProbe : fileProbe;
    if (afterProbe.line !== wanted.line || Math.abs(afterProbe.y - wanted.y) > 1.5 || afterProbe.bar !== wanted.bar)
      throw new Error(
        `the ${active} tab was at ${JSON.stringify(wanted)} and came back at ${JSON.stringify(afterProbe)}`,
      );
    const kept = await draft();
    if (kept !== "an unsent draft") throw new Error(`the draft came back as ${JSON.stringify(kept)}`);
  },

  /**
   * A queue action that opens the review screen is not dropped: opening a thread, a log and a place
   * from the queue each bring the screen up and reach it.
   */
  async fromQueue(d) {
    const showQueue = async () => {
      await d.page(`__t.commands.run("view.queue")`);
      await d.until(`document.querySelector(".review") === null`, "the review screen to go");
    };
    const opened = () =>
      d.page(
        `(async () => { const { appState } = await import("/src/app/appState.svelte.ts"); return { shown: document.querySelector(".review") !== null, tabs: appState.controller ? [...appState.controller.tabs.open] : [] }; })()`,
      ) as Promise<{ shown: boolean; tabs: string[] }>;
    const onQueue = `document.querySelector(".review") === null`;

    // A thread on a file, made by another window while the queue shows.
    await d.until(`!!__t.changeset.layout`, "the changeset");
    const revision = (await d.page(`__t.changeset.layout.shown.revision`)) as number;
    await d.page(`(async () => {
      const { threads } = await import("/src/state/threads.svelte.ts");
      await threads.create({ revision: ${revision}, view: { type: "changeset" } }, { type: "file", side: "target", path: "a.txt" }, "a note on the file");
      return true;
    })()`);
    await d.until(
      `(async () => { const { threads } = await import("/src/state/threads.svelte.ts"); return threads.items.some((t) => t.path === "a.txt"); })()`,
      "the file's thread",
    );

    // 1. Opening a thread from the queue shows the review at it.
    await d.page(`(async () => {
      const { openThread } = await import("/src/app/actions.ts");
      const { threads } = await import("/src/state/threads.svelte.ts");
      openThread(threads.items.find((t) => t.path === "a.txt"));
      return true;
    })()`);
    await d.until(`document.querySelector(".review") !== null`, "the review screen the thread opened");
    // A thread on a file no tab shows goes to the Changes tab; what matters is that the screen came
    // up with its controller, so the intent was consumed rather than dropped while it was unset.
    const threadTabs = await opened();
    if (!threadTabs.shown || threadTabs.tabs.length === 0)
      throw new Error(`the thread intent reached no controller: ${JSON.stringify(threadTabs)}`);

    // 2. Opening a log from the queue shows the review; the fixture has no run, so it says so.
    await showQueue();
    await d.page(`(async () => {
      const { openCheckLog } = await import("/src/app/actions.ts");
      openCheckLog({ id: "job-a", name: "job-a", state: "failure", phase: "finished", pipeline: "p1", duration: 1, log: true });
      return true;
    })()`);
    await d.until(`document.querySelector(".review") !== null`, "the review screen the log opened");
    await d.until(
      `(async () => { const { notices } = await import("/src/state/notices.svelte.ts"); return /Could not load the log/.test(notices.text ?? ""); })()`,
      "the log to be reported as unavailable",
    );

    // 3. Opening a place from the queue shows the review at the place's file.
    await showQueue();
    if (!(await d.page(onQueue))) throw new Error("the review screen did not go away");
    await d.page(`(async () => {
      const { openPlace } = await import("/src/app/actions.ts");
      const { findSets } = await import("/src/state/findSets.svelte.ts");
      findSets.show = () => undefined;
      findSets.keepSearch({
        spec: {
          type: "search",
          request: { scope: { type: "view", view: { type: "changeset" } }, pattern: { text: "line", matchCase: false, wholeWord: false, regex: false }, mask: "", side: "both", onlyChanges: false, notSeen: false, limit: 0 },
          options: { text: "line", matchCase: false, wholeWord: false, regex: false, side: "both", onlyChanges: false },
          scope: { label: "All changes" },
        },
        results: { revision: ${revision}, files: [{ path: "b.txt", hunks: [], states: [], hits: 1, lines: [{ kind: "context", line: 1, text: "line", textStart: 0, hits: [{ from: 0, to: 4 }] }] }], hits: 1, truncated: false, searched: 1 },
      });
      const set = findSets.current;
      openPlace(set, set.places[0], true);
      return true;
    })()`);
    await d.until(`document.querySelector(".review") !== null`, "the review screen the place opened");
    const placeTabs = await opened();
    if (!placeTabs.tabs.some((id) => id.includes("b.txt")))
      throw new Error(`the place opened no tab: ${JSON.stringify(placeTabs)}`);
  },

  /** Find in Files exists only on the review screen: there and nowhere else must the key, the menu and Search Everywhere reach it. */
  async findInFiles(d) {
    const state = () =>
      d.page(`(async () => {
        const { commands } = await import("/src/state/commands.ts");
        const { findInFiles } = await import("/src/features/search/index.ts");
        const { mainMenu } = await import("/src/state/mainMenu.svelte.ts");
        const find = (items) => {
          for (const item of items) {
            if (item.id === "find.inFiles") return item;
            const child = item.children && find(item.children);
            if (child) return child;
          }
          return undefined;
        };
        return {
          enabled: commands.isEnabled("find.inFiles"),
          reason: commands.disabledReason("find.inFiles"),
          menu: find(mainMenu.items) !== undefined,
          open: findInFiles.open,
          popup: document.querySelector(".popup") !== null,
        };
      })()`) as Promise<{ enabled: boolean; reason?: string; menu: boolean; open: boolean; popup: boolean }>;
    /** The palette row that goes on to Find in Files, offered by the list or the empty state. */
    const paletteFindRow = () =>
      d.page(
        `[...document.querySelectorAll(".popup button")].some((b) => (b.textContent ?? "").includes("in files…"))`,
      ) as Promise<boolean>;
    const showPalette = async () => {
      await d.page(`__t.commands.run("palette.all")`);
      await d.until(`document.querySelector(".popup") !== null`, "Search Everywhere");
      await d.page(
        `(async () => { (await import("/src/features/palette/index.ts")).palette.query = "feature"; return true; })()`,
      );
      await d.sleep(250);
    };
    const hidePalette = async () => {
      await d.page(
        `(async () => { (await import("/src/features/palette/index.ts")).palette.close(); return true; })()`,
      );
      await d.until(`document.querySelector(".popup") === null`, "Search Everywhere to close");
    };
    const binding = "Meta+Shift+f";

    // 1. On the queue: the command is unavailable, its key does nothing, the menu has no entry and
    //    Search Everywhere offers no row on to it.
    const queued = await state();
    if (queued.enabled) throw new Error("Find in Files is available on the queue");
    if (queued.menu) throw new Error("the menu still has a Find in Files entry on the queue");
    await d.press(binding);
    await d.sleep(200);
    const afterKey = await state();
    if (afterKey.open || afterKey.popup) throw new Error("the Find in Files key opened it on the queue");
    await showPalette();
    if (await paletteFindRow()) throw new Error("Search Everywhere offers Find in Files on the queue");
    await hidePalette();

    // 2. On the review screen: the key, the menu entry and Search Everywhere all reach it.
    await d.page(`__t.commands.run("view.review")`);
    await d.until(`document.querySelector(".review") !== null`, "the review screen");
    const review = await state();
    if (!review.enabled) throw new Error(`Find in Files is not available on the review screen: ${review.reason}`);
    if (!review.menu) throw new Error("the menu has no Find in Files entry on the review screen");
    await d.press(binding);
    await d.until(
      `(async () => (await import("/src/features/search/index.ts")).findInFiles.open)()`,
      "the Find in Files dialog",
    );
    await d.page(
      `(async () => { (await import("/src/features/search/index.ts")).findInFiles.close(); return true; })()`,
    );
    await d.until(`document.querySelector(".popup") === null`, "the dialog to close");
    await showPalette();
    if (!(await paletteFindRow()))
      throw new Error("Search Everywhere offers no Find in Files row on the review screen");
    await hidePalette();
  },

  /**
   * Problems lists the changeset's files, not the commit's, while a commit is on view: it reads the
   * store's whole-changeset view, which the commit on screen is not.
   */
  async problems(d) {
    await d.openBranch();
    await d.page(`__t.changeset.showLens("perCommit")`);
    await d.until(`__t.changeset.shownCommit !== undefined`, "a commit on view");
    await d.page(
      `(async () => { const { shell } = await import("/src/features/chrome/shell.svelte.ts"); shell.showBottom("problems"); return true; })()`,
    );
    await d.until(`document.querySelector("section[aria-label=Problems]") !== null`, "the Problems tool");
    const files = ["a.txt", "b.txt", "c.txt"];
    await d.until(
      `(() => { const s = document.querySelector("section[aria-label=Problems]")?.innerText ?? ""; return ${JSON.stringify(files)}.every((p) => s.includes(p)); })()`,
      "the changeset's files in Problems",
    );
    const shown = (await d.page(`document.querySelector("section[aria-label=Problems]").innerText`)) as string;
    for (const path of files)
      if (!shown.includes(path)) throw new Error(`Problems does not list ${path} while a commit is on view:\n${shown}`);
  },

  /** Changes since the last review, run from the queue, reads the versions the screen's start makes, once. */
  async sinceLast(d) {
    await d.page(`__t.commands.run("view.review")`);
    await d.until(`document.querySelector(".review") !== null`, "the review screen");
    await d.page(call("toggleHunk", "a.txt", 0));
    await d.until(`__t.versions.items.length > 0`, "the first version");
    await d.revise("b.txt", "wt2", [7, 25, 33]);
    await d.until(`__t.versions.items.length > 1`, "the second version");
    await d.page(`__t.commands.run("view.queue")`);
    await d.until(`document.querySelector(".review") === null`, "the review screen to go");
    await d.page(`__t.session.backend.waitForQuiet()`);
    const before = (await d.page(`__wire.versions`)) as number;
    await d.page(`__t.commands.run("changeset.toggleSinceLast")`);
    await d.until(`document.querySelector(".review") !== null`, "the review screen back");
    await d.page(`__t.session.backend.waitForQuiet()`);
    await d.sleep(400);
    const after = (await d.page(`__wire.versions`)) as number;
    if (after - before !== 1)
      throw new Error(`changes since the last review read the versions ${after - before} times, not once`);
    if ((await d.page(`!!__t.changeset.range`)) !== true) throw new Error("nothing was shown since the last review");
  },
};
