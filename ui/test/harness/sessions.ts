// The scripted sessions. Each runs in a process and backend of its own; a step ends when the window has
// settled, and then the stores are compared with the backend.

import type { ChangesetKey } from "../../src/generated/ChangesetKey.ts";
import type { CommitChecks } from "../../src/generated/CommitChecks.ts";
import type { PullRequestInfo } from "../../src/generated/PullRequestInfo.ts";
import type { ServerMessage } from "../../src/generated/ServerMessage.ts";
import type { SymbolInfo } from "../../src/generated/SymbolInfo.ts";
import type { View } from "../../src/generated/View.ts";
import type { Request } from "../../src/generated/Request.ts";
import { appState } from "../../src/app/appState.svelte.ts";
import { discardPrompt } from "../../src/state/discardPrompt.svelte.ts";
import { findInFiles } from "../../src/features/search/FindInFiles.svelte.ts";
import { keyId } from "../../src/state/changesetKey.ts";
import type { ChangesetSession } from "../../src/state/changesetSession.svelte.ts";
import { STALE_NOTICE } from "../../src/lib/notice.ts";
import type { SessionEvent } from "../../src/state/backend.ts";
import { newSession } from "../../src/state/changesetSession.svelte.ts";
import type { UsagesSpec } from "../../src/state/findSets.svelte.ts";
import { opened } from "../../src/state/opened.svelte.ts";
import { excludePath } from "../../src/state/projectRequests.ts";
import { platform } from "../../src/state/platform.svelte.ts";
import { shell } from "../../src/features/chrome/shell.svelte.ts";
import { preferences } from "../../src/lib/preferences.svelte.ts";
import { problems } from "../../src/state/problems.svelte.ts";
import { progress } from "../../src/state/progress.svelte.ts";
import { session } from "../../src/state/session.svelte.ts";
import { structure } from "../../src/state/structure.svelte.ts";
import { queue } from "../../src/state/queue.svelte.ts";
import { reviewShown } from "../../src/state/reviewShown.svelte.ts";
import { serverList } from "../../src/state/serverList.svelte.ts";
import { defaultRange, rangeView } from "../../src/state/timelineRange.ts";
import type { ViewChange } from "../../src/state/viewChange.ts";
import { newScope } from "../../src/state/viewScope.svelte.ts";
import type { Harness, RequestMap } from "./harness.ts";
import { onView } from "./onView.ts";
import { type SessionName } from "./sessionNames.ts";

const CHANGESET: View = { type: "changeset" };

/** The requests that start the language servers or ask them. */
const ANALYSES = new Set<Request["type"]>(["diagnostics", "fileSymbols", "fileOutline", "symbolAt"]);

/** The requests for what only the review screen shows. */
const REVIEW_ONLY = new Set<Request["type"]>(["versions", "commits", "pipelineSteps"]);

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** A step that sends one request of `type` and no other review request. */
const once = (type: Request["type"], more: RequestMap = {}) => ({ sent: { [type]: 1, ...more } });

/** Marks or flags as a click in the views on screen does, or in the ones built for `from`. */
const hunk = (h: Harness, path: string, index: number, from = h.shown()) =>
  h.asElement(() => onView().toggleHunk(from, path, index), from);
const flag = (h: Harness, path: string, index: number, from = h.shown()) =>
  h.asElement(() => onView().toggleFlag(from, path, index), from);
const file = (h: Harness, path: string) => h.asElement(() => onView().toggleFileSeen(h.shown(), path));
const directory = (h: Harness, dir: string, seen: boolean) =>
  h.asElement(() => onView().setDirectorySeen(h.shown(), dir, seen));

/** The first file of the view on screen. */
const first = () => (opened.session?.paths ?? [])[0];

async function reviewByCommit(h: Harness) {
  await opened.open({ type: "branch", name: "feature" });
  await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
  await onView().showLens("perCommit");
  await h.until(() => opened.session?.shownCommit !== undefined, "the first commit");
}

/** Records what the sessions on view tell the review screen changed about the view; `heard` takes what came since it was last asked. */
function listenForViewChanges() {
  const heard: ViewChange[] = [];
  const listen = (session: ChangesetSession | undefined) => session?.onViewChange((_, change) => heard.push(change));
  listen(opened.session);
  opened.onSession(listen);
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
  await h.until(
    () => (opened.session?.versions.items.length ?? 0) >= 2 && opened.session?.facts?.review.state === "open",
    "the second version",
  );
}

export const sessions: Record<SessionName, (h: Harness) => Promise<void>> = {
  /** Marks, flags and unmarks in the changeset view. */
  async changeset(h) {
    await h.step("opened", () => undefined);
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), once("toggleHunk"));
    await h.step("hunk flagged", () => flag(h, "b.txt", 1), once("toggleFlag"));
    await h.step("hunk unseen", () => hunk(h, "b.txt", 0), once("toggleHunk"));
    await h.step("file seen", () => file(h, "a.txt"), once("toggleFileSeen"));
    await h.step("folder seen", () => directory(h, "src", true), once("setFilesSeen"));
    await h.step("folder unseen", () => directory(h, "src", false), once("setFilesSeen"));
    await h.step(
      "thread opened",
      () =>
        h.asElement(() => onView().threads.create(h.shown(), { type: "changeset" }, "a note on the whole changeset")),
      once("createThread"),
    );
  },

  /** A push to the branch on view: what was seen before it is remapped onto the lines it left. */
  async push(h) {
    await opened.open({ type: "branch", name: "feature" });
    await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
    await h.step("a file seen before the push", () => file(h, "a.txt"), once("toggleFileSeen"));
    await h.step("the branch is pushed to", async () => {
      const before = opened.session?.layout?.shown.revision ?? -1;
      h.fixture.push("a.txt", "p1", [2, 10]);
      await h.until(() => (opened.session?.layout?.shown.revision ?? -1) > before, "the new revision");
    });
  },

  /** A new version, the range since the last review, marks in it, and back. */
  async range(h) {
    const heard = listenForViewChanges();
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), once("toggleHunk"));
    heard();
    // A thread on a file the range does not touch: the changeset has it, the range does not.
    await h.step(
      "thread opened on a file",
      () =>
        h.asElement(() =>
          onView().threads.create(h.shown(), { type: "file", side: "target", path: "a.txt" }, "a note on a.txt"),
        ),
      once("createThread"),
    );
    await h.step("version settled", async () => {
      await settleVersion(h);
      const changes = heard();
      if (!changes.includes("revision") || changes.includes("start"))
        throw new Error(`a new version told ${JSON.stringify(changes)}, not a newer revision`);
    });
    await h.step("range shown", async () => {
      await onView().setRange(defaultRange(onView().versions.items));
      expectChanges(heard(), ["start"]);
      // The invariants tell the changeset's threads from the range's only if the backend does.
      const range = onView().range;
      const [inChangeset, inRange] = await Promise.all([h.other.threads(CHANGESET), h.other.threads(rangeView(range))]);
      if (inChangeset.threads.length === inRange.threads.length)
        throw new Error("the range lists the changeset's threads: the step cannot tell them apart");
    });
    await h.step("flagged in range", () => flag(h, first(), 0), once("toggleFlag"));
    await h.step("seen in range", () => hunk(h, first(), 1), once("toggleHunk"));
    await h.step("unseen in range", () => hunk(h, first(), 1), once("toggleHunk"));
    expectChanges(heard(), []);
    await h.step("back to the changeset", async () => {
      await onView().setRange(undefined);
      expectChanges(heard(), ["start"]);
    });
    await h.step("seen in the new version", () => hunk(h, "b.txt", 2), once("toggleHunk"));
    await h.step(
      "a range of versions that are gone gives way to the changeset",
      async () => {
        await onView().setRange({ from: 98, to: 99 });
        if (opened.session?.range) throw new Error("the range stayed");
      },
      { notice: "no such version 98", rejected: 1 },
    );
    await h.step(
      "a range the backend calls invalid is not replaced by the changeset",
      async () => {
        await onView().setRange({ from: 1, to: 1 });
        const stayed = opened.session?.range !== undefined;
        await onView().setRange(undefined);
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
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), once("toggleHunk"));
    await h.step("version settled", () => settleVersion(h));
    await h.step("a range is shown", () => onView().setRange(defaultRange(onView().versions.items)));
    await h.step(
      "a burst of marks in a range sends no whole-changeset fetch",
      () => {
        void hunk(h, first(), 0);
        void hunk(h, first(), 1);
      },
      { sent: { toggleHunk: 2, "manifest(changeset)": 0 } },
    );
    await h.step("opened the branch by commit", () => reviewByCommit(h), { sent: { commits: 1 } });
    await h.step(
      "a burst of marks in a commit sends no whole-changeset fetch",
      () => {
        void hunk(h, first(), 0);
        void flag(h, first(), 1);
      },
      { sent: { toggleHunk: 1, toggleFlag: 1, "manifest(changeset)": 0 } },
    );
    await h.step(
      "a settle sends one whole-changeset fetch",
      async () => {
        const before = opened.session?.layout?.shown.revision ?? -1;
        h.fixture.push("a.txt", "p9", [2, 10, 30]);
        await h.until(() => (opened.session?.layout?.shown.revision ?? -1) > before, "the new revision");
      },
      { sent: { "manifest(changeset)": 1 } },
    );
  },

  /** The changeset reviewed by commit: marks, stepping, and the lens back. The store reads the commits again on each mark and view change. */
  async commits(h) {
    const heard = listenForViewChanges();
    await h.step("opened the branch by commit", () => reviewByCommit(h), { sent: { commits: 1 } });
    heard();
    await h.step("seen in a commit", () => hunk(h, first(), 0), once("toggleHunk", { commits: 1 }));
    await h.step(
      "next commit",
      async () => {
        await onView().showCommit(1);
        expectChanges(heard(), ["start"]);
      },
      { sent: { commits: 1 } },
    );
    await h.step("flagged in a commit", () => flag(h, first(), 0), once("toggleFlag", { commits: 1 }));
    await h.step("file seen in a commit", () => file(h, first()), once("toggleFileSeen", { commits: 1 }));
    await h.step("last commit", () => onView().showCommit(2), { sent: { commits: 1 } });
    await h.step("lens back to the whole", () => onView().showLens("squash"), { sent: { commits: 0 } });
    await h.step("seen in the changeset", () => hunk(h, "a.txt", 0), once("toggleHunk", { commits: 0 }));
    await h.step("by commit again", () => onView().showLens("perCommit"), { sent: { commits: 1 } });
  },

  /**
   * The requests the window sends after each kind of change, pinned exactly: what a mark, a settle, a
   * narrowing, an excluded path and the end of a review cost, in the changeset view and by commit. The
   * first mark of a review starts it and is not pinned.
   */
  async requests(h) {
    await h.step("the review is started", () => hunk(h, "b.txt", 0), once("toggleHunk"));
    // What a change of the files in view makes the window read, in the changeset view.
    const moved = { "manifest(changeset)": 1, threads: 1, versions: 1, listChangesets: 1 };
    // And by commit: the commit's manifest, the commits' progress and the changeset's manifest too.
    const movedByCommit = { ...moved, "manifest(commit)": 1, commits: 1 };
    const finishedNotice = "Review finished with unseen changes left";
    await h.step("a repeated mark in the changeset view", () => hunk(h, "b.txt", 1), {
      sent: { toggleHunk: 1 },
      exact: true,
    });
    await h.step(
      "a thread in the changeset view",
      () => h.asElement(() => onView().threads.create(h.shown(), { type: "changeset" }, "a note")),
      { sent: { createThread: 1, threads: 1, listChangesets: 1 }, exact: true },
    );
    await h.step("a settle in the changeset view", () => settleVersion(h), { sent: moved, exact: true });
    await h.step("the first mark of a version", () => hunk(h, "b.txt", 2), {
      sent: { toggleHunk: 1, versions: 1, listChangesets: 1 },
      exact: true,
    });
    await h.step("a narrowing in the changeset view", () => onView().reviewOnly("src"), {
      sent: { narrowReview: 1, ...moved },
      exact: true,
    });
    await h.step("the narrowing ended", () => onView().narrow(undefined), {
      sent: { narrowReview: 1, ...moved },
      exact: true,
    });
    await h.step("a path excluded in the changeset view", () => excludePath("src/c.txt", false), {
      sent: { excludePath: 1, ...moved },
      exact: true,
    });
    await h.step("the review is finished in the changeset view", () => onView().finishReview(), {
      sent: {
        finishReview: 1,
        "manifest(changeset)": 1,
        threads: 1,
        versions: 1,
        listChangesets: 2,
        changesetLines: 1,
      },
      exact: true,
      notice: finishedNotice,
    });
    await h.step("the branch is opened by commit", () => reviewByCommit(h));
    await h.step("the review by commit is started", () => hunk(h, first(), 0), once("toggleHunk"));
    await h.step("a repeated mark by commit", () => hunk(h, first(), 1), {
      sent: { toggleHunk: 1, commits: 1 },
      exact: true,
    });
    await h.step(
      "a settle by commit",
      async () => {
        const before = opened.session?.layout?.shown.revision ?? -1;
        h.fixture.push("a.txt", "p9", [2, 10, 30]);
        await h.until(() => (opened.session?.layout?.shown.revision ?? -1) > before, "the new revision");
      },
      { sent: { ...movedByCommit, changesetLines: 1 }, exact: true },
    );
    await h.step("a path excluded by commit", () => excludePath("b.txt", false), {
      sent: { excludePath: 1, ...movedByCommit },
      exact: true,
    });
    await h.step("the review is finished by commit", () => onView().finishReview(), {
      sent: {
        finishReview: 1,
        "manifest(commit)": 1,
        "manifest(changeset)": 1,
        commits: 1,
        threads: 1,
        versions: 1,
        listChangesets: 2,
        changesetLines: 1,
      },
      exact: true,
      notice: finishedNotice,
    });
  },

  /**
   * The Commits tool and the review lens are one choice: choosing the tool asks for the by-commit
   * lens and the rail shows it, leaving asks for all changes, and a lens set by the backend selects
   * the tool. One lens request goes out per choice, none when the backend set the lens.
   */
  async commitsTool(h) {
    const asked = () => h.sent.filter((r) => r.type === "setLens").length;
    await h.step("the branch is opened", async () => {
      await opened.open({ type: "branch", name: "feature" });
      await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
    });
    await h.step("the Commits tool is chosen", async () => {
      const before = asked();
      shell.toggle("commits");
      await h.until(
        () => opened.session?.lens === "perCommit" && opened.session.shownCommit !== undefined,
        "the by-commit review",
      );
      if (shell.left.tool !== "commits") throw new Error(`the rail shows ${shell.left.tool}, not Commits`);
      if (asked() - before !== 1)
        throw new Error(`choosing the Commits tool asked for the lens ${asked() - before} times`);
    });
    await h.step("another tool is chosen", async () => {
      const before = asked();
      shell.toggle("all");
      await h.until(
        () => opened.session?.lens === "squash" && opened.session.shownCommit === undefined,
        "the whole changeset",
      );
      if (shell.left.tool !== "all") throw new Error(`the rail shows ${shell.left.tool}, not All Files`);
      if (asked() - before !== 1)
        throw new Error(`leaving the Commits tool asked for the lens ${asked() - before} times`);
    });
    await h.step("the backend sets the lens", async () => {
      const before = asked();
      await h.other.setLens("perCommit");
      await h.until(
        () => opened.session?.lens === "perCommit" && opened.session.shownCommit !== undefined,
        "the by-commit review",
      );
      if (shell.left.tool !== "commits") throw new Error(`the rail shows ${shell.left.tool}, not Commits`);
      if (asked() - before !== 0) throw new Error("the window asked for the lens the backend set");
    });
    await h.step("the backend returns the lens to all changes", async () => {
      const before = asked();
      await h.other.setLens("squash");
      await h.until(
        () => opened.session?.lens === "squash" && opened.session.shownCommit === undefined,
        "the whole changeset",
      );
      if (shell.left.tool !== "all") throw new Error(`the rail shows ${shell.left.tool}, not the tool chosen last`);
      if (asked() - before !== 0) throw new Error("the window asked for the lens the backend set");
    });
    await h.step("the lens is remembered for the branch", async () => {
      const before = asked();
      await h.other.setLens("perCommit");
      await opened.open({ type: "workingTree" });
      await h.until(() => opened.session?.facts?.target.changeset.type === "workingTree", "the working tree");
      await opened.open({ type: "branch", name: "feature" });
      await h.until(
        () => opened.session?.lens === "perCommit" && opened.session.shownCommit !== undefined,
        "the remembered lens",
      );
      if (shell.left.tool !== "commits") throw new Error(`the rail shows ${shell.left.tool}, not Commits`);
      if (asked() - before !== 0) throw new Error("opening a remembered lens asked for the lens");
    });
    await h.step("a changeset of one commit cannot be reviewed by commit", async () => {
      h.fixture.resetHeadTo("main");
      h.fixture.commitWorkingTree();
      await opened.open({ type: "branch", name: "feature" });
      await h.until(() => opened.session?.facts?.commits === 1, "the one-commit branch");
      await h.other.setLens("squash");
      await h.until(() => opened.session?.lens === "squash", "the whole changeset");
      const before = asked();
      shell.toggle("commits");
      if (shell.left.tool !== "changes") throw new Error(`the rail shows ${shell.left.tool}, not Changes`);
      if (asked() - before !== 0)
        throw new Error(`choosing the Commits tool asked for the lens ${asked() - before} times`);
      if (opened.session?.lens !== "squash") throw new Error(`the lens became ${opened.session?.lens}`);
    });
  },

  /** A scope of a commit that is not the one on screen has that commit's files and marks, and keeps them while the window moves on. Its commits come from the store's list, not a read of its own. */
  async pinned(h) {
    await h.step("opened the branch by commit", () => reviewByCommit(h), { sent: { commits: 1 } });
    const commit = (index: number): View => ({ type: "commit", index });
    const id = (opened.session?.commits ?? [])[0].commit.id;
    const scope = newScope(
      h.other,
      { type: "commit", id },
      () => undefined,
      onView().commitList,
      () => opened.session?.currentShown()?.revision,
    );
    const hunksOf = (layout: { files: readonly { path: string; hunks: readonly unknown[] }[] } | undefined) =>
      layout?.files.find((f) => f.path === "a.txt")?.hunks.length;
    await h.step(
      "another commit on screen",
      async () => {
        const before = opened.session?.commits ?? [];
        await onView().showCommit(2);
        // Wait for the store's list to be of the revision on screen, so the scope reuses it rather than racing its read.
        await h.until(() => (opened.session?.commits ?? []) !== before, "the commits of the commit on screen");
        await scope.load();
        const own = await h.other.manifest(commit(0));
        const wanted = own.files.find((f) => f.path === "a.txt")?.hunks.length;
        if (!wanted || hunksOf(scope.layout) !== wanted || scope.commit?.commit.id !== id)
          throw new Error("the scope does not read its own commit");
        if (hunksOf(onView().currentLayout()) === wanted) throw new Error("the commits cannot be told apart");
        const text = await h.other.fileText("b.txt", commit(0));
        if (text.content.type !== "text" || text.content.text.includes("c2"))
          throw new Error("a file the commit does not touch is not read as the commit leaves it");
        const missing = await h.other.fileText("not/there.txt", commit(0));
        if (missing.content.type !== "noText" || missing.content.reason !== "absent")
          throw new Error("a file the commit lacks is not answered as absent");
      },
      { sent: { commits: 1 } },
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
      { sent: { commits: 1 } },
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
        const before = opened.session?.commits ?? [];
        const answer = await h.other.toggleHunk(revision, "a.txt", 1, commit(0));
        await h.until(() => (opened.session?.commits ?? []) !== before, "the commits after the mark");
        scope.heard(answer, revision);
        await scope.whenLoaded();
        if (scope.state("a.txt")?.review.hunks[1] !== "seen") throw new Error("the scope did not follow the mark");
        const afterMark = reads;
        scope.heard(answer, revision);
        await scope.whenLoaded();
        if (reads !== afterMark || afterMark !== 1) throw new Error(`the scope read ${reads} times for one mark`);
        h.other.manifest = manifest;
      },
      { sent: { commits: 1 } },
    );
    await h.step(
      "the whole-changeset view takes a mark of its file as the store does",
      async () => {
        const whole = onView().whole;
        const revision = whole.currentShown()?.revision ?? -1;
        const answer = await h.other.toggleHunk(revision, "b.txt", 0, CHANGESET);
        whole.heard(answer, revision);
        if (whole.state("b.txt")?.review.hunks[0] !== "seen")
          throw new Error("the whole-changeset view did not follow");
      },
      { sent: { commits: 1 } },
    );
    // A tab on the changeset while a commit is on screen comments through the whole view, not the one on screen.
    await h.step(
      "a comment from the whole-changeset view while a commit is on screen",
      async () => {
        const whole = onView().whole;
        const shown = whole.currentShown();
        if (!shown) throw new Error("the whole changeset has not read");
        const target = { type: "file", side: "target", path: "a.txt" } as const;
        const sent = await h.asElement(
          () => onView().threads.create(shown, target, "a file note", (s) => whole.admit(s)),
          shown,
        );
        if (!sent) throw new Error("the comment was refused");
      },
      once("createThread", { commits: 0 }),
    );
    // A background read that races the changeset moving on keeps the scope and says nothing; the
    // store's reload reads the newer revision and brings the scope up to date then.
    await h.step("a scope read that races a revision change is silent and catches up on the next read", async () => {
      let revision = opened.session?.currentShown()?.revision ?? -1;
      const racing = newScope(
        h.other,
        { type: "commit", id },
        () => undefined,
        onView().commitList,
        () => revision,
      );
      await racing.load();
      const wasRevision = racing.currentShown()?.revision ?? -1;
      if (wasRevision !== revision) throw new Error("the racing scope did not read its revision");
      h.fixture.push("a.txt", "p9", [2, 10, 30]);
      await h.until(() => (opened.session?.layout?.shown.revision ?? -1) > wasRevision, "the changeset to move on");
      // The scope reads at the old revision, which the backend has moved past: it keeps its view and says nothing.
      await racing.load();
      if (racing.currentShown()?.revision !== wasRevision) throw new Error("the racing read replaced the scope's view");
      // The store's reload reads at the revision now on view, and the scope catches up with it.
      revision = opened.session?.currentShown()?.revision ?? -1;
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
        void onView().showCommit(2);
      },
      once("toggleHunk"),
    );
    await h.step(
      "flag while stepping back",
      () => {
        const from = h.shown();
        void flag(h, "a.txt", 0, from);
        void onView().showCommit(0);
      },
      once("toggleFlag"),
    );
    // Once the next commit is asked for, a request built for the one on screen is refused.
    await h.step(
      "mark refused once the next commit is asked for",
      () => {
        const from = h.shown();
        void onView().showCommit(1);
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
    await h.step("range shown", () => onView().setRange(defaultRange(onView().versions.items)));
    await h.step("other marks while in the range", () => mark("b.txt", 2));
    await h.step("back to the changeset", () => onView().setRange(undefined));
    await h.step("opened the branch by commit", () => reviewByCommit(h));
    await h.step("other marks while by commit", () => mark("a.txt", 0));
  },

  /** The socket drops while another window marks; the window connects again and reads anew. */
  async reconnect(h) {
    await h.step("hunk seen", () => hunk(h, "b.txt", 0), once("toggleHunk"));
    await h.step("other marks while the socket is down", async () => {
      const back = h.dropSocket();
      const { revision } = await h.other.manifest(CHANGESET);
      await h.other.toggleHunk(revision, "b.txt", 1, CHANGESET);
      await back;
    });
    await h.step("seen after reconnecting", () => hunk(h, "b.txt", 1), once("toggleHunk"));
  },

  /**
   * The pull request list is read when the queue is shown and on a refresh, and never by opening a
   * changeset or starting a review.
   */
  async listings(h) {
    await h.step("the queue is shown", () => queue.show(), { sent: { listPullRequests: 1 } });
    await h.step(
      "a changeset is opened",
      async () => {
        await opened.open({ type: "branch", name: "feature" });
        await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
      },
      { sent: { listPullRequests: 0 } },
    );
    await h.step(
      "a review is started",
      async () => {
        await h.until(() => (opened.session?.paths ?? []).length > 0, "the branch's files");
        await hunk(h, first(), 0);
        await h.until(() => opened.session?.facts?.review.state === "open", "the review to start");
      },
      once("toggleHunk", { listPullRequests: 0 }),
    );
    await h.step("the queue is refreshed", () => queue.refresh(), { sent: { listPullRequests: 1 } });
  },

  /**
   * Choosing rows of the queue puts changesets on view without asking the language servers for anything, and
   * without reading the versions or the commits; the review screen does.
   */
  async queue(h) {
    const asked = () => h.sent.filter((r) => ANALYSES.has(r.type)).length;
    const reviewReads = () => h.sent.filter((r) => REVIEW_ONLY.has(r.type)).length;
    const versionReads = () => h.sent.filter((r) => r.type === "versions").length;
    const told = (shown: boolean) => h.sent.filter((r) => r.type === "setReviewShown" && r.shown === shown).length;
    await h.step("rows chosen on the queue", async () => {
      // The files that are asked about are those a language serves, once the servers are known:
      // the fixture's own are served by none.
      await h.until(() => serverList.servers.length > 0, "the servers");
      h.fixture.write("d.rs", "wt", [3]);
      await h.until(() => (opened.session?.paths ?? []).includes("d.rs"), "the Rust file in the working tree");
      await opened.open({ type: "branch", name: "feature" });
      await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
      await opened.open({ type: "workingTree" });
      await h.until(() => opened.session?.facts?.target.changeset.type === "workingTree", "the working tree");
      if (!problems.analysable.includes("d.rs")) throw new Error("the changeset has no file to analyse");
      await sleep(500);
      if (asked() || told(true)) throw new Error("the queue asked the language servers for something");
      if (reviewReads()) throw new Error("the queue read what only the review screen shows");
    });
    await h.step("review screen shown", async () => {
      reviewShown.set(true);
      await h.until(() => told(true) === 1, "the backend to be told");
      await h.until(() => versionReads() === 1, "the versions to be read");
      // The backend analyses the file itself and pushes the answer; the store applies it.
      const answered = () => onView().symbols.files.get("d.rs")?.state;
      await h.until(() => answered() === "done" || answered() === "failed", "the symbols");
    });
    await h.step("back on the queue", async () => {
      reviewShown.set(false);
      await h.until(() => told(false) === 1, "the backend to be told");
      const before = asked();
      const readBefore = reviewReads();
      await opened.open({ type: "branch", name: "feature" });
      await sleep(500);
      if (asked() !== before) throw new Error("the queue asked the language servers for something");
      if (reviewReads() !== readBefore) throw new Error("the queue read what only the review screen shows");
    });
  },

  /** The Pipelines choice and folds last as long as the session, through the review screen coming and going, and end with it. */
  async pipelines(h) {
    const held = onView();
    await h.step("a review is started", () => hunk(h, "b.txt", 0), once("toggleHunk"));
    await h.step("a job is chosen and a run folded", () => {
      held.pipelines.selected = "job-1";
      held.pipelines.toggle("run-1");
    });
    await h.step("the review screen goes away and comes back", async () => {
      reviewShown.set(false);
      reviewShown.set(true);
      await h.until(() => held.versions.items.length > 0, "the versions to be read again");
      if (held.pipelines.selected !== "job-1" || !held.pipelines.folded.has("run-1"))
        throw new Error("the choice and the folds did not outlast the review screen");
    });
    await h.step("another changeset is on view", async () => {
      await opened.open({ type: "branch", name: "feature" });
      await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
      if (held.versions.items.length) throw new Error("the ended session still holds the versions");
    });
    await h.step("back on the first one", async () => {
      await opened.open({ type: "workingTree" });
      await h.until(() => opened.session?.facts?.target.changeset.type === "workingTree", "the working tree");
      const again = onView();
      if (again === held) throw new Error("the session was kept");
      if (again.pipelines.selected !== undefined || again.pipelines.folded.size)
        throw new Error("the choice and the folds of the first session are still there");
    });
  },

  /**
   * Only a session of a pull request asks the platform about it and holds its checks, and nothing an
   * ended one was waiting for reaches the window. The fixture has no platform, so the pull request
   * sessions are made beside the one on view and the platform's answers are stood in for.
   */
  async pullRequest(h) {
    const infoRequests = () => h.sent.filter((r) => r.type === "pullRequestInfo").length;
    const pullRequest: ChangesetKey = { type: "pullRequest", remote: "github.com/acme/app", number: 7 };
    const failure = (job: string): CommitChecks => ({
      commit: "abc",
      state: "failure",
      runs: [{ id: job, name: job, state: "failure", phase: "finished", duration: 1, log: true }],
      pipelines: [],
    });
    const checksEvent = (checks: CommitChecks[]): SessionEvent => ({
      type: "checksChanged",
      project: 0,
      checks,
      error: null,
      limitedUntil: null,
    });
    await h.step("working tree and branch sessions ask nothing of the platform", async () => {
      if (opened.session?.pullRequest) throw new Error("the working tree session holds pull request info");
      await opened.open({ type: "branch", name: "feature" });
      await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
      if (opened.session?.pullRequest) throw new Error("the branch session holds pull request info");
      if (infoRequests()) throw new Error("a session of no pull request asked the platform about one");
    });
    await h.step(
      "a pull request session asks, and takes the checks it hears",
      async () => {
        const owner = newSession({ key: pullRequest });
        try {
          owner.start();
          await h.until(() => infoRequests() === 1, "the pull request info to be asked for");
          owner.hear(checksEvent([failure("job-a")]));
          if (owner.checks.of("abc")?.runs[0]?.id !== "job-a") throw new Error("the session took no checks");
          owner.hear(checksEvent([failure("job-b")]));
          if (owner.checks.of("abc")?.runs[0]?.id !== "job-b") throw new Error("new checks did not replace the old");
          if (opened.session?.checks.of("abc")) throw new Error("another session holds the checks");
        } finally {
          owner.dispose();
        }
      },
      { rejected: 1 },
    );
    await h.step("what an ended pull request session was waiting for changes nothing", async () => {
      const backend = session.connected();
      const info = backend.pullRequestInfo.bind(backend);
      let release!: (info: PullRequestInfo) => void;
      const held = new Promise<PullRequestInfo>((resolve) => (release = resolve));
      backend.pullRequestInfo = () => held;
      const owner = newSession({ key: pullRequest });
      try {
        owner.start();
        owner.hear(checksEvent([failure("job-a")]));
        owner.dispose();
        release({ platform: { kind: "gitHub", host: "github.com" } } as unknown as PullRequestInfo);
        await held;
        await sleep(50);
        if (owner.pullRequest?.info) throw new Error("an answer after the session ended was taken");
        if (platform.status) throw new Error("an answer after the session ended set the platform status");
        owner.hear(checksEvent([failure("job-b")]));
        if (owner.checks.of("abc")?.runs[0]?.id !== "job-a")
          throw new Error("checks were taken after the session ended");
      } finally {
        backend.pullRequestInfo = info;
        owner.dispose();
      }
    });
  },

  /**
   * What the Find tool found, Find in Files' place and chosen result, and the text in the comment boxes
   * belong to the changeset on view: the next one starts without them, and an answer of the one left
   * touches nothing.
   */
  async owned(h) {
    const usages: UsagesSpec = { type: "usages", symbol: "foo", side: "target", path: "a.txt", line: 0, col: 0 };
    const held = onView();
    await h.step("the first changeset holds results and unsent comments", () => {
      held.findSets.keepSearch({
        spec: {
          type: "search",
          request: {
            scope: { type: "view", view: CHANGESET },
            pattern: { text: "line", matchCase: false, wholeWord: false, regex: false },
            mask: "",
            side: "both",
            onlyChanges: false,
            notSeen: false,
            limit: 0,
          },
          options: { text: "line", matchCase: false, wholeWord: false, regex: false, side: "both", onlyChanges: false },
          scope: { label: "All changes" },
        },
        results: { type: "searchResults", revision: 1, files: [], hits: 0, truncated: false, searched: 0 },
      });
      held.findInFiles.scope = "commit:1";
      held.findInFiles.selected = "a.txt:3";
      held.drafts.set("reply:1", "a half-written reply");
      held.changesetComment.composing = true;
      held.changesetComment.body = "a word on the whole";
      // A failed pull request sync is announced as the backend would; the footer keeps it.
      const backend = session.connected();
      const conn = (backend as unknown as { conn: { onMessage(m: ServerMessage): void } }).conn;
      const event: SessionEvent = {
        type: "task",
        project: backend.project ?? -1,
        id: 9_000,
        kind: "sync",
        title: "Pull request sync",
        done: null,
        total: null,
        detail: null,
        state: { type: "failed", message: "offline" },
      };
      conn.onMessage({ type: "event", event });
      if (progress.failed.length !== 1) throw new Error("the footer holds no failure");
      if (held.unsentComments() !== 2) throw new Error(`${held.unsentComments()} boxes hold text, not 2`);
      if (!held.findSets.sets.length) throw new Error("the find set was not placed");
    });
    await h.step("a run still going when the changeset is switched away from", async () => {
      const backend = session.connected();
      const run = backend.usages.bind(backend);
      let release!: () => void;
      const gate = new Promise<void>((resolve) => (release = resolve));
      let asked = false;
      backend.usages = async () => {
        asked = true;
        await gate;
        return { unavailable: { message: "late" } } as unknown as Awaited<ReturnType<typeof run>>;
      };
      try {
        held.findSets.findUsages(usages);
        const set = held.findSets.current;
        await h.until(() => asked, "the usages to be asked for");
        const switching = opened.open({ type: "branch", name: "feature" });
        await h.until(() => discardPrompt.count === 2, "the question about the two comments");
        discardPrompt.answer(true);
        await switching;
        await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
        release();
        await sleep(100);
        if (!set || set.message !== undefined || set.phase !== "running")
          throw new Error("the answer of the ended session was taken");
      } finally {
        backend.usages = run;
      }
    });
    await h.step("the next changeset starts with none of it", () => {
      const next = onView();
      if (next === held) throw new Error("the session was kept");
      if (next.findSets.sets.length || next.findSets.current) throw new Error("the find sets came along");
      if (next.findInFiles.scope !== undefined || next.findInFiles.selected !== undefined)
        throw new Error("Find in Files' place or chosen result came along");
      if (next.drafts.count() || next.changesetComment.body || next.changesetComment.composing)
        throw new Error("a comment box came along");
      if (next.unsentComments() !== 0) throw new Error("the next changeset has unsent comments");
      if (progress.failed.length) throw new Error("a failure of the changeset left stays in the footer");
    });
  },

  /**
   * The changed symbols, the symbol on show and the outline belong to the changeset on view: the next one
   * starts without them, and an answer or a push of the one left touches nothing.
   */
  async analysis(h) {
    const held = onView();
    const symbol = {
      id: { path: "d.rs", name: ["foo"] },
      modules: 0,
      kind: "function",
      span: { start: { line: 0, col: 0 }, end: { line: 0, col: 3 } },
      at: { line: 0, col: 0 },
      test: false,
    } as unknown as SymbolInfo;
    // The answers about the symbol and the outline are held back until the changeset is left.
    const backend = session.connected();
    const withheld = ["signature", "symbolCalls", "symbolUsages", "reachability", "fileOutline"] as const;
    const stubs = backend as unknown as Record<(typeof withheld)[number], () => Promise<unknown>>;
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    let asked = 0;
    let revision = 0;
    for (const name of withheld)
      stubs[name] = async () => {
        if (name === "signature" || name === "fileOutline") asked++;
        await gate;
        return { symbols: [], unavailable: { message: "late" } };
      };
    try {
      await h.step("the first changeset shows a symbol and an outline", async () => {
        h.fixture.write("d.rs", "wt", [3]);
        await h.until(() => (opened.session?.paths ?? []).includes("d.rs"), "the Rust file in the working tree");
        const answered = () => held.symbols.files.get("d.rs")?.state;
        await h.until(() => answered() === "done" || answered() === "failed", "the symbols");
        revision = h.shown().revision;
        held.symbols.choose(symbol);
        held.structure.show({ tab: "changes", path: "d.rs", shown: h.shown(), panel: false, caret: () => undefined });
        await h.until(() => asked === 2, "the signature and the outline to be asked for");
      });
      await h.step("another changeset is switched to while they are out", async () => {
        await opened.open({ type: "branch", name: "feature" });
        await h.until(() => opened.session?.facts?.target.changeset.type === "branch", "the branch");
        release();
        await sleep(100);
        if (held.symbols.signature !== undefined) throw new Error("the answer of the ended session was taken");
        if (held.structure.load.state !== "loading") throw new Error("the outline of the ended session was taken");
      });
      await h.step("the next changeset starts with none of it", () => {
        const next = onView();
        if (next === held) throw new Error("the session was kept");
        if (next.symbols.selected !== undefined) throw new Error("the selected symbol came along");
        if (next.structure.target !== undefined) throw new Error("the outline's target came along");
        if (next.symbols.files.has("d.rs")) throw new Error("the symbols of the working tree came along");
        const push = {
          type: "fileSymbols",
          project: 0,
          symbols: { revision, path: "late.rs", symbols: [], context: [], gap: null },
          done: 1,
          total: 1,
        } as unknown as SessionEvent;
        next.hear(push);
        if (next.symbols.files.has("late.rs")) throw new Error("a push for another revision was taken");
      });
    } finally {
      for (const name of withheld) Reflect.deleteProperty(backend, name);
    }
  },

  /** Closing the project, opening another and switching to another changeset wait for the answer about comment text not added. */
  async leaving(h) {
    const answer = async (discard: boolean) => {
      await h.until(() => discardPrompt.count !== undefined, "the question");
      const asked = discardPrompt.count;
      discardPrompt.answer(discard);
      return asked;
    };
    const holding = (text: string) => {
      const draft = onView().changesetComment;
      draft.composing = true;
      draft.body = text;
    };
    await h.step("text is typed in a comment box", () => holding("a word"));
    await h.step("Keep editing: closing the project does nothing", async () => {
      const closing = session.closeRepo();
      if ((await answer(false)) !== 1) throw new Error("the question was not about one comment");
      await closing;
      if (session.repo === undefined) throw new Error("the project was closed");
      if (onView().changesetComment.body !== "a word") throw new Error("the text is gone");
    });
    await h.step("Keep editing: opening another project does nothing", async () => {
      const opening = session.openRepo(h.fixture.anotherRepo());
      await answer(false);
      await opening;
      if (opened.session?.changesetComment.body !== "a word") throw new Error("the project was left");
    });
    await h.step("Keep editing: switching to another changeset does nothing", async () => {
      const switching = opened.open({ type: "branch", name: "feature" });
      await answer(false);
      await switching;
      if (opened.session?.key.type !== "workingTree") throw new Error("the changeset was switched");
    });
    // No step follows: a step waits for an open project.
    const closing = session.closeRepo();
    await answer(true);
    await closing;
    if (session.repo !== undefined) throw new Error("Discard did not close the project");
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
      { sent: { toggleHunk: 20 } },
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
      { sent: { toggleHunk: 4, toggleFlag: 4 } },
    );
    await h.step("changesets asked for one after another without waiting", async () => {
      const feature: ChangesetKey = { type: "branch", name: "feature" };
      const tree: ChangesetKey = { type: "workingTree" };
      for (const key of [tree, feature, tree, feature, tree]) void opened.open(key);
      await opened.open(feature);
      await h.until(() => opened.session !== undefined && !opened.loading, "the last changeset");
      // What the invariants then compare with the backend is the changeset it has on view, whichever of the
      // requests it served last: nothing of the changesets passed through is left in it.
      const key = opened.session?.key;
      if (!key || keyId(key) !== keyId(opened.session?.facts?.target.changeset ?? tree))
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
        await opened.open({ type: "workingTree" });
        await h.until(() => opened.session !== undefined && !opened.loading, "the next changeset");
        release();
        await marking;
        backend.toggleHunk = toggle;
      },
      once("toggleHunk"),
    );
    // The viewer is told the view is gone; the harness checks the notice.
    await h.step(
      "a comment on a changeset that is switched away from is not sent",
      async () => {
        const owner = onView();
        const shown = owner.currentShown();
        if (!shown) throw new Error("no changeset is on view");
        const backend = session.connected();
        const create = backend.createThread.bind(backend);
        let sent = 0;
        backend.createThread = (...args) => {
          sent++;
          return create(...args);
        };
        const switching = opened.open({ type: "branch", name: "feature" });
        if (opened.session) throw new Error("the session outlived the request for another changeset");
        const created = await owner.threads.create(shown, { type: "changeset" }, "late");
        await switching;
        backend.createThread = create;
        if (created || sent) throw new Error("a comment was sent while no changeset was on view");
        await h.until(() => opened.session !== undefined && !opened.loading, "the changeset");
      },
      { notice: STALE_NOTICE },
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
