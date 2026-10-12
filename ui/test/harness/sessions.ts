// The scripted sessions. Each runs in a process and backend of its own; a step ends when the window has
// settled, and then the stores are compared with the backend.

import type { View } from "../../src/generated/View.ts";
import { changeset } from "../../src/state/changeset.svelte.ts";
import { threads } from "../../src/state/threads.svelte.ts";
import { defaultRange } from "../../src/state/timelineRange.ts";
import { versions } from "../../src/state/versions.svelte.ts";
import type { Harness } from "./harness.ts";

const CHANGESET: View = { type: "changeset" };

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
    // Only the working tree's base follows its branch's upstream, which the fetch brings forward.
    await h.step("fetched", async () => {
      const behind = changeset.base?.behind;
      h.fixture.advanceOrigin();
      await changeset.fetch();
      await h.until(() => changeset.base?.behind !== behind, "the base to be behind after the fetch");
    });
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
