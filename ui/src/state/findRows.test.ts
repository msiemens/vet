import assert from "node:assert/strict";
import { test } from "node:test";
import { NO_FILTERS, itemsOf, narrowing, visible } from "./findRows.ts";
import type { Place } from "./foundPlace.ts";

const place = (path: string, line: number, init: Partial<Place> & { changedFile?: true } = {}): Place => {
  const { changedFile, ...rest } = init;
  return {
    id: `target:${path}:${line}:0`,
    file: {
      path,
      status: changedFile && "modified",
      hunks: [],
      states: [],
      external: false,
      label: path,
    },
    line,
    col: 0,
    width: 1,
    side: "target",
    kind: "context",
    runs: [],
    text: "let delay = 1;",
    hits: [],
    review: "none",
    changed: false,
    ...rest,
  };
};

void test("files with a place on a changed line come first, then the others by path, libraries last; lines in order", () => {
  const places = [
    place("src/z.rs", 3),
    { ...place("/dep/lib.rs", 1), file: { ...place("/dep/lib.rs", 1).file, external: true } },
    place("src/b.rs", 9, { changed: true }),
    place("src/b.rs", 2),
    place("src/a.rs", 5),
  ];
  const items = itemsOf(places, new Set());
  assert.deepEqual(
    items.map((i) => (i.kind === "file" ? i.file.path : `${i.place.line}`)),
    ["src/b.rs", "2", "9", "src/a.rs", "5", "src/z.rs", "3", "/dep/lib.rs", "1"],
  );
});

void test("a folded file lists its header alone, with its count and how many places are not seen", () => {
  const places = [
    place("a.rs", 1, { review: "unseen" }),
    place("a.rs", 2, { review: "old" }),
    place("a.rs", 3, { review: "seen" }),
  ];
  const items = itemsOf(places, new Set(["f:a.rs"]));
  assert.equal(items.length, 1);
  assert.deepEqual(items[0].kind === "file" && [items[0].count, items[0].notSeen, items[0].open], [3, 2, false]);
});

void test("the filters combine: a typed query reads symbol, line and path; the toggles narrow to the changeset and to hunks not seen", () => {
  const places = [
    place("src/a.rs", 1, { changedFile: true, review: "unseen", within: "Retry::next_delay" }),
    place("src/a.rs", 2, { changedFile: true, review: "seen", text: "jitter()" }),
    place("docs/b.md", 3, { review: "none", text: "jitter" }),
  ];
  const lines = (f: Partial<typeof NO_FILTERS>) => visible(places, { ...NO_FILTERS, ...f }).map((p) => p.line);
  assert.deepEqual(lines({}), [1, 2, 3]);
  assert.deepEqual(lines({ query: "JITTER" }), [2, 3]);
  assert.deepEqual(lines({ query: "retry::next" }), [1]);
  assert.deepEqual(lines({ query: "docs/" }), [3]);
  assert.deepEqual(lines({ changeset: true }), [1, 2]);
  assert.deepEqual(lines({ notSeen: true }), [1]);
  assert.deepEqual(lines({ changeset: true, query: "jitter" }), [2]);
  assert.deepEqual(lines({ notSeen: true, query: "jitter" }), []);
});

void test("seen against an old base is not seen", () => {
  const old = [place("a.rs", 1, { review: "old" })];
  assert.equal(visible(old, { ...NO_FILTERS, notSeen: true }).length, 1);
});

void test("a filter is narrowing only when it hides something", () => {
  assert.equal(narrowing(NO_FILTERS), false);
  assert.equal(narrowing({ ...NO_FILTERS, query: "  " }), false);
  assert.equal(narrowing({ ...NO_FILTERS, notSeen: true }), true);
});
