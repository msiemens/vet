import assert from "node:assert/strict";
import { test } from "node:test";
import type { Base } from "../../generated/Base.ts";
import type { CommitInfo } from "../../generated/CommitInfo.ts";
import type { ExcludedGroup } from "../../generated/ExcludedGroup.ts";
import type { PullRequestInfo } from "../../generated/PullRequestInfo.ts";
import type { ReviewFilter } from "../../generated/ReviewFilter.ts";
import type { VersionInfo } from "../../generated/VersionInfo.ts";
import { type EmptyInput, type Panel, emptyView } from "./emptyChanges.ts";

const NOW = new Date(2026, 9, 12, 12, 0).getTime();
const commit = (shortId: string, subject = "a commit"): CommitInfo => ({
  id: `${shortId}ffff`,
  shortId,
  subject,
  author: "Anna",
  time: NOW / 1000 - 3600,
});
const base = (containedIn?: Base["containedIn"]): Base => ({
  commit: commit("b4c7a21"),
  containedIn,
  headOffBase: false,
});
const NO_FILTER: ReviewFilter = { roots: [], include: [], exclude: [], hideBinary: false, reviewGenerated: false };

function input(over: Partial<EmptyInput> = {}): EmptyInput {
  return {
    key: { type: "branch", name: "feature" },
    target: { changeset: { type: "branch", name: "feature" }, name: "feature", branch: "feature", draft: false },
    view: "changeset",
    base: base(),
    changesetBase: base(),
    review: { state: "browsing" },
    excluded: [],
    filter: NO_FILTER,
    versions: [],
    commits: [],
    threads: { total: 0, open: 0 },
    now: NOW,
    ...over,
  };
}

const pullRequest = (state: PullRequestInfo["state"]): PullRequestInfo =>
  ({
    number: 7,
    state,
    sourceBranch: "bugs/lock",
    authorName: "Anna",
    platform: { kind: "github" },
  }) as PullRequestInfo;

const labels = (panels: Panel[]) => panels.flatMap((p) => p.rows.map((r) => r.label));
const group = (by: ExcludedGroup["by"], paths: string[], folders: ExcludedGroup["folders"] = []): ExcludedGroup => ({
  by,
  count: paths.length,
  delta: { added: paths.length, removed: 0 },
  files: paths.map((path) => ({ path, delta: { added: 1, removed: 0 } })),
  folders,
});

void test("a branch the default branch holds names the merge that brought it in, and only when git could say", () => {
  const merged = input({
    changesetBase: base({ branch: "main", sameTip: false, mergedBy: commit("a1b2c3d", "Merge pull request #7") }),
  });
  const view = emptyView(merged);
  assert.equal(view.title, "Already in main");
  assert.deepEqual(labels(view.panels), ["Merged by", "When"]);

  const fastForward = emptyView(input({ changesetBase: base({ branch: "main", sameTip: false }) }));
  assert.equal(fastForward.title, "Already in main");
  assert.deepEqual(labels(fastForward.panels), []);
});

void test("a merge that carried commits says how many and how far the branch has gone since", () => {
  const view = emptyView(
    input({
      target: { ...input().target, head: "7c8d9e0aaaa" },
      changesetBase: base({
        branch: "main",
        sameTip: false,
        mergedBy: commit("a1b2c3d", "Merge"),
        commits: 4,
        since: 11,
      }),
    }),
  );
  const rows = view.panels[0].rows;
  assert.deepEqual(
    rows.map((r) => [r.label, r.meta, r.value.at(-1)]),
    [
      ["Merged by", undefined, " Merge"],
      ["When", undefined, "Anna · 11:00"],
      ["Branch tip", "4 commits, all in main", { mono: "7c8d9e0" }],
      ["main since", undefined, "11 commits ahead of the merge"],
    ],
  );
});

void test("a branch at the default branch's tip has no commits yet, not a merge", () => {
  const view = emptyView(input({ changesetBase: base({ branch: "main", sameTip: true }) }));
  assert.equal(view.title, "No commits yet");
  assert.deepEqual(labels(view.panels), ["Branch", "Target"]);
});

void test("a pull request at the target's tip has no commits to review yet", () => {
  const view = emptyView(
    input({
      key: { type: "pullRequest", remote: "github.com/acme/app", number: 7 },
      changesetBase: base({ branch: "main", sameTip: true }),
      pullRequest: pullRequest("open"),
    }),
  );
  assert.equal(view.title, "No commits to review yet");
});

void test("a changeset with nothing different and no known reason names its base with the hash as code", () => {
  const view = emptyView(input());
  assert.equal(view.title, "No changes");
  assert.deepEqual(view.panels[0].rows[0], { label: "Base", value: [{ mono: "b4c7a21" }], meta: "a commit" });
});

void test("a merged pull request keeps the viewer's review only while there are threads, and says what is open", () => {
  const held = base({ branch: "main", sameTip: false, mergedBy: commit("a1b2c3d", "Merge") });
  const merged = (threads: EmptyInput["threads"]) =>
    emptyView(
      input({
        key: { type: "pullRequest", remote: "github.com/acme/app", number: 7 },
        changesetBase: held,
        pullRequest: pullRequest("merged"),
        threads,
      }),
    );
  assert.equal(merged({ total: 0, open: 0 }).title, "Merged");
  assert.deepEqual(labels(merged({ total: 0, open: 0 }).panels), ["Source", "Merge commit"]);
  assert.equal(merged({ total: 0, open: 0 }).panels[0].rows[0].meta, undefined, "no head is known to name");
  const review = merged({ total: 3, open: 1 }).panels[0].rows.find((r) => r.label === "Your review");
  assert.ok(review);
  assert.deepEqual(review.value, ["3 threads, 1 still open"]);
  assert.deepEqual(review.action, { id: "comments", label: "Comments" });
  assert.deepEqual(merged({ total: 2, open: 0 }).panels[0].rows.at(-1)?.value, ["2 threads, all resolved"]);
});

void test("a workspace that was edited and put back lists its versions, newest first, each but the first to show", () => {
  const version = (n: number, files: VersionInfo["files"]): VersionInfo => ({
    n,
    at: NOW / 1000 - 600 * (4 - n),
    origin: "settled",
    reviewedHere: false,
    files,
  });
  const edit = (added: number, removed: number): VersionInfo["files"] => [
    { path: "src/Module.cs", delta: { added, removed } },
  ];
  const versions = [version(1, []), version(2, edit(6, 0)), version(3, edit(0, 6))];
  const key = { type: "workingTree" } as const;
  const view = emptyView(input({ key, review: { state: "open", version: 3, updatedAt: 0 }, versions }));
  assert.equal(view.title, "No changes left");
  const rows = view.panels[0].rows;
  assert.deepEqual(
    rows.map((r) => [r.label, r.action?.n, r.meta]),
    [
      ["v3", 3, "Module.cs · undid v2"],
      ["v2", 2, "Module.cs"],
      ["v1", undefined, "opened"],
    ],
  );

  const fresh = emptyView(
    input({ key, review: { state: "open", version: 1, updatedAt: 0 }, versions: versions.slice(0, 1) }),
  );
  assert.equal(fresh.title, "No changes yet");
});

void test("a watched workspace names its folder only when the folder is known", () => {
  const key = { type: "workingTree" } as const;
  assert.deepEqual(labels(emptyView(input({ key, repo: "/code/app" })).panels), ["Watching", "HEAD", "Compared with"]);
  assert.deepEqual(labels(emptyView(input({ key })).panels), ["HEAD", "Compared with"]);
});

void test("when the filter hides every file each rule's files are listed and its link brings back just those", () => {
  const filter: ReviewFilter = { ...NO_FILTER, roots: ["src"], exclude: ["**/*.snap", "gen/**"] };
  const view = emptyView(
    input({
      filter,
      excluded: [
        group([{ kind: "outsideRoots" }], ["docs/a.md", "README.md"]),
        group([{ kind: "pattern", pattern: "**/*.snap" }], ["tests/x.snap"]),
        group([{ kind: "generated" }], ["Cargo.lock"]),
      ],
    }),
  );
  assert.equal(view.title, "Everything is filtered out");
  assert.match(view.lead, /All 4 changed files/);
  const lifted = view.panels.map((p) => [p.heading, p.link?.filter]);
  assert.deepEqual(lifted, [
    ["Outside review roots", { ...filter, roots: [] }],
    ["Match **/*.snap", { ...filter, exclude: ["gen/**"] }],
    ["Generated", { ...filter, reviewGenerated: true }],
  ]);
});

void test("a file that two rules leave out is in one panel, and its link brings back both", () => {
  const filter: ReviewFilter = { ...NO_FILTER, roots: ["src"], reviewGenerated: false };
  const view = emptyView(
    input({ filter, excluded: [group([{ kind: "outsideRoots" }, { kind: "generated" }], ["Cargo.lock"])] }),
  );
  assert.equal(view.panels[0].heading, "Outside review roots, Generated");
  assert.deepEqual(view.panels[0].link?.filter, { ...filter, roots: [], reviewGenerated: true });
});

void test("a long group lists its first files and counts the rest", () => {
  const paths = Array.from({ length: 8 }, (_, i) => `lock/${i}.lock`);
  const view = emptyView(input({ excluded: [group([{ kind: "generated" }], paths)] }));
  const rows = view.panels[0].rows;
  assert.equal(rows.length, 6);
  assert.deepEqual(rows.at(-1)?.value, ["+ 3 more"]);
  assert.equal(rows.at(-1)?.open, undefined);
});

void test("a narrowed review offers the folders its narrowing left out and no way to lift project rules", () => {
  const narrowing: ReviewFilter = { ...NO_FILTER, roots: ["src/legacy"] };
  const folder = (path: string, dir: boolean, files: number): ExcludedGroup["folders"][number] => ({
    path,
    dir,
    files,
    delta: { added: files, removed: 0 },
  });
  const view = emptyView(
    input({
      narrowing,
      excluded: [
        group(
          [{ kind: "narrowing" }],
          ["src/api/a.rs", "src/api/b.rs", "src/sync/c.rs", "top.rs"],
          [folder("src/api", true, 2), folder("src/sync", true, 1), folder("top.rs", false, 1)],
        ),
        group([{ kind: "generated" }], ["Cargo.lock"]),
      ],
    }),
  );
  assert.equal(view.title, "Nothing in src/legacy");
  assert.match(view.lead, /The 4 changed files are elsewhere/);
  const folders = view.panels[0].rows;
  assert.deepEqual(
    folders.map((r) => [r.value, r.meta, r.action?.filter?.roots, r.action?.filter?.include]),
    [
      [[{ mono: "src/api" }], "2 files · +2 −0", ["src/api"], []],
      [[{ mono: "src/sync" }], "1 file · +1 −0", ["src/sync"], []],
      [[{ mono: "top.rs" }], "1 file · +1 −0", [], ["/top.rs"]],
    ],
  );
  assert.equal(view.panels[1].link, undefined, "the project's rules cannot be lifted from inside a narrowing");
  assert.deepEqual(
    view.actions.map((a) => a.id),
    ["reset", "filter"],
  );
});

void test("a narrowing whose left-out files are all the project's still offers to reset it", () => {
  const view = emptyView(
    input({
      narrowing: { ...NO_FILTER, roots: ["src/legacy"] },
      excluded: [group([{ kind: "generated" }], ["Cargo.lock"])],
    }),
  );
  assert.deepEqual(
    view.actions.map((a) => a.id),
    ["reset", "filter"],
  );
});

void test("a push that changed nothing says whether the base moved, and shows the versions compared", () => {
  const version = (n: number, base: string): VersionInfo => ({
    n,
    at: NOW / 1000 - 60 * (5 - n),
    origin: "settled",
    tip: commit(`tip${n}`),
    base: commit(base),
    reviewedHere: false,
    files: [],
  });
  const versions = [version(1, "m1"), version(2, "m1"), version(3, "m1"), version(4, "m2")];
  const view = emptyView(
    input({ view: "range", versions, interdiff: { from: 3, to: 4, rebased: true, markable: true, limits: [] } }),
  );
  assert.equal(view.title, "Push v4 changed nothing");
  assert.deepEqual(
    view.panels[0].rows.map((r) => [r.label, r.meta]),
    [
      ["v3", "based on m1"],
      ["v4", "based on m2"],
      ["Why", undefined],
    ],
  );
  assert.match(view.lead, /base moved/);
  const several = emptyView(
    input({ view: "range", versions, interdiff: { from: 1, to: 3, rebased: false, markable: true, limits: [] } }),
  );
  assert.equal(several.title, "Pushes v2–v3 changed nothing");
  assert.doesNotMatch(several.lead, /base moved/);
});

void test("an empty commit leads to its parent only when the parent is one of the commits, and on to the next", () => {
  const unit = (id: string) => ({ commit: commit(id) }) as EmptyInput["commits"][number];
  const parent = commit("p1");
  const common = { view: "commit", base: { ...base(), commit: parent }, commitIndex: 1 } as const;
  const inside = emptyView(input({ ...common, commits: [unit("p1"), unit("c2"), unit("c3")] }));
  assert.equal(inside.panels[0].rows[0].action?.n, 0);
  assert.deepEqual(
    inside.actions.map((a) => a.id),
    ["next", "back"],
  );
  const last = emptyView(input({ ...common, commits: [unit("zz"), unit("c2")] }));
  assert.equal(last.panels[0].rows[0].action, undefined);
  assert.deepEqual(
    last.actions.map((a) => a.id),
    ["back"],
  );
});
