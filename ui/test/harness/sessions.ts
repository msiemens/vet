// The scripted sessions. Each runs in a process and backend of its own; a step ends when the window has
// settled, and then the stores are compared with the backend.

import type { View } from "../../src/generated/View.ts";
import type { Request } from "../../src/generated/Request.ts";
import { changeset } from "../../src/state/changeset.svelte.ts";
import { problems } from "../../src/state/problems.svelte.ts";
import { queue } from "../../src/state/queue.svelte.ts";
import { reviewShown } from "../../src/state/reviewShown.svelte.ts";
import { serverList } from "../../src/state/serverList.svelte.ts";
// Asks for the changeset's symbols while it is shown.
import "../../src/state/symbols.svelte.ts";
import { threads } from "../../src/state/threads.svelte.ts";
import { defaultRange } from "../../src/state/timelineRange.ts";
import { versions } from "../../src/state/versions.svelte.ts";
import { newScope } from "../../src/state/viewScope.svelte.ts";
import type { Harness } from "./harness.ts";
import { watchFromEffect } from "./watching.svelte.ts";

const CHANGESET: View = { type: "changeset" };

/** The requests that start the language servers or ask them. */
const ANALYSES = new Set<Request["type"]>(["diagnostics", "fileSymbols", "fileOutline", "symbolAt"]);

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** A step that sends one review request. */
const ONE = { marks: 1 };

/** Marks or flags as a click in the views on screen does, or in the ones built for `from`. */
const hunk = (h: Harness, path: string, index: number, from = h.shown()) =>
  h.asElement(() => changeset.toggleHunk(from, path, index), from);
const flag = (h: Harness, path: string, index: number, from = h.shown()) =>
  h.asElement(() => changeset.toggleFlag(from, path, index), from);
const file = (h: Harness, path: string) => h.asElement(() => changeset.toggleFileSeen(h.shown(), path));
const directory = (h: Harness, dir: string, seen: boolean) =>
  h.asElement(() => changeset.setDirectorySeen(h.shown(), dir, seen));

/** The first file of the view on screen. */
const first = () => changeset.paths[0];

async function reviewByCommit(h: Harness) {
  await changeset.openChangeset({ type: "branch", name: "feature" });
  await h.until(() => changeset.facts?.target.changeset.type === "branch", "the branch");
  await changeset.showLens("perCommit");
  await h.until(() => changeset.shownCommit !== undefined, "the first commit");
}

/** A new version of the working tree, as an edit on disk makes it. */
async function settleVersion(h: Harness) {
  h.fixture.write("b.txt", "wt2", [7, 25, 33]);
  await h.until(() => versions.items.length >= 2 && changeset.facts?.review.state === "open", "the second version");
}

export const sessions: Record<string, (h: Harness) => Promise<void>> = {
  /** Marks, flags and unmarks in the changeset view. */
  async changeset(h) {
    await h.step("opened", () => undefined);
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), ONE);
    await h.step("hunk flagged", () => flag(h, "b.txt", 1), ONE);
    await h.step("hunk unseen", () => hunk(h, "b.txt", 0), ONE);
    await h.step("file seen", () => file(h, "a.txt"), ONE);
    await h.step("folder seen", () => directory(h, "src", true), ONE);
    await h.step("folder unseen", () => directory(h, "src", false), ONE);
    await h.step(
      "thread opened",
      () => h.asElement(() => threads.create(h.shown(), { type: "changeset" }, "a note on the whole changeset")),
      ONE,
    );
  },

  /** A new version, the range since the last review, marks in it, and back. */
  async range(h) {
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), ONE);
    await h.step("version settled", () => settleVersion(h));
    await h.step("range shown", () => changeset.setRange(defaultRange(versions.items)));
    await h.step("flagged in range", () => flag(h, first(), 0), ONE);
    await h.step("seen in range", () => hunk(h, first(), 1), ONE);
    await h.step("unseen in range", () => hunk(h, first(), 1), ONE);
    await h.step("back to the changeset", () => changeset.setRange(undefined));
    await h.step("seen in the new version", () => hunk(h, "b.txt", 2), ONE);
  },

  /** The changeset reviewed by commit: marks, stepping, and the lens back. */
  async commits(h) {
    await h.step("opened the branch by commit", () => reviewByCommit(h));
    await h.step("seen in a commit", () => hunk(h, first(), 0), ONE);
    await h.step("next commit", () => changeset.showCommit(1));
    await h.step("flagged in a commit", () => flag(h, first(), 0), ONE);
    await h.step("file seen in a commit", () => file(h, first()), ONE);
    await h.step("last commit", () => changeset.showCommit(2));
    await h.step("lens back to the whole", () => changeset.showLens("squash"));
    await h.step("seen in the changeset", () => hunk(h, "a.txt", 0), ONE);
    await h.step("by commit again", () => changeset.showLens("perCommit"));
  },

  /** A scope of a commit that is not the one on screen has that commit's files and marks, and keeps them while the window moves on. */
  async pinned(h) {
    await h.step("opened the branch by commit", () => reviewByCommit(h));
    const commit = (index: number): View => ({ type: "commit", index });
    const id = changeset.commits[0].commit.id;
    const scope = newScope(h.other, { type: "commit", id }, () => undefined);
    const hunksOf = (layout: { files: readonly { path: string; hunks: readonly unknown[] }[] } | undefined) =>
      layout?.files.find((f) => f.path === "a.txt")?.hunks.length;
    await h.step("another commit on screen", async () => {
      await changeset.showCommit(2);
      await scope.load();
      const own = await h.other.manifest(commit(0));
      const wanted = own.files.find((f) => f.path === "a.txt")?.hunks.length;
      if (!wanted || hunksOf(scope.layout) !== wanted || scope.commit?.commit.id !== id)
        throw new Error("the scope does not read its own commit");
      if (hunksOf(changeset.currentLayout()) === wanted) throw new Error("the commits cannot be told apart");
      const text = await h.other.fileText("b.txt", commit(0));
      if (text.content.type !== "text" || text.content.text.includes("c2"))
        throw new Error("a file the commit does not touch is not read as the commit leaves it");
      const missing = await h.other.fileText("not/there.txt", commit(0));
      if (missing.content.type !== "noText" || missing.content.reason !== "absent")
        throw new Error("a file the commit lacks is not answered as absent");
    });
    await h.step("a mark in the scope", async () => {
      const shown = scope.currentShown();
      if (!shown) throw new Error("the scope has not read");
      await scope.toggleHunk(shown, "a.txt", 0);
      const own = await h.other.manifest(commit(0));
      const held = scope.state("a.txt")?.review.hunks;
      const real = own.files.find((f) => f.path === "a.txt")?.review.hunks;
      if (held?.[0] !== "seen" || JSON.stringify(held) !== JSON.stringify(real))
        throw new Error("the scope does not hold the mark");
    });
    await h.step("a mark made elsewhere is read once, its echo is not read again", async () => {
      let reads = 0;
      const manifest = h.other.manifest.bind(h.other);
      h.other.manifest = (view) => {
        reads++;
        return manifest(view);
      };
      const revision = scope.currentShown()?.revision ?? -1;
      const answer = await h.other.toggleHunk(revision, "a.txt", 1, commit(0));
      scope.heard(answer, revision);
      await scope.whenLoaded();
      if (scope.state("a.txt")?.review.hunks[1] !== "seen") throw new Error("the scope did not follow the mark");
      const afterMark = reads;
      scope.heard(answer, revision);
      await scope.whenLoaded();
      if (reads !== afterMark || afterMark !== 1) throw new Error(`the scope read ${reads} times for one mark`);
      h.other.manifest = manifest;
    });
    await h.step("the changeset's scope takes a mark of its file as the store does", async () => {
      const whole = newScope(h.other, { type: "changeset" }, () => undefined);
      await whole.load();
      const revision = whole.currentShown()?.revision ?? -1;
      const answer = await h.other.toggleHunk(revision, "b.txt", 0, CHANGESET);
      whole.heard(answer, revision);
      if (whole.state("b.txt")?.review.hunks[0] !== "seen") throw new Error("the changeset's scope did not follow");
    });
    // A tab on the changeset while a commit is on screen comments through its own view, not the one on screen.
    await h.step(
      "a comment from the changeset's scope while a commit is on screen",
      async () => {
        const whole = newScope(h.other, { type: "changeset" }, () => undefined);
        await whole.load();
        const shown = whole.currentShown();
        if (!shown) throw new Error("the scope has not read");
        const target = { type: "file", side: "target", path: "a.txt" } as const;
        const sent = await h.asElement(
          () => threads.create(shown, target, "a file note", (s) => whole.admit(s)),
          shown,
        );
        if (!sent) throw new Error("the comment was refused");
      },
      ONE,
    );
  },

  /** A mark sent while the commit to show has not arrived is meant for the commit on screen. */
  async stepping(h) {
    await h.step("opened the branch by commit", () => reviewByCommit(h));
    await h.step(
      "mark while stepping",
      () => {
        const from = h.shown();
        void hunk(h, "a.txt", 1, from);
        void changeset.showCommit(2);
      },
      ONE,
    );
    await h.step(
      "flag while stepping back",
      () => {
        const from = h.shown();
        void flag(h, "a.txt", 0, from);
        void changeset.showCommit(0);
      },
      ONE,
    );
    // Once the next commit is asked for, a request built for the one on screen is refused.
    await h.step(
      "mark refused once the next commit is asked for",
      () => {
        const from = h.shown();
        void changeset.showCommit(1);
        void hunk(h, "a.txt", 0, from);
      },
      { notice: "Changes arrived — try again" },
    );
  },

  /** Another window marks and comments while this one shows the changeset, a range and a commit. */
  async otherWindow(h) {
    const mark = async (path: string, index: number) =>
      h.other.toggleHunk((await h.other.manifest(CHANGESET)).revision, path, index, CHANGESET);
    await h.step("other marks in the changeset", () => mark("b.txt", 0));
    await h.step("other flags in the changeset", async () => {
      const { revision } = await h.other.manifest(CHANGESET);
      await h.other.toggleFlag(revision, "b.txt", 1, CHANGESET);
    });
    await h.step("other comments", async () => {
      const { revision } = await h.other.manifest(CHANGESET);
      await h.other.createThread(revision, { type: "changeset" }, "from the other window", CHANGESET);
    });
    await h.step("version settled", () => settleVersion(h));
    await h.step("range shown", () => changeset.setRange(defaultRange(versions.items)));
    await h.step("other marks while in the range", () => mark("b.txt", 2));
    await h.step("back to the changeset", () => changeset.setRange(undefined));
    await h.step("opened the branch by commit", () => reviewByCommit(h));
    await h.step("other marks while by commit", () => mark("a.txt", 0));
  },

  /** The socket drops while another window marks; the window connects again and reads anew. */
  async reconnect(h) {
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), ONE);
    await h.step("other marks while the socket is down", async () => {
      const back = h.dropSocket();
      const { revision } = await h.other.manifest(CHANGESET);
      await h.other.toggleHunk(revision, "b.txt", 1, CHANGESET);
      await back;
    });
    await h.step("seen after reconnecting", () => hunk(h, "b.txt", 1), ONE);
  },

  /** The pull requests are asked of the platform once for a view of the list, and once more for a burst of events. */
  async listings(h) {
    let stop: (() => void) | undefined;
    await h.step(
      "the list is watched from an effect",
      () => {
        stop = watchFromEffect();
      },
      { listings: 1 },
    );
    await h.step(
      "a burst of events",
      () => {
        for (let i = 0; i < 10; i++) void queue.loadAll();
      },
      { listings: 2 },
    );
    await h.step("the list is not watched any more", () => stop?.());
  },

  /** Choosing rows of the queue puts changesets on view without asking the language servers for anything; the review screen does. */
  async queue(h) {
    const asked = () => h.sent.filter((r) => ANALYSES.has(r.type)).length;
    const told = (shown: boolean) => h.sent.filter((r) => r.type === "setReviewShown" && r.shown === shown).length;
    await h.step("rows chosen on the queue", async () => {
      // The files that are asked about are those a language serves, once the servers are known:
      // the fixture's own are served by none.
      await h.until(() => serverList.servers.length > 0, "the servers");
      h.fixture.write("d.rs", "wt", [3]);
      await h.until(() => changeset.paths.includes("d.rs"), "the Rust file in the working tree");
      await changeset.openChangeset({ type: "branch", name: "feature" });
      await h.until(() => changeset.facts?.target.changeset.type === "branch", "the branch");
      await changeset.openChangeset({ type: "workingTree" });
      await h.until(() => changeset.facts?.target.changeset.type === "workingTree", "the working tree");
      if (!problems.analysable.includes("d.rs")) throw new Error("the changeset has no file to analyse");
      await sleep(500);
      if (asked() || told(true)) throw new Error("the queue asked the language servers for something");
    });
    await h.step("review screen shown", async () => {
      reviewShown.set(true);
      await h.until(() => told(true) === 1, "the backend to be told");
      await h.until(() => h.sent.some((r) => r.type === "fileSymbols"), "the symbols");
    });
    await h.step("back on the queue", async () => {
      reviewShown.set(false);
      await h.until(() => told(false) === 1, "the backend to be told");
      const before = asked();
      await changeset.openChangeset({ type: "branch", name: "feature" });
      await sleep(500);
      if (asked() !== before) throw new Error("the queue asked the language servers for something");
    });
  },

  /** Toggles sent without waiting for each other. */
  async rapid(h) {
    await h.step(
      "twenty toggles of one hunk",
      () => {
        for (let i = 0; i < 20; i++) void hunk(h, "b.txt", 0);
      },
      { marks: 20 },
    );
    await h.step(
      "toggles across files",
      () => {
        for (const [path, index] of [
          ["a.txt", 0],
          ["b.txt", 1],
          ["a.txt", 3],
          ["b.txt", 0],
        ] as const) {
          void hunk(h, path, index);
          void flag(h, path, index);
        }
      },
      { marks: 8 },
    );
  },
};
