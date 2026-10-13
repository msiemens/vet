import assert from "node:assert/strict";
import { test } from "node:test";
import type { SearchFile } from "../../generated/SearchFile.ts";
import type { SearchLine } from "../../generated/SearchLine.ts";
import { decodeHighlights } from "../../lib/highlights.ts";
import {
  previewRows,
  resolveScope,
  resultId,
  resultRows,
  scopeChoices,
  segments,
  selectedRow,
  stepLine,
  trimmed,
  viewScopeId,
} from "./searchModel.ts";

const line = (n: number, text = "x", hits = [{ from: 0, to: 1 }], textStart = 0): SearchLine => ({
  kind: "added",
  line: n,
  text,
  textStart,
  hits,
});

const file = (path: string, ...lines: SearchLine[]): SearchFile => ({
  path,
  hunks: [],
  states: [],
  lines,
  hits: lines.length,
});

const SOURCES = {
  files: 4,
  range: { from: 2, to: 3 },
  commits: [
    { shortId: "aaa", subject: "first" },
    { shortId: "bbb", subject: "second" },
  ],
};

void test("every place has one choice that searches it, and the view on show is the one marked", () => {
  for (const view of [
    { type: "changeset" },
    { type: "commit", index: 1 },
    { type: "interdiff", from: 2, to: 3 },
  ] as const) {
    const choices = scopeChoices({ ...SOURCES, view });
    assert.equal(new Set(choices.map((c) => c.id)).size, choices.length);
    // The id of the view on show is the id of the choice that searches that view.
    const onShow = choices.find((c) => c.id === viewScopeId(view));
    assert.deepEqual(onShow?.scope, { type: "view", view });
    assert.deepEqual(
      choices.filter((c) => c.detail === "on show").map((c) => c.id),
      view.type === "changeset" ? [] : [viewScopeId(view)],
    );
  }
  const repository = scopeChoices({ ...SOURCES, view: { type: "changeset" } }).at(-1);
  assert.deepEqual(repository?.scope, { type: "repository" });
});

void test("the versions are offered only when the review has some, and the commits in their order", () => {
  const without = scopeChoices({ ...SOURCES, range: undefined, view: { type: "changeset" } });
  assert.deepEqual(
    without.map((c) => c.scope.type === "view" && c.scope.view.type),
    ["changeset", "commit", "commit", false],
  );
  const commits = without.filter((c) => c.group === "commit").map((c) => c.scope);
  assert.deepEqual(commits, [
    { type: "view", view: { type: "commit", index: 0 } },
    { type: "view", view: { type: "commit", index: 1 } },
  ]);
});

void test("a scope whose commit or versions left the changeset falls back to All changes", () => {
  const choices = scopeChoices({ view: { type: "changeset" }, files: 3, commits: [] });
  assert.equal(resolveScope(choices, "commit:4").id, "changes");
  assert.equal(resolveScope(choices, "repository").id, "repository");
});

void test("a collapsed file shows its header alone, and stepping skips headers", () => {
  const files = [file("a.rs", line(1), line(2)), file("b.rs", line(3))];
  const rows = resultRows(files, new Set(["a.rs"]));
  assert.deepEqual(
    rows.map((r) => r.kind),
    ["file", "file", "line"],
  );
  const all = resultRows(files, new Set());
  assert.equal(stepLine(all, -1, 1), 1);
  assert.equal(stepLine(all, 2, 1), 4);
  assert.equal(stepLine(all, 4, 1), undefined);
  assert.equal(stepLine(all, 1, -1), undefined);
  assert.equal(stepLine(all, -1, -1), 4);
});

void test("the selected result is found again after the results change, else the first line is", () => {
  const rows = resultRows([file("a.rs", line(1), line(2))], new Set());
  assert.equal(selectedRow(rows, resultId("a.rs", 1)), 2);
  assert.equal(selectedRow(rows, resultId("gone.rs", 0)), 1);
  assert.equal(selectedRow([], undefined), -1);
});

void test("a row leaves out the line's indent, and the matches' columns follow", () => {
  const l = line(0, "    let delay = 1;", [{ from: 8, to: 13 }]);
  assert.deepEqual(trimmed(l), { text: "let delay = 1;", hits: [{ from: 4, to: 9 }] });
  // A cut line starts at a column of its own.
  const cut = line(0, "elay = 1;", [{ from: 101, to: 105 }], 100);
  assert.deepEqual(trimmed(cut).hits, [{ from: 1, to: 5 }]);
});

const base = "a\nb\nc\nd\ne\nf\n";
const target = "a\nB\nB2\nc\nd\ne\nf\n";
// Base line 1 ("b") became target lines 1 and 2.
const hunks = [{ baseStart: 1, baseLen: 1, targetStart: 1, targetLen: 2 }];

const shape = (rows: ReturnType<typeof previewRows>) => rows.map((r) => `${r.kind[0]}:${r.text}${r.at ? "*" : ""}`);

void test("a preview is the unified diff around the result, a hunk's removed lines before its added ones", () => {
  const rows = previewRows({ base, target, hunks, kind: "added", line: 2, span: 2, withBase: true });
  assert.deepEqual(shape(rows), ["r:b", "a:B", "a:B2*", "c:c", "c:d"]);
  const removed = previewRows({ base, target, hunks, kind: "removed", line: 1, span: 1, withBase: true });
  assert.deepEqual(shape(removed), ["c:a", "r:b*", "a:B"]);
  assert.deepEqual(
    removed.map((r) => [r.oldNo, r.newNo]),
    [
      [1, 1],
      [2, undefined],
      [undefined, 2],
    ],
  );
});

void test("a preview of the target alone has no removed lines and keeps its line numbers", () => {
  const rows = previewRows({ base, target, hunks, kind: "context", line: 3, span: 1, withBase: false });
  assert.deepEqual(shape(rows), ["a:B2", "c:c*", "c:d"]);
  assert.deepEqual(
    rows.map((r) => r.newNo),
    [3, 4, 5],
  );
});

void test("a preview's line starts count a line break as the editor does, one unit", () => {
  for (const eol of ["\r\n", "\r", "\n"]) {
    const text = ["aa", "bbb", "c"].join(eol);
    const rows = previewRows({ base: "", target: text, hunks: [], kind: "context", line: 2, span: 5, withBase: false });
    assert.deepEqual(
      rows.map((r) => [r.text, r.start]),
      [
        ["aa", 0],
        ["bbb", 3],
        ["c", 7],
      ],
    );
  }
});

void test("a result that is not in the text has no preview", () => {
  assert.deepEqual(previewRows({ base, target, hunks, kind: "added", line: 40, span: 2, withBase: true }), []);
});

void test("matches cut a coloured stretch, and the current one is told apart", () => {
  // "let" is a keyword: [gap 0, length 3, kind 0].
  const highlights = decodeHighlights([0, 3, 0]);
  const hits = [
    { from: 2, to: 9 },
    { from: 9, to: 10 },
  ];
  assert.deepEqual(segments("let delay;", 0, highlights, hits, 1), [
    { text: "le", className: "syn-keyword" },
    { text: "t", className: "syn-keyword", find: "hit" },
    { text: " delay", className: undefined, find: "hit" },
    { text: ";", className: undefined, find: "current" },
  ]);
});
