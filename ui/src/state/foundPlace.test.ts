import assert from "node:assert/strict";
import { test } from "node:test";
import type { LocationRow } from "../generated/LocationRow.ts";
import type { SearchFile } from "../generated/SearchFile.ts";
import { placesFromRows, placesFromSearch, reviewOf } from "./foundPlace.ts";

const hunk = { baseStart: 4, baseLen: 1, targetStart: 4, targetLen: 2 };

void test("a place is in the state of its hunk, and outside any hunk it has none", () => {
  assert.deepEqual(
    (["seen", "informational", "seenAgainstOldBase", "unseen", "unapplied", undefined] as const).map(reviewOf),
    ["seen", "seen", "old", "unseen", "unseen", "none"],
  );
});

void test("a search puts a removed line on the base side and marks the matches in the trimmed line", () => {
  const file: SearchFile = {
    path: "src/a.rs",
    status: "modified",
    hunks: [hunk],
    states: ["seenAgainstOldBase"],
    hits: 2,
    lines: [
      { kind: "removed", line: 4, hunk: 0, text: "    old_delay()", textStart: 0, hits: [{ from: 4, to: 13 }] },
      { kind: "context", line: 9, text: "    old_delay()", textStart: 0, hits: [{ from: 4, to: 13 }] },
    ],
  };
  const [removed, context] = placesFromSearch([file]);
  assert.deepEqual([removed.side, removed.kind, removed.review, removed.changed], ["base", "removed", "old", true]);
  assert.deepEqual([context.side, context.kind, context.review, context.changed], ["target", "context", "none", false]);
  assert.deepEqual(
    removed.runs.map((r) => [r.text, r.hit]),
    [
      ["old_delay", true],
      ["()", false],
    ],
  );
  assert.equal(removed.file.status, "modified");
  assert.notEqual(removed.id, context.id);
});

const row = (init: Partial<LocationRow>): LocationRow => ({
  path: "src/a.rs",
  line: 5,
  col: 9,
  within: null,
  text: "x.next_delay(1)",
  highlights: [],
  matchStart: 2,
  matchEnd: 12,
  change: "unchanged",
  external: false,
  ...init,
});

void test("a usage in a hunk takes the state of that hunk from what the changeset says of its file", () => {
  const changed = (path: string) =>
    path === "src/a.rs" ? { status: "modified" as const, hunks: [hunk], states: ["seen" as const] } : undefined;
  const [inHunk, outside, elsewhere] = placesFromRows(
    [
      row({ change: "hunk", hunk: 0 }),
      row({ line: 40, change: "file" }),
      row({ path: "src/b.rs", change: "unchanged" }),
    ],
    "target",
    changed,
  );
  assert.deepEqual([inHunk.review, inHunk.changed, inHunk.kind], ["seen", true, "added"]);
  assert.deepEqual([outside.review, outside.changed, outside.kind], ["none", false, "context"]);
  assert.equal(elsewhere.file.status, undefined);
  // Positions are 0-based; the match is where the row says, in the line's own columns.
  assert.deepEqual([inHunk.line, inHunk.col, inHunk.width], [4, 8, 10]);
  assert.deepEqual(inHunk.hits, [{ from: 8, to: 18 }]);
});

void test("a usage names its symbol, or is at module level when no symbol holds it", () => {
  const symbol = { id: { path: "src/a.rs", name: ["Retry", "next"] }, modules: 0, kind: "function" as const };
  const [inside, top] = placesFromRows([row({ within: symbol }), row({ line: 2 })], "target", () => undefined);
  assert.equal(inside.within, "Retry::next");
  assert.equal(top.within, "module level");
});

void test("a usage found on the base side is a removed line of the base text", () => {
  const [place] = placesFromRows([row({ change: "hunk", hunk: 0 })], "base", () => undefined);
  assert.deepEqual([place.side, place.kind], ["base", "removed"]);
});

void test("a library's file is named by its package and is marked as not openable", () => {
  const [place] = placesFromRows(
    [row({ path: "/home/u/.cargo/registry/src/index.crates.io-1/tokio-1.38.0/src/time/sleep.rs", external: true })],
    "target",
    () => assert.fail("a library file is not the changeset's"),
  );
  assert.equal(place.file.external, true);
  assert.equal(place.file.label, "tokio 1.38.0 › src/time/sleep.rs");
});
