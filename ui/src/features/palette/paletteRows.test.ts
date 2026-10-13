import assert from "node:assert/strict";
import { test } from "node:test";
import {
  type ActionCandidate,
  type Candidates,
  type ChangesetCandidate,
  type Entry,
  type FileCandidate,
  type ThreadCandidate,
  RECENT_KEPT,
  build,
  entriesOf,
  rank,
  recentId,
  remember,
  splitLine,
} from "./paletteRows.ts";

const file = (path: string, over: Partial<FileCandidate> = {}): FileCandidate => ({
  kind: "file",
  key: path,
  path,
  changed: true,
  status: "M",
  added: 1,
  removed: 0,
  seen: 0,
  oldBase: 0,
  ...over,
});
const action = (label: string, over: Partial<ActionCandidate> = {}): ActionCandidate => ({
  kind: "action",
  key: label,
  label,
  off: false,
  ...over,
});
const thread = (id: number, text: string, over: Partial<ThreadCandidate> = {}): ThreadCandidate => ({
  kind: "thread",
  key: String(id),
  text,
  where: "a.rs:1",
  resolved: false,
  moved: false,
  ...over,
});
const changeset = (key: string, title: string, over: Partial<ChangesetCandidate> = {}): ChangesetCandidate => ({
  kind: "changeset",
  key,
  title,
  source: "local",
  icon: "branch",
  onView: false,
  finished: false,
  ...over,
});
const none: Candidates = { file: [], symbol: [], changeset: [], thread: [], action: [] };

const keys = (rows: { item: { key: string } }[]) => rows.map((r) => r.item.key);
const rowKeys = (entries: Entry[]) => entries.flatMap((e) => (e.type === "row" ? [e.row.item.key] : []));

void test("letters on word starts find a file and highlight them", () => {
  const rows = rank([file("src/sync/retry_policy.rs"), file("src/sync/job.rs")], "rtp", []);
  assert.deepEqual(keys(rows), ["src/sync/retry_policy.rs"]);
  assert.deepEqual(rows[0].ranges, [
    [9, 10],
    [11, 12],
    [15, 16],
  ]);
});

void test("a match in the file name outranks the same match only in the folders", () => {
  const rows = rank([file("docs/retry/notes.md"), file("src/retry.rs"), file("tests/retry/mod.rs")], "retry", []);
  assert.equal(rows[0].item.key, "src/retry.rs");
});

void test("a changed file leads an unchanged one however well the unchanged one matches", () => {
  const rows = rank(
    [file("benches/retry.rs", { changed: false }), file("src/sync/rate_limiter_retry.rs", { seen: 100 })],
    "retry.rs",
    [],
  );
  assert.deepEqual(keys(rows), ["src/sync/rate_limiter_retry.rs", "benches/retry.rs"]);
});

void test("a changed file that is not fully seen leads one that is", () => {
  const rows = rank([file("a/retry.rs", { seen: 100 }), file("b/retry.rs", { seen: 40 })], "retry", []);
  assert.deepEqual(keys(rows), ["b/retry.rs", "a/retry.rs"]);
});

void test("a recent result wins between close matches but never over a better match", () => {
  const items = [file("a/retry.rs"), file("b/retry.rs"), file("c/rety_extra_long_name.rs")];
  const recent = [recentId("file", "b/retry.rs")];
  assert.equal(rank(items, "retry", recent)[0].item.key, "b/retry.rs");
  assert.equal(rank(items, "retry", [])[0].item.key, "a/retry.rs");
  const far = [file("retry/mod.rs"), file("y/retry.rs")];
  assert.equal(rank(far, "retry", [recentId("file", "retry/mod.rs")])[0].item.key, "y/retry.rs");
});

void test("a line typed after a file's name narrows to the name and is kept for opening", () => {
  assert.deepEqual(splitLine("retry.rs:40"), { query: "retry.rs", line: 40 });
  assert.deepEqual(splitLine("retry.rs:40:7"), { query: "retry.rs", line: 40 });
  assert.deepEqual(splitLine("retry.rs"), { query: "retry.rs" });
  const rows = rank([file("src/retry.rs"), file("src/job.rs")], "retry.rs:40", []);
  assert.deepEqual(keys(rows), ["src/retry.rs"]);
  assert.equal(rows[0].line, 40);
});

void test("an action that cannot run is listed after those that can, with its reason", () => {
  const rows = rank(
    [action("Post Comments", { off: true, disabled: "No comment waits" }), action("Re-confirm All", { off: false })],
    "o",
    [],
  );
  assert.deepEqual(keys(rows), ["Re-confirm All", "Post Comments"]);
  const all = rank([action("Zoom", { off: true }), action("Alpha")], "", []);
  assert.deepEqual(keys(all), ["Alpha", "Zoom"]);
});

void test("open threads come before resolved ones, in a tab and in a query", () => {
  const items = [thread(1, "jitter cap", { resolved: true }), thread(2, "jitter cap again")];
  assert.deepEqual(keys(rank(items, "jitter", [])), ["2", "1"]);
  const tab = build("thread", "", { ...none, thread: items }, []);
  assert.deepEqual(
    tab.groups.map((g) => [g.label, rowKeys(g.entries)]),
    [
      ["Open", ["2"]],
      ["Resolved", ["1"]],
    ],
  );
});

void test("the changeset on view is listed but never first", () => {
  const rows = rank([changeset("a", "retry policy", { onView: true }), changeset("b", "retry sync")], "retry", []);
  assert.deepEqual(keys(rows), ["b", "a"]);
});

void test("empty files list the least seen changed files first", () => {
  const rows = rank([file("a.rs", { seen: 100 }), file("b.rs", { seen: 0 }), file("c.rs", { seen: 50 })], "", []);
  assert.deepEqual(keys(rows), ["b.rs", "c.rs", "a.rs"]);
});

void test("All caps each kind, ends it in the way to its tab and orders the kinds by their best result", () => {
  const candidates: Candidates = {
    ...none,
    file: ["src/retry_a.rs", "src/retry_b.rs", "src/retry_c.rs", "src/retry_d.rs"].map((p) => file(p)),
    action: [action("Retry sync")],
  };
  const { groups, counts } = build("all", "retry", candidates, []);
  assert.deepEqual(
    groups.map((g) => g.label),
    ["Files", "Actions", ""],
  );
  const files = groups[0].entries;
  assert.equal(files.length, 4);
  assert.deepEqual(files[3], { type: "more", kind: "file", count: 4 });
  assert.deepEqual(groups[2].entries, [{ type: "find", query: "retry" }]);
  assert.deepEqual(counts && [counts.all, counts.file, counts.action], [5, 4, 1]);
});

void test("in All a kind whose best result is better than the others' is listed first", () => {
  const candidates: Candidates = {
    ...none,
    file: [file("retry/mod.rs", { changed: false })],
    action: [action("retry")],
  };
  const labels = (c: Candidates) => build("all", "retry", c, []).groups.map((g) => g.label);
  assert.deepEqual(labels(candidates), ["Actions", "Files", ""]);
  assert.deepEqual(labels({ ...candidates, file: [file("retry/mod.rs")] }), ["Files", "Actions", ""]);
});

void test("All with nothing found has no way on to Find in Files, and no counts without a query", () => {
  assert.deepEqual(build("all", "zzz", { ...none, file: [file("a.rs")] }, []).groups, []);
  assert.equal(build("all", "", none, []).counts, undefined);
});

void test("empty All lists the recent results, then the changed files least seen", () => {
  const candidates: Candidates = {
    ...none,
    file: [file("a.rs", { seen: 100 }), file("b.rs", { seen: 10 }), file("c.rs", { seen: 60 })],
    action: [action("Settings")],
  };
  const { groups } = build("all", "", candidates, [recentId("action", "Settings"), recentId("file", "gone.rs")]);
  assert.deepEqual(
    groups.map((g) => [g.label, rowKeys(g.entries)]),
    [
      ["Recent", ["Settings"]],
      ["Changed files, least reviewed first", ["b.rs", "c.rs"]],
    ],
  );
});

void test("a file's own tab does not list a recent file twice", () => {
  const candidates: Candidates = { ...none, file: [file("a.rs"), file("b.rs")] };
  const { groups } = build("file", "", candidates, [recentId("file", "b.rs")]);
  assert.deepEqual(rowKeys(entriesOf(groups)), ["b.rs", "a.rs"]);
});

void test("a query in a kind's tab ends in the way on to Find in Files, except for actions", () => {
  const candidates: Candidates = { ...none, file: [file("retry.rs")], action: [action("Retry")] };
  assert.equal(entriesOf(build("file", "retry", candidates, []).groups).at(-1)?.type, "find");
  assert.equal(entriesOf(build("action", "retry", candidates, []).groups).at(-1)?.type, "row");
});

void test("the files tab lists the changed files and the others apart, with how many there are", () => {
  const candidates: Candidates = {
    ...none,
    file: [
      file("src/retry.rs"),
      file("benches/retry.rs", { changed: false }),
      file("docs/retry.md", { changed: false }),
    ],
  };
  const { groups } = build("file", "retry", candidates, []);
  assert.deepEqual(
    groups.map((g) => [g.label, g.count, rowKeys(g.entries)]),
    [
      ["Changed files", 1, ["src/retry.rs"]],
      ["Other files", 2, ["benches/retry.rs", "docs/retry.md"]],
      ["", undefined, []],
    ],
  );
});

void test("a tab draws the best two hundred of a long list and says how many it left out", () => {
  const many = Array.from({ length: 450 }, (_, i) => file(`vendor/pkg${i}/mod.rs`, { changed: false }));
  const group = build("file", "mod", { ...none, file: many }, []).groups[0];
  assert.equal(group.count, 450);
  assert.equal(group.entries.length, 200);
  assert.equal(group.cut, 250);
});

void test("while the project's files are listed the others' place says so, before the way on", () => {
  const candidates: Candidates = { ...none, file: [file("src/retry.rs")] };
  for (const tab of ["all", "file"] as const) {
    const labels = build(tab, "retry", candidates, [], true).groups.map((g) => [g.label, g.loading !== undefined]);
    assert.deepEqual(labels.slice(-2), [
      ["Other files", true],
      ["", false],
    ]);
  }
  assert.equal(
    build("file", "retry", candidates, [], false).groups.some((g) => g.loading),
    false,
  );
});

void test("remembering moves a result to the front and keeps the last thirty", () => {
  assert.deepEqual(remember(["a", "b", "c"], "b"), ["b", "a", "c"]);
  const many = Array.from({ length: RECENT_KEPT }, (_, i) => `f${i}`);
  const next = remember(many, "new");
  assert.equal(next.length, RECENT_KEPT);
  assert.equal(next[0], "new");
  assert.equal(next.includes(`f${RECENT_KEPT - 1}`), false);
});
