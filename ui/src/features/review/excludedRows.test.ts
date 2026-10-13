import assert from "node:assert/strict";
import { test } from "node:test";
import type { ExcludedGroup } from "../../generated/ExcludedGroup.ts";
import { excludedRows, extraRows } from "./excludedRows.ts";

const file = (path: string) => ({ path, delta: { added: 1, removed: 0 } });
const groups: ExcludedGroup[] = [
  { by: { kind: "narrowing" }, count: 2, delta: { added: 2, removed: 0 }, files: [file("lib/a.rs"), file("lib/b.rs")] },
  { by: { kind: "generated" }, count: 5, delta: { added: 5, removed: 0 }, files: [file("Cargo.lock")] },
];

void test("each excluded file is listed with why the review leaves it out, a narrowing apart from the project's rules", () => {
  const rows = excludedRows(groups);
  assert.deepEqual(
    rows.map((r) => [r.name, r.why]),
    [
      ["lib/a.rs", "Outside the narrowing"],
      ["lib/b.rs", "Outside the narrowing"],
      ["Cargo.lock", "Generated"],
      ["+ 4 more", "Generated"],
    ],
  );
});

void test("the group's files follow its row only while it is open, and there is no row without excluded changes", () => {
  const rows = excludedRows(groups);
  assert.equal(extraRows(true, false, rows), 1);
  assert.equal(extraRows(true, true, rows), 5);
  assert.equal(extraRows(false, true, rows), 0);
});
