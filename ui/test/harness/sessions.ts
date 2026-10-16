// The scripted sessions. Each runs in a process and backend of its own; a step ends when the window has
// settled, and then the stores are compared with the backend.

import type { ChangesetKey } from "../../src/generated/ChangesetKey.ts";
import type { View } from "../../src/generated/View.ts";
import type { Request } from "../../src/generated/Request.ts";
import { appState } from "../../src/app/appState.svelte.ts";
import { findInFiles } from "../../src/features/search/FindInFiles.svelte.ts";
import { changeset } from "../../src/state/changeset.svelte.ts";
import { keyId } from "../../src/state/changesetKey.ts";
import { opened } from "../../src/state/opened.svelte.ts";
import { shell } from "../../src/features/chrome/shell.svelte.ts";
import { preferences } from "../../src/lib/preferences.svelte.ts";
import { problems } from "../../src/state/problems.svelte.ts";
import { session } from "../../src/state/session.svelte.ts";
import { structure } from "../../src/state/structure.svelte.ts";
import { queue } from "../../src/state/queue.svelte.ts";
import { reviewShown } from "../../src/state/reviewShown.svelte.ts";
import { serverList } from "../../src/state/serverList.svelte.ts";
// Holds the changeset's symbols, which the backend pushes while it is shown.
import { symbols } from "../../src/state/symbols.svelte.ts";
import { threads } from "../../src/state/threads.svelte.ts";
import { defaultRange } from "../../src/state/timelineRange.ts";
import type { ViewChange } from "../../src/state/viewChange.ts";
import { versions } from "../../src/state/versions.svelte.ts";
import { newScope } from "../../src/state/viewScope.svelte.ts";
import type { Harness } from "./harness.ts";
import { type SessionName } from "./sessionNames.ts";

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

/** Records what the store tells the review screen changed about the view; `heard` takes what came since it was last asked. */
function listenForViewChanges() {
  const heard: ViewChange[] = [];
  changeset.onViewChange((_, change) => heard.push(change));
  return () => heard.splice(0);
}

/** Throws unless `heard` is exactly `expected`. */
function expectChanges(heard: ViewChange[], expected: ViewChange[]) {
  if (JSON.stringify(heard) !== JSON.stringify(expected))
    throw new Error(`the view changes were ${JSON.stringify(heard)}, not ${JSON.stringify(expected)}`);
}

/** A new version of the working tree, as an edit on disk makes it. */
async function settleVersion(h: Harness) {
  h.fixture.write("b.txt", "wt2", [7, 25, 33]);
  await h.until(() => versions.items.length >= 2 && changeset.facts?.review.state === "open", "the second version");
}

export const sessions: Record<SessionName, (h: Harness) => Promise<void>> = {
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

  /** A push to the branch on view: what was seen before it is remapped onto the lines it left. */
  async push(h) {
    await changeset.openChangeset({ type: "branch", name: "feature" });
    await h.until(() => changeset.facts?.target.changeset.type === "branch", "the branch");
    await h.step("a file seen before the push", () => file(h, "a.txt"), ONE);
    await h.step("the branch is pushed to", async () => {
      const before = changeset.layout?.shown.revision ?? -1;
      h.fixture.push("a.txt", "p1", [2, 10]);
      await h.until(() => (changeset.layout?.shown.revision ?? -1) > before, "the new revision");
    });
  },

  /** A new version, the range since the last review, marks in it, and back. */
  async range(h) {
    const heard = listenForViewChanges();
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), ONE);
    heard();
    await h.step("version settled", async () => {
      await settleVersion(h);
      const changes = heard();
      if (!changes.includes("revision") || changes.includes("start"))
        throw new Error(`a new version told ${JSON.stringify(changes)}, not a newer revision`);
    });
    await h.step("range shown", async () => {
      await changeset.setRange(defaultRange(versions.items));
      expectChanges(heard(), ["start"]);
    });
    await h.step("flagged in range", () => flag(h, first(), 0), ONE);
    await h.step("seen in range", () => hunk(h, first(), 1), ONE);
    await h.step("unseen in range", () => hunk(h, first(), 1), ONE);
    expectChanges(heard(), []);
    await h.step("back to the changeset", async () => {
      await changeset.setRange(undefined);
      expectChanges(heard(), ["start"]);
    });
    await h.step("seen in the new version", () => hunk(h, "b.txt", 2), ONE);
    await h.step(
      "a range of versions that are gone gives way to the changeset",
      async () => {
        await changeset.setRange({ from: 98, to: 99 });
        if (changeset.range) throw new Error("the range stayed");
      },
      { notice: "no such version 98", rejected: 1 },
    );
    await h.step(
      "a range the backend calls invalid is not replaced by the changeset",
      async () => {
        await changeset.setRange({ from: 1, to: 1 });
        const stayed = changeset.range !== undefined;
        await changeset.setRange(undefined);
        if (!stayed) throw new Error("the range was dropped");
      },
      { notice: "an interdiff compares an earlier version with a later one", rejected: 1 },
    );
  },

  /**
   * The whole changeset is read once per revision: a burst of marks in a range or in a commit reads
   * nothing of it, and a settle reads it once.
   */
  async whole(h) {
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), ONE);
    await h.step("version settled", () => settleVersion(h));
    await h.step("a range is shown", () => changeset.setRange(defaultRange(versions.items)));
    await h.step(
      "a burst of marks in a range sends no whole-changeset fetch",
      () => {
        void hunk(h, first(), 0);
        void hunk(h, first(), 1);
      },
      { marks: 2, whole: 0 },
    );
    await h.step("opened the branch by commit", () => reviewByCommit(h), { commits: 1 });
    await h.step(
      "a burst of marks in a commit sends no whole-changeset fetch",
      () => {
        void hunk(h, first(), 0);
        void flag(h, first(), 1);
      },
      { marks: 2, whole: 0 },
    );
    await h.step(
      "a settle sends one whole-changeset fetch",
      async () => {
        const before = changeset.layout?.shown.revision ?? -1;
        h.fixture.push("a.txt", "p9", [2, 10, 30]);
        await h.until(() => (changeset.layout?.shown.revision ?? -1) > before, "the new revision");
      },
      { whole: 1 },
    );
  },

  /** The changeset reviewed by commit: marks, stepping, and the lens back. The store reads the commits again on each mark and view change. */
  async commits(h) {
    const heard = listenForViewChanges();
    await h.step("opened the branch by commit", () => reviewByCommit(h), { commits: 1 });
    heard();
    await h.step("seen in a commit", () => hunk(h, first(), 0), { ...ONE, commits: 2 });
    await h.step(
      "next commit",
      async () => {
        await changeset.showCommit(1);
        expectChanges(heard(), ["start"]);
      },
      { commits: 1 },
    );
    await h.step("flagged in a commit", () => flag(h, first(), 0), { ...ONE, commits: 2 });
    await h.step("file seen in a commit", () => file(h, first()), { ...ONE, commits: 2 });
    await h.step("last commit", () => changeset.showCommit(2), { commits: 1 });
    await h.step("lens back to the whole", () => changeset.showLens("squash"), { commits: 0 });
    await h.step("seen in the changeset", () => hunk(h, "a.txt", 0), { ...ONE, commits: 0 });
    await h.step("by commit again", () => changeset.showLens("perCommit"), { commits: 1 });
  },

  /**
   * The Commits tool and the review lens are one choice: choosing the tool asks for the by-commit
   * lens and the rail shows it, leaving asks for all changes, and a lens set by the backend selects
   * the tool. One lens request goes out per choice, none when the backend set the lens.
   */
  async commitsTool(h) {
    const asked = () => h.sent.filter((r) => r.type === "setLens").length;
    await h.step("the branch is opened", async () => {
      await changeset.openChangeset({ type: "branch", name: "feature" });
      await h.until(() => changeset.facts?.target.changeset.type === "branch", "the branch");
    });
    await h.step("the Commits tool is chosen", async () => {
      const before = asked();
      shell.toggle("commits");
      await h.until(
        () => changeset.lens === "perCommit" && changeset.shownCommit !== undefined,
        "the by-commit review",
      );
      if (shell.left.tool !== "commits") throw new Error(`the rail shows ${shell.left.tool}, not Commits`);
      if (asked() - before !== 1)
        throw new Error(`choosing the Commits tool asked for the lens ${asked() - before} times`);
    });
    await h.step("another tool is chosen", async () => {
      const before = asked();
      shell.toggle("all");
      await h.until(() => changeset.lens === "squash" && changeset.shownCommit === undefined, "the whole changeset");
      if (shell.left.tool !== "all") throw new Error(`the rail shows ${shell.left.tool}, not All Files`);
      if (asked() - before !== 1)
        throw new Error(`leaving the Commits tool asked for the lens ${asked() - before} times`);
    });
    await h.step("the backend sets the lens", async () => {
      const before = asked();
      await h.other.setLens("perCommit");
      await h.until(
        () => changeset.lens === "perCommit" && changeset.shownCommit !== undefined,
        "the by-commit review",
      );
      if (shell.left.tool !== "commits") throw new Error(`the rail shows ${shell.left.tool}, not Commits`);
      if (asked() - before !== 0) throw new Error("the window asked for the lens the backend set");
    });
    await h.step("the backend returns the lens to all changes", async () => {
      const before = asked();
      await h.other.setLens("squash");
      await h.until(() => changeset.lens === "squash" && changeset.shownCommit === undefined, "the whole changeset");
      if (shell.left.tool !== "all") throw new Error(`the rail shows ${shell.left.tool}, not the tool chosen last`);
      if (asked() - before !== 0) throw new Error("the window asked for the lens the backend set");
    });
    await h.step("the lens is remembered for the branch", async () => {
      const before = asked();
      await h.other.setLens("perCommit");
      await changeset.openChangeset({ type: "workingTree" });
      await h.until(() => changeset.facts?.target.changeset.type === "workingTree", "the working tree");
      await changeset.openChangeset({ type: "branch", name: "feature" });
      await h.until(() => changeset.lens === "perCommit" && changeset.shownCommit !== undefined, "the remembered lens");
      if (shell.left.tool !== "commits") throw new Error(`the rail shows ${shell.left.tool}, not Commits`);
      if (asked() - before !== 0) throw new Error("opening a remembered lens asked for the lens");
    });
    await h.step("a changeset of one commit cannot be reviewed by commit", async () => {
      h.fixture.resetHeadTo("main");
      h.fixture.commitWorkingTree();
      await changeset.openChangeset({ type: "branch", name: "feature" });
      await h.until(() => changeset.facts?.commits === 1, "the one-commit branch");
      await h.other.setLens("squash");
      await h.until(() => changeset.lens === "squash", "the whole changeset");
      const before = asked();
      shell.toggle("commits");
      if (shell.left.tool !== "changes") throw new Error(`the rail shows ${shell.left.tool}, not Changes`);
      if (asked() - before !== 0)
        throw new Error(`choosing the Commits tool asked for the lens ${asked() - before} times`);
      if (changeset.lens !== "squash") throw new Error(`the lens became ${changeset.lens}`);
    });
  },

  /** A scope of a commit that is not the one on screen has that commit's files and marks, and keeps them while the window moves on. Its commits come from the store's list, not a read of its own. */
  async pinned(h) {
    await h.step("opened the branch by commit", () => reviewByCommit(h), { commits: 1 });
    const commit = (index: number): View => ({ type: "commit", index });
    const id = changeset.commits[0].commit.id;
    const scope = newScope(
      h.other,
      { type: "commit", id },
      () => undefined,
      changeset.commitList,
      () => changeset.currentShown()?.revision,
    );
    const hunksOf = (layout: { files: readonly { path: string; hunks: readonly unknown[] }[] } | undefined) =>
      layout?.files.find((f) => f.path === "a.txt")?.hunks.length;
    await h.step(
      "another commit on screen",
      async () => {
        const before = changeset.commits;
        await changeset.showCommit(2);
        // Wait for the store's list to be of the revision on screen, so the scope reuses it rather than racing its read.
        await h.until(() => changeset.commits !== before, "the commits of the commit on screen");
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
      },
      { commits: 1 },
    );
    await h.step(
      "a mark in the scope",
      async () => {
        const shown = scope.currentShown();
        if (!shown) throw new Error("the scope has not read");
        await scope.toggleHunk(shown, "a.txt", 0);
        const own = await h.other.manifest(commit(0));
        const held = scope.state("a.txt")?.review.hunks;
        const real = own.files.find((f) => f.path === "a.txt")?.review.hunks;
        if (held?.[0] !== "seen" || JSON.stringify(held) !== JSON.stringify(real))
          throw new Error("the scope does not hold the mark");
      },
      { commits: 1 },
    );
    await h.step(
      "a mark made elsewhere is read once, its echo is not read again",
      async () => {
        let reads = 0;
        const manifest = h.other.manifest.bind(h.other);
        h.other.manifest = (view) => {
          reads++;
          return manifest(view);
        };
        const revision = scope.currentShown()?.revision ?? -1;
        const before = changeset.commits;
        const answer = await h.other.toggleHunk(revision, "a.txt", 1, commit(0));
        await h.until(() => changeset.commits !== before, "the commits after the mark");
        scope.heard(answer, revision);
        await scope.whenLoaded();
        if (scope.state("a.txt")?.review.hunks[1] !== "seen") throw new Error("the scope did not follow the mark");
        const afterMark = reads;
        scope.heard(answer, revision);
        await scope.whenLoaded();
        if (reads !== afterMark || afterMark !== 1) throw new Error(`the scope read ${reads} times for one mark`);
        h.other.manifest = manifest;
      },
      { commits: 1 },
    );
    await h.step(
      "the whole-changeset view takes a mark of its file as the store does",
      async () => {
        const whole = changeset.whole;
        const revision = whole.currentShown()?.revision ?? -1;
        const answer = await h.other.toggleHunk(revision, "b.txt", 0, CHANGESET);
        whole.heard(answer, revision);
        if (whole.state("b.txt")?.review.hunks[0] !== "seen")
          throw new Error("the whole-changeset view did not follow");
      },
      { commits: 1 },
    );
    // A tab on the changeset while a commit is on screen comments through the whole view, not the one on screen.
    await h.step(
      "a comment from the whole-changeset view while a commit is on screen",
      async () => {
        const whole = changeset.whole;
        const shown = whole.currentShown();
        if (!shown) throw new Error("the whole changeset has not read");
        const target = { type: "file", side: "target", path: "a.txt" } as const;
        const sent = await h.asElement(
          () => threads.create(shown, target, "a file note", (s) => whole.admit(s)),
          shown,
        );
        if (!sent) throw new Error("the comment was refused");
      },
      { ...ONE, commits: 0 },
    );
    // A background read that races the changeset moving on keeps the scope and says nothing; the
    // store's reload reads the newer revision and brings the scope up to date then.
    await h.step("a scope read that races a revision change is silent and catches up on the next read", async () => {
      let revision = changeset.currentShown()?.revision ?? -1;
      const racing = newScope(
        h.other,
        { type: "commit", id },
        () => undefined,
        changeset.commitList,
        () => revision,
      );
      await racing.load();
      const wasRevision = racing.currentShown()?.revision ?? -1;
      if (wasRevision !== revision) throw new Error("the racing scope did not read its revision");
      h.fixture.push("a.txt", "p9", [2, 10, 30]);
      await h.until(() => (changeset.layout?.shown.revision ?? -1) > wasRevision, "the changeset to move on");
      // The scope reads at the old revision, which the backend has moved past: it keeps its view and says nothing.
      await racing.load();
      if (racing.currentShown()?.revision !== wasRevision) throw new Error("the racing read replaced the scope's view");
      // The store's reload reads at the revision now on view, and the scope catches up with it.
      revision = changeset.currentShown()?.revision ?? -1;
      await racing.load();
      if (revision <= wasRevision) throw new Error("the changeset did not move to a newer revision");
      if ((racing.currentShown()?.revision ?? -1) !== revision) throw new Error("the scope did not catch up");
    });
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

  /**
   * The pull request list is read when the queue is shown and on a refresh, and never by opening a
   * changeset or starting a review.
   */
  async listings(h) {
    await h.step("the queue is shown", () => queue.show(), { listings: 1 });
    await h.step(
      "a changeset is opened",
      async () => {
        await changeset.openChangeset({ type: "branch", name: "feature" });
        await h.until(() => changeset.facts?.target.changeset.type === "branch", "the branch");
      },
      { listings: 0 },
    );
    await h.step(
      "a review is started",
      async () => {
        await h.until(() => changeset.paths.length > 0, "the branch's files");
        await hunk(h, first(), 0);
        await h.until(() => changeset.facts?.review.state === "open", "the review to start");
      },
      { marks: 1, listings: 0 },
    );
    await h.step("the queue is refreshed", () => queue.refresh(), { listings: 1 });
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
      // The backend analyses the file itself and pushes the answer; the store applies it.
      const answered = () => symbols.files.get("d.rs")?.state;
      await h.until(() => answered() === "done" || answered() === "failed", "the symbols");
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

  /** What the window remembers reaches the backend, another window's changes are followed, and a connection that dropped reads them anew. */
  async preferences(h) {
    const kept = async (scope: string | null, key: string) =>
      (await h.other.preferences(scope)).find((p) => p.key === key)?.value;
    const eventually = async (what: string, read: () => Promise<unknown>, want: unknown) => {
      for (let i = 0; i < 200; i++) {
        if ((await read()) === want) return;
        await sleep(25);
      }
      throw new Error(`timed out waiting for ${what}`);
    };
    await h.step("a change here is the backend's", async () => {
      structure.setSort(true);
      preferences.write("project", "tabs", "[1]");
      await eventually("the app preference", () => kept(null, "structure.sort"), "true");
      await eventually("the project preference", () => kept(session.repo ?? null, "tabs"), "[1]");
    });
    await h.step("a change in another window is followed", async () => {
      await h.other.setPreference(null, "structure.sort", undefined);
      await h.until(() => !structure.sort, "the sort to be forgotten");
      await h.other.setPreference(session.repo ?? null, "tabs", "[2]");
      await h.until(() => preferences.read("project", "tabs") === "[2]", "the project preference");
    });
    await h.step("preferences changed while the socket is down are read anew", async () => {
      const back = h.dropSocket();
      await h.other.setPreference(null, "structure.changedOnly", "true");
      await back;
      await h.until(() => structure.changedOnly, "the preference after reconnecting");
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
    await h.step("changesets asked for one after another without waiting", async () => {
      const feature: ChangesetKey = { type: "branch", name: "feature" };
      const tree: ChangesetKey = { type: "workingTree" };
      for (const key of [tree, feature, tree, feature, tree]) void changeset.openChangeset(key);
      await changeset.openChangeset(feature);
      await h.until(() => opened.session !== undefined && !opened.loading, "the last changeset");
      // What the invariants then compare with the backend is the changeset it has on view, whichever of the
      // requests it served last: nothing of the changesets passed through is left in it.
      const key = opened.session?.key;
      if (!key || keyId(key) !== keyId(changeset.facts?.target.changeset ?? tree))
        throw new Error("the session on view is not of the changeset the manifest is of");
    });
    await h.step(
      "a mark answered after its changeset was switched away from",
      async () => {
        const backend = session.connected();
        const toggle = backend.toggleHunk.bind(backend);
        let arrived = false;
        let release!: () => void;
        const held = new Promise<void>((resolve) => (release = resolve));
        // The answer of the changeset on view comes after the next one is on view, as a failure.
        backend.toggleHunk = async (...args) => {
          await toggle(...args);
          arrived = true;
          await held;
          throw new Error("a late failure");
        };
        const marking = hunk(h, "b.txt", 0);
        await h.until(() => arrived, "the mark to be answered");
        await changeset.openChangeset({ type: "workingTree" });
        await h.until(() => opened.session !== undefined && !opened.loading, "the next changeset");
        release();
        await marking;
        backend.toggleHunk = toggle;
      },
      ONE,
    );
  },

  /** The dialogs of a project are put away when the project closes and another opens. */
  async projectDialogs(h) {
    await h.step("the project's dialogs are open", () => {
      findInFiles.show({ query: "line" });
      appState.dialog = "finish";
      appState.switcherOpen = true;
    });
    await session.closeRepo();
    await h.until(() => session.repo === undefined, "the project to close");
    await session.openRepo(h.fixture.anotherRepo());
    await h.until(() => session.repo !== undefined, "the next project to open");
    if (findInFiles.open) throw new Error("Find in Files is still open in the next project");
    if (appState.dialog !== undefined)
      throw new Error(`the ${appState.dialog} dialog is still open in the next project`);
    if (appState.switcherOpen) throw new Error("the changeset switcher is still open in the next project");
  },
};
