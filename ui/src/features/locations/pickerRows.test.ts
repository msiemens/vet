import assert from "node:assert/strict";
import { test } from "node:test";
import type { LocationRow } from "../../generated/LocationRow.ts";
import { codeRuns, countText, groupRows, libraryLabel, matchedRow, symbolLabel, visibleRows } from "./pickerRows.ts";

const row = (path: string, line: number, init: Partial<LocationRow> = {}): LocationRow => ({
  path,
  line,
  col: 1,
  within: null,
  text: "",
  highlights: [],
  matchStart: 0,
  matchEnd: 0,
  change: "unchanged",
  external: false,
  ...init,
});

const shape = (items: ReturnType<typeof groupRows>) =>
  items.map((i) => (i.kind === "section" ? i.label : i.kind === "file" ? `${i.path} ×${i.count}` : `${i.row.line}`));

void test("places are grouped under the changeset, the other files and the libraries, each file in line order", () => {
  const items = groupRows(
    [
      row("/dep/lib.rs", 3, { external: true }),
      row("src/b.rs", 9),
      row("src/a.rs", 20, { change: "file" }),
      row("src/a.rs", 4, { change: "hunk" }),
      row("src/c.rs", 1, { change: "file" }),
    ],
    () => undefined,
  );
  assert.deepEqual(shape(items), [
    "CHANGESET",
    "src/a.rs ×2",
    "4",
    "20",
    "src/c.rs ×1",
    "1",
    "OTHER FILES",
    "src/b.rs ×1",
    "9",
    "LIBRARIES",
    "/dep/lib.rs ×1",
    "3",
  ]);
});

void test("a changed file with a place in a hunk comes before one without, whatever their names", () => {
  const items = groupRows([row("a.rs", 1, { change: "file" }), row("z.rs", 1, { change: "hunk" })], () => undefined);
  assert.deepEqual(
    items.filter((i) => i.kind === "file").map((i) => i.path),
    ["z.rs", "a.rs"],
  );
});

void test("the places are numbered in the order shown, and only the changeset's files carry a status", () => {
  const items = groupRows(
    [row("a.rs", 7), row("b.rs", 2, { change: "hunk" }), row("b.rs", 1, { change: "hunk" })],
    (path) => (path === "b.rs" ? "M" : "A"),
  );
  assert.deepEqual(
    items.flatMap((i) => (i.kind === "hit" ? [[i.at, i.row.path, i.row.line]] : [])),
    [
      [0, "b.rs", 1],
      [1, "b.rs", 2],
      [2, "a.rs", 7],
    ],
  );
  assert.deepEqual(
    items.flatMap((i) => (i.kind === "file" ? [[i.path, i.status]] : [])),
    [
      ["b.rs", "M"],
      ["a.rs", undefined],
    ],
  );
});

void test("typing keeps the places whose symbol, line or file matches, and the toggle only those of the changeset", () => {
  const rows = [
    row("src/a.rs", 1, { text: "let d = next_delay();", change: "hunk" }),
    row("src/a.rs", 2, { text: "print()", within: { path: "src/a.rs", name: ["Retry", "preview"] } }),
    row("lib/b.rs", 3, { text: "x", external: true }),
  ];
  assert.equal(visibleRows(rows, "", false).length, 3);
  assert.deepEqual(
    visibleRows(rows, "next_delay", false).map((r) => r.line),
    [1],
  );
  assert.deepEqual(
    visibleRows(rows, "preview", false).map((r) => r.line),
    [2],
  );
  assert.deepEqual(
    visibleRows(rows, "", true).map((r) => r.line),
    [1],
  );
});

void test("a place is named by its symbol and the last part of its container, in the file's language's separator", () => {
  const label = (path: string, name: string[]) => symbolLabel({ path, within: { path, name } });
  assert.equal(label("src/sync/retry.rs", ["RetryPolicy", "preview"]), "RetryPolicy::preview");
  assert.equal(label("src/retry.ts", ["Retry", "preview"]), "Retry.preview");
  assert.equal(
    label("src/Services/TaskDispatcher.cs", [
      "HES.LCD500.Application.Services.TaskEngine",
      "TaskDispatcher",
      "Trigger<TTask, TState>(TTask task, Job job) : void",
    ]),
    "TaskDispatcher.Trigger<TTask, TState>",
  );
  assert.equal(label("src/A.cs", ["HES.LCD500.TaskEngine", "TaskDispatcher"]), "TaskEngine.TaskDispatcher");
  // A Rust impl block is named by its type.
  assert.equal(label("src/http/client.rs", ["impl Client", "send"]), "Client::send");
  assert.equal(label("src/sync/retry.rs", ["impl Backoff for RetryPolicy", "delay_for"]), "RetryPolicy::delay_for");
  assert.equal(label("src/sync/retry.rs", ["impl<T: Copy> Wrapper<T>", "get"]), "Wrapper<T>::get");
  assert.equal(label("src/sync/retry.rs", ["impl Backoff for RetryPolicy"]), "RetryPolicy");
  assert.equal(label("src/lib.rs", ["free_fn"]), "free_fn");
  assert.equal(symbolLabel({ path: "src/billing/util.py", within: null }), "util.py");
});

void test("a line is cut where its colours change and where the match is, in UTF-16 units", () => {
  // `let` is a keyword (0..3), `foo` a function (10..13) after "é😀", which is three units.
  const runs = codeRuns({
    text: "let é😀 = foo();",
    highlights: [0, 3, 0, 7, 3, 5],
    matchStart: 10,
    matchEnd: 13,
  });
  assert.deepEqual(
    runs.map((r) => [r.text, r.className, r.hit]),
    [
      ["let", "syn-keyword", false],
      [" é😀 = ", undefined, false],
      ["foo", "syn-function", true],
      ["();", undefined, false],
    ],
  );
});

void test("a match inside a coloured stretch splits it and keeps its colour", () => {
  const runs = codeRuns({ text: "next_delay(3)", highlights: [0, 10, 5], matchStart: 5, matchEnd: 8 });
  assert.deepEqual(
    runs.map((r) => [r.text, r.className, r.hit]),
    [
      ["next_", "syn-function", false],
      ["del", "syn-function", true],
      ["ay", "syn-function", false],
      ["(3)", undefined, false],
    ],
  );
  assert.deepEqual(codeRuns({ text: "", highlights: [], matchStart: 0, matchEnd: 0 }), []);
});

void test("a flat list puts places in changed hunks first, then changed files, other files and libraries, each by path and line", () => {
  const rows = visibleRows(
    [
      row("/dep/x.rs", 1, { external: true }),
      row("src/a.rs", 9),
      row("src/b.rs", 3, { change: "file" }),
      row("src/z.rs", 5, { change: "hunk" }),
      row("src/c.rs", 8, { change: "hunk" }),
      row("src/c.rs", 2, { change: "hunk" }),
    ],
    "",
    false,
  );
  assert.deepEqual(
    rows.map((r) => `${r.path}:${r.line}`),
    ["src/c.rs:2", "src/c.rs:8", "src/z.rs:5", "src/b.rs:3", "src/a.rs:9", "/dep/x.rs:1"],
  );
});

void test("an implementation marks the searched name in its line, else the place found", () => {
  const impl = row("a.rs", 1, { text: "impl Backoff for ConstantBackoff {", matchStart: 16, matchEnd: 31 });
  const marked = matchedRow(impl, "Backoff");
  assert.equal(impl.text.slice(marked.matchStart, marked.matchEnd), "Backoff");
  assert.equal(marked.matchStart, 5);
  // A longer name that contains it is not the name.
  const other = row("a.rs", 1, { text: "impl Foo for ConstantBackoff {", matchStart: 13, matchEnd: 28 });
  assert.deepEqual([matchedRow(other, "Backoff").matchStart, matchedRow(other, "Backoff").matchEnd], [13, 28]);
  // Names with regular-expression characters are taken literally.
  const dollar = row("a.ts", 1, { text: "class A extends $store {", matchStart: 0, matchEnd: 5 });
  assert.equal(matchedRow(dollar, "$store").matchStart, 16);
});

void test("a cut answer says how many there were and how many are listed", () => {
  assert.deepEqual(countText(47, 47), { count: "47" });
  assert.deepEqual(countText(1340, 200), { count: "1,340", cut: "first 200" });
});

void test("a library's file is named by its package where the path tells", () => {
  assert.deepEqual(
    libraryLabel("/home/u/.cargo/registry/src/index.crates.io-6f17d22bba15001f/backoff-0.4.0/src/exponential.rs"),
    {
      package: "backoff 0.4.0",
      rest: "src/exponential.rs",
    },
  );
  assert.deepEqual(libraryLabel("/p/node_modules/@scope/pkg/dist/index.d.ts"), {
    package: "@scope/pkg",
    rest: "dist/index.d.ts",
  });
  assert.deepEqual(libraryLabel("/usr/lib/rustlib/src/core/option.rs"), {
    rest: "/usr/lib/rustlib/src/core/option.rs",
  });
});
