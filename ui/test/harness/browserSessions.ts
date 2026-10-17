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
  `__t.opened.session.${method}(__t.opened.session.currentShown(),${JSON.stringify(path)}${hunk === undefined ? "" : `, ${hunk}`})`;

export const sessions: Record<string, (d: Driver) => Promise<void>> = {
  /**
   * The All files tree lists the target snapshot; while the review screen is away on the queue it
   * lists nothing, a revision that settles there waits, and the tree loads when the screen comes up.
   */
  async allFiles(d) {
    await d.until(`!!__t.opened.session?.layout`, "the changeset");
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
    await d.until(`!!__t.opened.session?.layout`, "the changeset");
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
    await d.page(`(() => { const l = __t.opened.session.cells.listeners; window.__muted = new Set(l); l.clear(); })()`);
    await d.step("another window marks while the views are not told", () => d.otherMark("b.txt", 0), { stale: true });
    await d.page(`window.__muted.forEach((l) => __t.opened.session.cells.listeners.add(l))`);
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
    await keeps("a push is chosen", all, () => d.page(`__t.opened.session.setRange({ from: 1, to: 2 })`));
    await keeps("all changes again", all, () => d.page(`__t.opened.session.showWhole()`));

    await d.openBranch();
    await d.openText("a.txt");
    await show("unified");
    const whole = await scrolledTo();
    await keeps("a commit is shown", whole, () => d.page(`__t.opened.session.showLens("perCommit")`));
    await keeps("all changes after the commit", whole, () => d.page(`__t.opened.session.showLens("squash")`));
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
    const files = () => d.page(`__t.opened.session.paths.length`) as Promise<number>;
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
      await d.page(`__t.opened.session.setRange({ from: 1, to: 2 })`);
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
        __t.opened.session.symbols.choose({ id: { path: "a.txt", name: ["foo"] }, modules: 0, kind: "function", span: { start: { line: 0, col: 0 }, end: { line: 0, col: 3 } }, at: { line: 0, col: 0 }, test: false });
        return true;
      })()`);
    const showBottom = (tool: string) =>
      d.page(
        `(async () => { const { shell } = await import("/src/features/chrome/shell.svelte.ts"); shell.showBottom(${JSON.stringify(tool)}); return true; })()`,
      );
    const keepSearch = () =>
      d.page(`(async () => {
        __t.opened.session.findSets.keepSearch({
          spec: {
            type: "search",
            request: { scope: { type: "changeset" }, pattern: { text: "line", matchCase: false, wholeWord: false, regex: false }, mask: "", side: "both", onlyChanges: false, notSeen: false, limit: 0 },
            options: { text: "line", matchCase: false, wholeWord: false, regex: false, side: "both", onlyChanges: false },
            scope: { label: "All changes" },
          },
          results: {
            revision: __t.opened.session?.layout.shown.revision,
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
    await d.until(`__t.opened.session.versions.items.length > 0`, "the first version");
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
    await d.until(`__t.opened.session?.facts?.commits === 4`, "the branch");
    await d.page(
      `(async () => { const { shell } = await import("/src/features/chrome/shell.svelte.ts"); shell.showLeft("commits"); return true; })()`,
    );
    await d.until(`__t.opened.session?.lens === "perCommit" && __t.opened.session.commits.length > 0`, "the commits");
    const commit = (await d.page(`__t.opened.session.commits[__t.opened.session.shownCommit].commit.id`)) as string;
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
    const view = await d.page(`__t.opened.session?.layout?.shown.view.type`);
    if (view !== "commit") throw new Error(`the view came back as ${String(view)}, not the commit it was left at`);
  },

  /**
   * A server that becomes ready tells what it offers with the runtimes it pushes; the list of the
   * servers is not read again for it, only for a change of the servers themselves.
   */
  async serverPushes(d) {
    const quiet = async () => {
      await d.page(`__t.session.backend.waitForQuiet()`);
      await d.sleep(300);
    };
    const listed = () => d.page(`window.__wire.listServers ?? 0`) as Promise<number>;
    const project = () => d.page(`__t.session.backend.project`) as Promise<number>;
    const fire = (event: Record<string, unknown>) =>
      d.page(
        `(() => { const c = __t.session.backend.conn; for (const l of [...c.listeners]) l(${JSON.stringify(event)}); return true; })()`,
      );
    const ready = [
      ["rust", "target"],
      ["rust", "base"],
      ["python", "target"],
      ["python", "base"],
      ["typescript", "target"],
    ] as const;
    const all = {
      documentSymbols: true,
      definition: true,
      references: true,
      implementations: true,
      hover: true,
      callHierarchy: true,
      workspaceSymbols: true,
    };

    await quiet();
    const before = await listed();
    const at = await project();
    for (let i = 0; i < ready.length; i++) {
      await fire({
        type: "serverRuntimes",
        project: at,
        runtimes: {
          serial: 1000 + i,
          runtimes: ready.slice(0, i + 1).map(([language, side]) => ({ language, side, phase: { type: "ready" } })),
          features: [{ language: "rust", features: { ...all, implementations: i < 2 } }],
        },
      });
    }
    await quiet();
    if ((await listed()) !== before) throw new Error("servers becoming ready read the list of the servers again");
    const offers = await d.page(
      `(async () => (await import("/src/state/serverList.svelte.ts")).serverList.runtimes.features.find((f) => f.language === "rust")?.features.implementations)()`,
    );
    if (offers !== false) throw new Error(`the pushed features were not taken: implementations is ${String(offers)}`);

    await fire({ type: "serversChanged", project: at });
    await quiet();
    if ((await listed()) !== before + 1) throw new Error("a change of the servers did not read the list exactly once");
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
    const draft = () => d.page(`__t.opened.session.changesetComment.body`);

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
      `(() => { const draft = __t.opened.session.changesetComment; draft.composing = true; draft.body = "an unsent draft"; return true; })()`,
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
   * Switching to another changeset and back: the changeset's tabs and scroll positions come back, none
   * of the other's show while it is on view, the window's tool panels keep their settings, and the
   * controller of the session left is destroyed.
   */
  async switching(d) {
    const tabs = () =>
      d.page(
        `(async () => { const { appState } = await import("/src/app/appState.svelte.ts"); const c = appState.controller; return c && { open: [...c.tabs.open], active: c.tabs.active, view: c.activeMode }; })()`,
      );
    const settings = () =>
      d.page(
        `({
          errors: document.querySelector('section[aria-label=Problems] button[title="Errors"]')?.getAttribute("aria-pressed"),
          scope: [...document.querySelectorAll('section[aria-label=Problems] [aria-label=Scope] button')].find((b) => b.getAttribute("aria-pressed") === "true")?.textContent,
        })`,
      );
    const listeners = () => d.page(`__t.session.resyncListeners.size`) as Promise<number>;
    const switchTo = async (key: string, type: string) => {
      await d.page(`__t.opened.open(${key})`);
      await d.until(
        `__t.opened.session?.facts?.target.changeset.type === ${JSON.stringify(type)} && document.querySelectorAll(".pane.active .cm-editor").length > 0`,
        `the ${type} on view`,
      );
    };

    // The working tree: the Changes tab scrolled, a file's text open and scrolled, Problems set up.
    await d.until(`document.querySelector(".pane.active .scroll") !== null`, "the Changes tab");
    await d.openText("a.txt");
    await d.page(`document.querySelector(".pane.active .scroll").scrollTop = 240`);
    await d.sleep(500);
    const fileProbe = await d.probe();
    await d.page(
      `(async () => { const { shell } = await import("/src/features/chrome/shell.svelte.ts"); shell.showBottom("problems"); return true; })()`,
    );
    await d.until(`document.querySelector("section[aria-label=Problems]") !== null`, "the Problems tool");
    await d.click('section[aria-label=Problems] button[title="Errors"]');
    await d.click("section[aria-label=Problems] [aria-label=Scope] button:nth-child(2)");
    const set = await settings();
    if (JSON.stringify(set) === JSON.stringify({ errors: "true", scope: "Changed files" }))
      throw new Error(`the Problems settings did not change: ${JSON.stringify(set)}`);
    // The last change is the Changes tab chosen, and the switch follows at once: it saves what the pause did not.
    await d.click("[role=tablist] [role=tab]");
    const before = await tabs();
    const held = await listeners();
    await d.page(`(window.__left = __t.opened.session, true)`);

    // The branch: nothing of the working tree shows, and the session left is over.
    await switchTo(`{ type: "branch", name: "feature" }`, "branch");
    const onBranch = await tabs();
    if (JSON.stringify(onBranch) !== JSON.stringify({ open: ["changes"], active: "changes", view: "unified" }))
      throw new Error(`the branch shows the tabs ${JSON.stringify(onBranch)}`);
    const texts = (await d.page(`Object.keys(window.__vet.rendered().texts)`)) as string[];
    if (texts.length) throw new Error(`the branch shows the text of ${texts.join(", ")}`);
    const left = await d.page(`({ alive: window.__left.lifetime.alive, listening: window.__left.viewListeners.size })`);
    if (JSON.stringify(left) !== JSON.stringify({ alive: false, listening: 0 }))
      throw new Error(`the session left is still listened to: ${JSON.stringify(left)}`);
    if (JSON.stringify(await settings()) !== JSON.stringify(set))
      throw new Error(`the Problems settings were ${JSON.stringify(set)} and are ${JSON.stringify(await settings())}`);

    // Back: the tabs and the places are the working tree's, and no controller of the branch is left.
    await switchTo(`{ type: "workingTree" }`, "workingTree");
    await d.until(`document.querySelectorAll(".pane.active .cm-editor").length > 0`, "the editors");
    await d.sleep(500);
    const after = await tabs();
    if (JSON.stringify(after) !== JSON.stringify(before))
      throw new Error(`the tabs were ${JSON.stringify(before)} and came back as ${JSON.stringify(after)}`);
    const sameSpot = (a: Probe, b: Probe) => a.line === b.line && Math.abs(a.y - b.y) <= 1.5 && a.bar === b.bar;
    await d.click("[role=tablist] [role=tab] >> nth=1");
    await d.sleep(500);
    const fileBack = await d.probe();
    if (!sameSpot(fileProbe, fileBack))
      throw new Error(`the file was at ${JSON.stringify(fileProbe)} and came back at ${JSON.stringify(fileBack)}`);

    // Left on the Changes tab, its place comes back.
    await d.click("[role=tablist] [role=tab]");
    await d.page(`document.querySelector(".pane.active .scroll").scrollTop = 800`);
    await d.sleep(500);
    const changesProbe = await d.probe();
    await switchTo(`{ type: "branch", name: "feature" }`, "branch");
    await switchTo(`{ type: "workingTree" }`, "workingTree");
    await d.sleep(1000);
    // The place is restored by file and distance, which lands on the same file but not on the same pixel.
    const scrolled = (await d.page(`document.querySelector(".pane.active .scroll").scrollTop`)) as number;
    if (scrolled < 400) throw new Error(`the Changes tab was at ${changesProbe.line} and came back at the top`);
    if (JSON.stringify(await settings()) !== JSON.stringify(set))
      throw new Error(`the Problems settings were ${JSON.stringify(set)} and are ${JSON.stringify(await settings())}`);
    const kept = await listeners();
    if (kept !== held) throw new Error(`the window had ${held} resync listeners and has ${kept} after switching`);
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
    await d.until(`!!__t.opened.session?.layout`, "the changeset");
    const revision = (await d.page(`__t.opened.session?.layout.shown.revision`)) as number;
    await d.page(`(async () => {
      await __t.opened.session.threads.create({ revision: ${revision}, view: { type: "changeset" } }, { type: "file", side: "target", path: "a.txt" }, "a note on the file");
      return true;
    })()`);
    await d.until(`__t.opened.session.threads.items.some((t) => t.path === "a.txt")`, "the file's thread");

    // 1. Opening a thread from the queue shows the review at it.
    await d.page(`(async () => {
      const { openThread } = await import("/src/app/actions.ts");
      openThread(__t.opened.session.threads.items.find((t) => t.path === "a.txt"));
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
      const { findTool } = await import("/src/state/findTool.svelte.ts");
      findTool.show = () => undefined;
      const findSets = __t.opened.session.findSets;
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
    await d.page(`__t.opened.session.showLens("perCommit")`);
    await d.until(`__t.opened.session?.shownCommit !== undefined`, "a commit on view");
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

  /**
   * The Changes tab starts at the top when another range or commit is shown, and keeps its place when
   * a new revision of the same view arrives. The views are long, so a tab that kept its place by mistake
   * would not be at the top.
   */
  async viewStart(d) {
    const TOTAL = 120;
    interface Place {
      path: string;
      y: number;
      scrollTop: number;
    }
    const place = () =>
      d.page(`(() => {
        const scroller = document.querySelector(".pane.active .scroll");
        const top = scroller.getBoundingClientRect().top;
        for (const s of scroller.querySelectorAll("section.file")) {
          const at = s.getBoundingClientRect();
          if (at.bottom > top + 1) return { path: s.querySelector(".path").textContent, y: Math.round(at.top - top), scrollTop: scroller.scrollTop };
        }
      })()`) as Promise<Place>;
    const first = () => d.page(`__t.opened.session?.shownFiles[0].path`) as Promise<string>;
    const scrolledAway = async (): Promise<Place> => {
      await d.page(
        `(() => { const s = document.querySelector(".pane.active .scroll"); s.scrollTop = s.scrollHeight / 2; })()`,
      );
      await d.sleep(500);
      const at = await place();
      if (at.scrollTop < 400 || at.path === (await first()))
        throw new Error(`the Changes tab did not scroll: ${JSON.stringify(at)}`);
      return at;
    };
    const atTop = async (what: string) => {
      const at = await place();
      if (at.scrollTop > 2 || at.path !== (await first()))
        throw new Error(`${what}: the Changes tab is at ${JSON.stringify(at)}, not at the top`);
    };
    const keeps = async (what: string, before: Place) => {
      await d.sleep(500);
      const at = await place();
      if (at.path !== before.path || Math.abs(at.y - before.y) > 2 || at.scrollTop < 400)
        throw new Error(`${what}: the Changes tab was at ${JSON.stringify(before)} and is at ${JSON.stringify(at)}`);
    };

    await d.manyFiles(TOTAL);
    await d.step("a hunk is seen", () => d.page(call("toggleHunk", "b.txt", 0)), ONE);
    await d.until(`__t.opened.session.versions.items.length >= 1`, "the first version");
    const all = await scrolledAway();
    await d.step("every file changes: a new revision", () => d.manyFiles(TOTAL, "v2"));
    await keeps("a new revision of All changes", all);
    await d.until(`__t.opened.session.versions.items.length >= 2`, "the second version");

    await scrolledAway();
    await d.step("a push is chosen", () => d.page(`__t.opened.session.setRange({ from: 1, to: 2 })`));
    await atTop("a range is shown");
    const range = await scrolledAway();
    await d.step("every file changes again: a new revision of the range", () => d.manyFiles(TOTAL, "v3"));
    await keeps("a new revision of a range", range);
    await d.step("all changes again", () => d.page(`__t.opened.session.showWhole()`));
    await atTop("all changes after the range");

    await d.openBranch();
    await d.step("a push adds the files", () => d.pushManyFiles(TOTAL, "v4"));
    await d.step("a push changes every file", () => d.pushManyFiles(TOTAL, "v5"));
    await d.page(`__t.opened.session.showLens("perCommit")`);
    await d.step("a long commit is shown", () => d.page(`__t.opened.session.showCommit(4)`));
    await d.until(
      `__t.opened.session?.shownCommit === 4 && __t.opened.session?.shownFiles.length > 100`,
      "the long commit",
    );
    await scrolledAway();
    await d.step("another long commit is shown", () => d.page(`__t.opened.session.showCommit(5)`));
    await d.until(
      `__t.opened.session?.shownCommit === 5 && __t.opened.session?.shownFiles.length > 100`,
      "the next long commit",
    );
    await atTop("another commit is shown");
  },

  /** Changes since the last review, run from the queue, reads the versions the screen's start makes, once. */
  async sinceLast(d) {
    await d.page(`__t.commands.run("view.review")`);
    await d.until(`document.querySelector(".review") !== null`, "the review screen");
    await d.page(call("toggleHunk", "a.txt", 0));
    await d.until(`__t.opened.session.versions.items.length > 0`, "the first version");
    await d.revise("b.txt", "wt2", [7, 25, 33]);
    await d.until(`__t.opened.session.versions.items.length > 1`, "the second version");
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
    if ((await d.page(`!!__t.opened.session.range`)) !== true)
      throw new Error("nothing was shown since the last review");
  },

  /**
   * Switching to another changeset with text in the comment box asks first: Keep editing (a click or
   * Escape) keeps the text and the changeset, Discard switches, and the next changeset has an empty box.
   */
  async discarding(d) {
    const dialog = () =>
      d.page(`(() => {
        const sheet = document.querySelector("dialog[open]");
        return sheet && {
          title: sheet.getAttribute("aria-label"),
          text: sheet.querySelector("p")?.textContent,
          buttons: [...sheet.querySelectorAll("footer button")].map((b) => b.textContent.trim()),
          focused: document.activeElement?.textContent?.trim(),
        };
      })()`) as Promise<{ title: string; text: string; buttons: string[]; focused: string } | null>;
    const typed = () => d.page(`document.querySelector("textarea[placeholder^='Comment on the changeset']")?.value`);
    const onView = () => d.page(`__t.opened.session?.key.type`);
    const switchAway = async () => {
      await d.page(`(window.__switch = __t.opened.open({ type: "branch", name: "feature" }), true)`);
      await d.until(`document.querySelector("dialog[open]") !== null`, "the dialog");
    };
    const expectGone = async (what: string) => {
      await d.until(`document.querySelector("dialog[open]") === null`, `the dialog to close on ${what}`);
      await d.page(`window.__switch`);
    };

    await d.page(`__t.commands.run("review.commentOnChangeset")`);
    await d.until(`document.querySelector("textarea[placeholder^='Comment on the changeset']") !== null`, "the box");
    await d.click("textarea[placeholder^='Comment on the changeset']");
    for (const key of "note") await d.press(key);
    if ((await typed()) !== "note") throw new Error(`the box holds ${JSON.stringify(await typed())}, not "note"`);

    await switchAway();
    const asked = await dialog();
    if (asked?.title !== "Discard your comment?") throw new Error(`the dialog is titled ${asked?.title}`);
    if (asked.text !== "You haven't added the comment you're writing. It will be lost.")
      throw new Error(`the dialog says ${JSON.stringify(asked.text)}`);
    if (asked.buttons.join("|") !== "Keep editing|Discard")
      throw new Error(`the buttons are ${asked.buttons.join(", ")}`);
    if (asked.focused !== "Keep editing") throw new Error(`${asked.focused} has the focus, not Keep editing`);
    if ((await onView()) !== "workingTree") throw new Error("the changeset was switched before the answer");
    await d.screenshot("discard-dialog");

    await d.press("Escape");
    await expectGone("Escape");
    if ((await onView()) !== "workingTree") throw new Error("Escape did not keep the changeset");
    if ((await typed()) !== "note") throw new Error("Escape did not keep the text");

    await switchAway();
    await d.click("dialog[open] footer button:first-child");
    await expectGone("Keep editing");
    if ((await onView()) !== "workingTree") throw new Error("Keep editing did not keep the changeset");
    if ((await typed()) !== "note") throw new Error("Keep editing did not keep the text");

    await switchAway();
    await d.click("dialog[open] footer button.destructive");
    await expectGone("Discard");
    await d.until(`__t.opened.session?.key.type === "branch" && !!__t.opened.session.layout`, "the branch");
    if ((await d.page(`__t.opened.session.changesetComment.body`)) !== "")
      throw new Error("the next changeset has the text of the one left");
  },

  /** What the Find tool listed belongs to the changeset: after a switch it is empty, and the first changeset's list is not brought back. */
  async findAfterSwitch(d) {
    await d.page(`(async () => {
      __t.opened.session.findSets.keepSearch({
        spec: {
          type: "search",
          request: { scope: { type: "changeset" }, pattern: { text: "line", matchCase: false, wholeWord: false, regex: false }, mask: "", side: "both", onlyChanges: false, notSeen: false, limit: 0 },
          options: { text: "line", matchCase: false, wholeWord: false, regex: false, side: "both", onlyChanges: false },
          scope: { label: "All changes" },
        },
        results: { type: "searchResults", revision: __t.opened.session.layout.shown.revision, files: [], hits: 0, truncated: false, searched: 0 },
      });
      const { shell } = await import("/src/features/chrome/shell.svelte.ts");
      shell.showBottom("find");
      return true;
    })()`);
    await d.until(`document.querySelector("section[aria-label=Find] [role=tablist] [role=tab]") !== null`, "the tab");
    await d.page(`__t.opened.open({ type: "branch", name: "feature" })`);
    await d.until(`__t.opened.session?.key.type === "branch" && !!__t.opened.session.layout`, "the branch");
    await d.until(
      `document.querySelector("section[aria-label=Find]")?.innerText.includes("Nothing kept yet") === true`,
      "the Find tool to be empty",
    );
    if ((await d.page(`document.querySelectorAll("section[aria-label=Find] [role=tab]").length`)) !== 0)
      throw new Error("a result set of the other changeset is listed");
    if ((await d.page(`__t.opened.session.findSets.sets.length`)) !== 0)
      throw new Error("the session holds result sets");
    await d.page(`__t.opened.open({ type: "workingTree" })`);
    await d.until(`__t.opened.session?.key.type === "workingTree" && !!__t.opened.session.layout`, "the working tree");
    if ((await d.page(`document.querySelectorAll("section[aria-label=Find] [role=tab]").length`)) !== 0)
      throw new Error("going back brought the result sets back");
  },

  /** Go to definition in a file no server serves tells the viewer why, in the backend's words. */
  async definitionNotice(d) {
    await d.openText("a.txt");
    await d.until(`document.querySelector(".pane.active .cm-editor") !== null`, "the file's editor");
    await d.click(".pane.active .cm-content .cm-line");
    await d.page(`(async () => { (await import("/src/state/notices.svelte.ts")).notices.close(); return true; })()`);
    const expected = (await d.page(`(async () => {
      const r = await __t.session.backend.definition(__t.opened.session.layout.shown.revision, "target", "a.txt", 0, 0);
      return r.unavailable?.message ?? (r.locations.length ? null : "No definition found");
    })()`)) as string | null;
    if (!expected) throw new Error("the backend found a definition in a.txt");
    await d.page(`__t.commands.run("review.goToDefinition")`);
    await d
      .until(
        `(async () => (await import("/src/state/notices.svelte.ts")).notices.text === ${JSON.stringify(expected)})()`,
        "the definition notice",
      )
      .catch(() => {
        throw new Error("go to definition said nothing");
      });
  },

  /**
   * The diagnostics pushed for the changeset on view stay across a trip to the queue and are taken
   * after it; a server change drops them, and another changeset starts with none.
   */
  async diagnosticsHeld(d) {
    const quiet = async () => {
      await d.page(`__t.session.backend.waitForQuiet()`);
      await d.sleep(300);
    };
    const project = () => d.page(`__t.session.backend.project`) as Promise<number>;
    const fire = (event: Record<string, unknown>) =>
      d.page(
        `(() => { const c = __t.session.backend.conn; for (const l of [...c.listeners]) l(${JSON.stringify(event)}); return true; })()`,
      );
    const push = async (path: string) =>
      fire({
        type: "fileDiagnostics",
        project: await project(),
        revision: await d.page(`__t.opened.session.layout.shown.revision`),
        path,
        diagnostics: [],
        count: 0,
        unavailable: null,
        originUnavailable: null,
        incomplete: null,
        compared: true,
      });
    const held = () =>
      d.page(
        `(async () => { const { semantic } = await import("/src/state/semantic.svelte.ts"); return { files: [...semantic.files.keys()].sort(), previous: semantic.previous.size }; })()`,
      ) as Promise<{ files: string[]; previous: number }>;
    const files = async () => JSON.stringify((await held()).files);

    await d.until(`!!__t.opened.session?.layout`, "the changeset");
    await push("held/a.rs");
    await quiet();
    if ((await files()) !== JSON.stringify(["held/a.rs"])) throw new Error("a push was not taken");

    await d.page(`__t.commands.run("view.queue")`);
    await d.until(`document.querySelector(".review") === null`, "the review screen to go");
    await d.page(`__t.commands.run("view.review")`);
    await d.until(
      `document.querySelector(".review") !== null && document.querySelectorAll(".cm-editor").length > 0`,
      "the review screen's editors",
    );
    await quiet();
    if ((await files()) !== JSON.stringify(["held/a.rs"]))
      throw new Error("the diagnostics were dropped by a trip to the queue");

    await push("held/b.rs");
    await quiet();
    if ((await files()) !== JSON.stringify(["held/a.rs", "held/b.rs"]))
      throw new Error("a push after a trip to the queue was not taken");

    await fire({ type: "serversChanged", project: await project() });
    await quiet();
    if ((await files()) !== "[]") throw new Error("a server change did not drop the diagnostics");

    await push("held/a.rs");
    await d.openBranch();
    await d.until(`__t.opened.session?.key.type === "branch" && !!__t.opened.session.layout`, "the branch");
    const other = await held();
    if (other.files.length || other.previous)
      throw new Error(`another changeset started with ${JSON.stringify(other)}`);
  },
};
