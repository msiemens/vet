import assert from "node:assert/strict";
import { test } from "node:test";
import type { LocationRow } from "../../generated/LocationRow.ts";
import { countText, groupRows, libraryLabel, visibleRows } from "./pickerRows.ts";

const row = (path: string, line: number, init: Partial<LocationRow> = {}): LocationRow => ({
  path,
  line,
  col: 1,
  within: null,
  text: "",
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
