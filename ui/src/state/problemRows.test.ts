import assert from "node:assert/strict";
import { test } from "node:test";
import type { Diagnostic } from "../generated/Diagnostic.ts";
import type { Hunk } from "../generated/Hunk.ts";
import {
  type Filters,
  type Problem,
  NO_FILTERS,
  countsOf,
  filtered,
  inOrder,
  problemsOf,
  reviewedAt,
  rowsOf,
  step,
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
  const rows = rowsOf(list, { grouping: "origin", known: true, folded: new Set() });
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
  const rows = rowsOf(list, { grouping: "origin", known: true, folded: new Set(["o:new:a.rs"]) });
  const a = rows.find((r) => r.kind === "file" && r.path === "a.rs");
  assert.ok(a?.kind === "file" && !a.open && a.count === 2);
  assert.equal(rows.filter((r) => r.kind === "problem").length, 1);
  const none = rowsOf(list, { grouping: "origin", known: true, folded: new Set(["o:new"]) });
  assert.deepEqual(
    none.map((r) => r.kind),
    ["group"],
  );
});

void test("until origins are known the list is by file and every row carries its origin", () => {
  const list = [at("a.rs", 3, { origin: "pending" })];
  const rows = rowsOf(list, { grouping: "origin", known: false, folded: new Set() });
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
  const rows = rowsOf(list, { grouping: "severity", known: true, folded: new Set() });
  assert.deepEqual(
    rows.map((r) =>
      r.kind === "group" ? r.title : r.kind === "problem" ? `${r.problem.path}:${r.problem.line}` : "?",
    ),
    ["Errors", "c.rs:2", "Warnings", "b.rs:4", "a.rs:1", "Information and hints", "a.rs:7"],
  );
  assert.ok(rows.every((r) => r.kind !== "problem" || r.showFile));
});

void test("stepping goes new problems first across files and wraps around", () => {
  const list = inOrder([
    at("b.rs", 1, { origin: "existing" }),
    at("b.rs", 8),
    at("a.rs", 5),
    at("a.rs", 2, { origin: "existing" }),
  ]);
  const names = list.map((p) => `${p.path}:${p.line}`);
  assert.deepEqual(names, ["a.rs:5", "b.rs:8", "a.rs:2", "b.rs:1"]);
  assert.equal(step(list, undefined, 1)?.line, 5);
  assert.equal(step(list, undefined, -1)?.line, 1);
  assert.equal(step(list, list[3].id, 1)?.id, list[0].id);
  assert.equal(step(list, list[0].id, -1)?.id, list[3].id);
  assert.equal(step([], undefined, 1), undefined);
});

void test("the headline counts the new errors and warnings once origins are known, else all of them", () => {
  const list = [
    at("a.rs", 1, { severity: "error" }),
    at("a.rs", 2),
    at("a.rs", 3, { origin: "existing", severity: "error" }),
    at("a.rs", 4, { severity: "info" }),
  ];
  assert.deepEqual(countsOf(list, true), { headline: 2, worst: "error", all: 4, newOnes: 3, existing: 1 });
  assert.equal(countsOf(list, false).headline, 3);
  assert.equal(countsOf([at("a.rs", 4, { severity: "info" })], true).worst, undefined);
  assert.equal(countsOf([at("a.rs", 2)], true).worst, "warning");
});
