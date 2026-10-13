import assert from "node:assert/strict";
import { test } from "node:test";
import type { LocationRow } from "../../generated/LocationRow.ts";
import { countText, groupRows, matchedRow, opensBelow, symbolLabel, visibleRows } from "./pickerRows.ts";

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

void test("the changeset's label is left out when it is the only section, and no other is", () => {
  const only = groupRows([row("a.rs", 1, { change: "hunk" }), row("b.rs", 2, { change: "file" })], () => undefined);
  assert.deepEqual(shape(only), ["a.rs ×1", "1", "b.rs ×1", "2"]);
  const lone = groupRows([row("b.rs", 2)], () => undefined);
  assert.deepEqual(shape(lone), ["OTHER FILES", "b.rs ×1", "2"]);
  const both = groupRows([row("a.rs", 1, { change: "hunk" }), row("b.rs", 2)], () => undefined);
  assert.deepEqual(
    shape(both).filter((s) => s === "CHANGESET" || s === "OTHER FILES"),
    ["CHANGESET", "OTHER FILES"],
  );
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
    row("src/a.rs", 2, {
      text: "print()",
      within: { id: { path: "src/a.rs", name: ["Retry", "preview"] }, modules: 0, kind: "function" },
    }),
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
  const label = (path: string, name: string[], modules = 0) =>
    symbolLabel({ path, within: { id: { path, name }, modules, kind: "function" } });
  assert.equal(label("src/sync/retry.rs", ["RetryPolicy", "preview"]), "RetryPolicy::preview");
  assert.equal(label("src/retry.ts", ["Retry", "preview"]), "Retry.preview");
  assert.equal(
    label(
      "src/Services/TaskDispatcher.cs",
      [
        "HES.LCD500.Application.Services.TaskEngine",
        "TaskDispatcher",
        "Trigger<TTask, TState>(TTask task, Job job) : void",
      ],
      1,
    ),
    "TaskDispatcher.Trigger<TTask, TState>",
  );
  assert.equal(label("src/A.cs", ["HES.LCD500.TaskEngine", "TaskDispatcher"], 1), "TaskDispatcher");
  // A Rust impl block is named by its type.
  assert.equal(label("src/http/client.rs", ["impl Client", "send"]), "Client::send");
  assert.equal(label("src/sync/retry.rs", ["impl Backoff for RetryPolicy", "delay_for"]), "RetryPolicy::delay_for");
  assert.equal(label("src/sync/retry.rs", ["impl<T: Copy> Wrapper<T>", "get"]), "Wrapper<T>::get");
  assert.equal(label("src/sync/retry.rs", ["impl Backoff for RetryPolicy"]), "RetryPolicy");
  assert.equal(label("src/lib.rs", ["free_fn"]), "free_fn");
  assert.equal(symbolLabel({ path: "src/billing/util.py", within: null }), "util.py");
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

void test("the popup opens on the side with room for its full height, else on the roomier side, whatever it holds", () => {
  const at = (top: number) => ({ top, bottom: top + 18 });
  // 900 high: room below a caret at 100 (782 >= 520); above a caret at 800 (788 above, 70 below).
  assert.equal(opensBelow(at(100), 900, 520), true);
  assert.equal(opensBelow(at(800), 900, 520), false);
  // Neither side fits 520: the larger one wins, and a tie goes below.
  assert.equal(opensBelow(at(300), 600, 520), false);
  assert.equal(opensBelow(at(200), 600, 520), true);
  assert.equal(opensBelow(at(291), 600, 520), true);
  // Room below at full height wins even where above is roomier.
  assert.equal(opensBelow(at(1000), 1600, 520), true);
});
