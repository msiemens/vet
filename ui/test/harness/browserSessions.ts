// The browser tier's scripted sessions: each drives the real app in a page and, after every step,
// lets the harness compare what the views drew with the backend. Each is a Playwright test.

import type { Driver, Probe } from "./browserHarness.ts";

/** The sessions whose script starts on the queue, before the review screen is shown. */
export const startOnQueue = new Set<string>(["hidden", "many"]);

/** A step that sends one review request. */
const ONE = { marks: 1 };

const call = (method: string, path: string, hunk?: number) =>
  `__t.changeset.${method}(__t.changeset.currentShown(), ${JSON.stringify(path)}${hunk === undefined ? "" : `, ${hunk}`})`;

export const sessions: Record<string, (d: Driver) => Promise<void>> = {
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
};
