import assert from "node:assert/strict";
import { test } from "node:test";
import type { Diagnostic } from "../generated/Diagnostic.ts";
import type { Hunk } from "../generated/Hunk.ts";
import {
  type Filters,
  type Problem,
  NO_FILTERS,
  countsOf,
  emptyOf,
  filtered,
  problemsOf,
  reviewedAt,
  rowsOf,
  stepInFile,
} from "./problemRows.ts";

const diagnostic = (line: number, over: Partial<Diagnostic> = {}): Diagnostic => ({
  span: { start: { line, col: 0 }, end: { line, col: 4 } },
  severity: "warning",
  message: `problem at ${line}`,
  source: "rustc",
  code: "dead_code",
  origin: "new",
  ...over,
});

const hunk = (targetStart: number, targetLen: number): Hunk => ({ baseStart: 0, baseLen: 0, targetStart, targetLen });

const at = (path: string, line: number, over: Partial<Diagnostic> = {}): Problem =>
  problemsOf(path, [diagnostic(line, over)], undefined)[0];

void test("a problem is in a hunk up to the context around it, and on a changed line only inside it", () => {
  const file = { hunks: [hunk(10, 2)], states: ["unseen" as const] };
  assert.deepEqual(reviewedAt(file, 10), { reviewed: "unseen", onChanged: true });
  assert.deepEqual(reviewedAt(file, 7), { reviewed: "unseen", onChanged: false });
  assert.deepEqual(reviewedAt(file, 14), { reviewed: "unseen", onChanged: false });
  assert.deepEqual(reviewedAt(file, 6), { reviewed: "none", onChanged: false });
  assert.deepEqual(reviewedAt(file, 15), { reviewed: "none", onChanged: false });
  assert.deepEqual(reviewedAt(undefined, 10), { reviewed: "none", onChanged: false });
});

void test("a hunk seen against an old base reads as old, and the state of the right hunk counts", () => {
  const file = { hunks: [hunk(2, 1), hunk(30, 1)], states: ["seen" as const, "seenAgainstOldBase" as const] };
  assert.equal(reviewedAt(file, 2).reviewed, "seen");
  assert.equal(reviewedAt(file, 31).reviewed, "old");
});

void test("twins and problems differing in severity, source or code have ids of their own", () => {
  const found = problemsOf(
    "a.rs",
    [
      diagnostic(1),
      diagnostic(1),
      diagnostic(1, { severity: "error" }),
      diagnostic(1, { source: "clippy" }),
      diagnostic(1, { code: "unused" }),
    ],
    undefined,
  );
  assert.equal(new Set(found.map((p) => p.id)).size, found.length);
  const again = problemsOf("a.rs", [diagnostic(1), diagnostic(1)], undefined);
  assert.deepEqual(
    again.map((p) => p.id),
    found.slice(0, 2).map((p) => p.id),
    "stable for the same diagnostics",
  );
});

void test("a file's problems come in the order of the file", () => {
  const found = problemsOf(
    "a.rs",
    [diagnostic(9), diagnostic(2), diagnostic(2, { span: { start: { line: 2, col: 9 }, end: { line: 2, col: 12 } } })],
    undefined,
  );
  assert.deepEqual(
    found.map((p) => [p.line, p.diagnostic.span.start.col]),
    [
      [2, 0],
      [2, 9],
      [9, 0],
    ],
  );
});

void test("only new problems pass the New filter, and only once origins are known", () => {
  const list = [at("a.rs", 1), at("a.rs", 2, { origin: "existing" }), at("a.rs", 3, { origin: "pending" })];
  const only: Filters = { ...NO_FILTERS, origin: "new" };
  assert.deepEqual(
    filtered(list, only, true).map((p) => p.line),
    [1],
  );
  assert.equal(filtered(list, only, false).length, 3);
});

void test("the severity chips, the hunk filters and the text each narrow the list", () => {
  const file = { hunks: [hunk(10, 1)], states: ["unseen" as const] };
  const list = [
    ...problemsOf("a.rs", [diagnostic(10, { severity: "error", message: "boom" })], file),
    ...problemsOf("a.rs", [diagnostic(12, { severity: "hint", message: "inactive" })], file),
    ...problemsOf("b.rs", [diagnostic(40, { message: "far away" })], file),
  ];
  const lines = (f: Partial<Filters>) =>
    filtered(list, { ...NO_FILTERS, ...f }, true).map((p) => `${p.path}:${p.line}`);
  assert.deepEqual(lines({ levels: { error: false, warning: true, info: true } }), ["a.rs:12", "b.rs:40"]);
  assert.deepEqual(lines({ levels: { error: true, warning: false, info: false } }), ["a.rs:10"]);
  assert.deepEqual(lines({ unseenOnly: true }), ["a.rs:10", "a.rs:12"]);
  assert.deepEqual(lines({ changedOnly: true }), ["a.rs:10"]);
  assert.deepEqual(lines({ query: "  FAR " }), ["b.rs:40"]);
  assert.deepEqual(lines({ query: "b.rs" }), ["b.rs:40"]);
});

void test("with origins known the list has the new problems above the old, each by file and line", () => {
  const list = [
    at("b.rs", 5, { origin: "existing" }),
    at("b.rs", 9),
    at("a.rs", 3),
    at("b.rs", 2),
    at("a.rs", 1, { origin: "existing" }),
  ];
  const rows = rowsOf(list, { grouping: "origin", settled: true, folded: new Set() });
  assert.deepEqual(
    rows.map((r) =>
      r.kind === "problem" ? `  ${r.problem.path}:${r.problem.line}` : r.kind === "file" ? ` ${r.path}` : r.title,
    ),
    [
      "New in this change",
      " a.rs",
      "  a.rs:3",
      " b.rs",
      "  b.rs:2",
      "  b.rs:9",
      "Already there",
      " a.rs",
      "  a.rs:1",
      " b.rs",
      "  b.rs:5",
    ],
  );
  assert.ok(rows.every((r) => r.kind !== "problem" || !r.pill));
});

void test("a folded group or file hides its rows but keeps its count", () => {
  const list = [at("a.rs", 3), at("a.rs", 4), at("b.rs", 1)];
  const rows = rowsOf(list, { grouping: "origin", settled: true, folded: new Set(["o:new:a.rs"]) });
  const a = rows.find((r) => r.kind === "file" && r.path === "a.rs");
  assert.ok(a?.kind === "file" && !a.open && a.count === 2);
  assert.equal(rows.filter((r) => r.kind === "problem").length, 1);
  const none = rowsOf(list, { grouping: "origin", settled: true, folded: new Set(["o:new"]) });
  assert.deepEqual(
    none.map((r) => r.kind),
    ["group"],
  );
});

void test("until origins are known the list is by file and every row carries its origin", () => {
  const list = [at("a.rs", 3, { origin: "pending" })];
  const rows = rowsOf(list, { grouping: "origin", settled: false, folded: new Set() });
  assert.deepEqual(
    rows.map((r) => r.kind),
    ["file", "problem"],
  );
  assert.ok(rows.every((r) => r.kind !== "problem" || r.pill));
});

void test("grouped by severity the rows name their file and put the new ones first", () => {
  const list = [
    at("a.rs", 1, { origin: "existing" }),
    at("b.rs", 4),
    at("c.rs", 2, { severity: "error" }),
    at("a.rs", 7, { severity: "hint" }),
  ];
  const rows = rowsOf(list, { grouping: "severity", settled: true, folded: new Set() });
  assert.deepEqual(
    rows.map((r) =>
      r.kind === "group" ? r.title : r.kind === "problem" ? `${r.problem.path}:${r.problem.line}` : "?",
    ),
    ["Errors", "c.rs:2", "Warnings", "b.rs:4", "a.rs:1", "Information and hints", "a.rs:7"],
  );
  assert.ok(rows.every((r) => r.kind !== "problem" || r.showFile));
});

void test("stepping goes from the caret to the next problem of the file and wraps around within it", () => {
  const list = problemsOf("a.rs", [diagnostic(2), diagnostic(5), diagnostic(9)], undefined);
  const line = (caret: number | undefined, delta: 1 | -1, selected?: string) =>
    stepInFile(list, caret, selected, delta)?.line;
  assert.equal(line(0, 1), 2);
  assert.equal(line(2, 1), 5, "a problem on the caret's line is not the next one");
  assert.equal(line(3, 1), 5);
  assert.equal(line(9, 1), 2, "past the last wraps to the first");
  assert.equal(line(5, -1), 2);
  assert.equal(line(3, -1), 2);
  assert.equal(line(2, -1), 9, "before the first wraps to the last");
  assert.equal(line(undefined, 1), 2);
  assert.equal(line(undefined, -1), 9);
  assert.equal(stepInFile([], 3, undefined, 1), undefined);
});

void test("problems on one line are each reached while the caret stays on it", () => {
  const list = problemsOf(
    "a.rs",
    [diagnostic(4), diagnostic(4, { message: "second" }), diagnostic(4, { message: "third" }), diagnostic(8)],
    undefined,
  );
  const first = stepInFile(list, 3, undefined, 1);
  assert.equal(first?.line, 4);
  const second = stepInFile(list, 4, first.id, 1);
  assert.equal(second?.diagnostic.message, list[1].diagnostic.message);
  assert.equal(stepInFile(list, 4, second.id, -1)?.id, first.id);
  assert.equal(stepInFile(list, 4, list[2].id, 1)?.line, 8);
});

void test("files whose origin is unknown get a group of their own after the others", () => {
  const list = [at("a.rs", 3), at("b.rs", 5, { origin: "unknown" }), at("a.rs", 1, { origin: "existing" })];
  const rows = rowsOf(list, { grouping: "origin", settled: true, folded: new Set() });
  assert.deepEqual(
    rows.flatMap((r) => (r.kind === "group" ? [[r.title, r.count]] : [])),
    [
      ["New in this change", 1],
      ["Already there", 1],
      ["Origin unknown", 1],
    ],
  );
});

void test("a problem still being compared is listed even where the other origins are settled", () => {
  const list = [at("a.rs", 3, { origin: "pending" }), at("a.rs", 5, { origin: "existing" })];
  const rows = rowsOf(list, { grouping: "origin", settled: true, folded: new Set() });
  assert.equal(rows.filter((r) => r.kind === "problem").length, 2);
});

void test("the empty list says why: hidden by the filters, only by New, none yet, or none", () => {
  const existing = [at("a.rs", 1, { origin: "existing" }), at("a.rs", 2, { origin: "existing" })];
  const none = (filters: Filters, all: Problem[], unanswered = 0, settled = true) =>
    emptyOf({ all, shown: filtered(all, filters, settled), filters, settled, unanswered });
  const onlyNew: Filters = { ...NO_FILTERS, origin: "new" };

  assert.equal(none(NO_FILTERS, [at("a.rs", 1)]), undefined, "a list with problems has no empty state");
  assert.deepEqual(none(onlyNew, existing), { kind: "onlyExisting", existing: 2 });
  assert.deepEqual(none({ ...onlyNew, changedOnly: true }, existing), { kind: "hidden", hidden: 2 });
  assert.deepEqual(none({ ...NO_FILTERS, query: "zzz" }, existing), { kind: "hidden", hidden: 2 });
  assert.deepEqual(none(onlyNew, [...existing, at("b.rs", 1, { origin: "unknown" })]), { kind: "hidden", hidden: 3 });
  assert.deepEqual(none(NO_FILTERS, []), { kind: "none" });
  assert.deepEqual(
    none({ ...NO_FILTERS, query: "zzz" }, []),
    { kind: "none" },
    "filters hide nothing of an empty scope",
  );
  assert.deepEqual(none(onlyNew, []), { kind: "none" });
  assert.deepEqual(none(NO_FILTERS, [], 2), { kind: "partial" });
});

void test("the headline counts errors and warnings that are new or may be, not those of unknown origin", () => {
  const list = [
    at("a.rs", 1, { severity: "error" }),
    at("a.rs", 2),
    at("a.rs", 3, { origin: "existing", severity: "error" }),
    at("a.rs", 4, { severity: "info" }),
    at("a.rs", 5, { origin: "pending" }),
    at("b.rs", 1, { origin: "unknown", severity: "error" }),
  ];
  assert.deepEqual(countsOf(list), {
    headline: 3,
    worst: "error",
    all: 6,
    newOnes: 3,
    existing: 1,
    unknown: 1,
    info: 1,
  });
  assert.equal(countsOf([at("a.rs", 4, { severity: "info" })]).worst, undefined);
  assert.equal(countsOf([at("a.rs", 2)]).worst, "warning");
  assert.equal(countsOf([at("b.rs", 1, { origin: "unknown", severity: "error" })]).worst, undefined);
});
