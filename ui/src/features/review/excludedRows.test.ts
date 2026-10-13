import assert from "node:assert/strict";
import { test } from "node:test";
import type { ExcludedGroup } from "../../generated/ExcludedGroup.ts";
import { excludedRows } from "./excludedRows.ts";

const file = (path: string) => ({ path, delta: { added: 1, removed: 0 } });
const groups: ExcludedGroup[] = [
  {
    by: [{ kind: "narrowing" }],
    count: 2,
    delta: { added: 2, removed: 0 },
    files: [file("lib/a.rs"), file("lib/b.rs")],
    folders: [],
  },
  {
    by: [{ kind: "outsideRoots" }, { kind: "generated" }],
    count: 5,
    delta: { added: 5, removed: 0 },
    files: [file("Cargo.lock")],
    folders: [],
  },
];

void test("each excluded file is listed with a short reason, a file two rules leave out with both", () => {
  const rows = excludedRows(groups);
  assert.deepEqual(
    rows.map((r) => [r.name, r.why]),
    [
      ["lib/a.rs", "narrowing"],
      ["lib/b.rs", "narrowing"],
      ["Cargo.lock", "outside roots, generated"],
      ["+ 4 more", "outside roots, generated"],
    ],
  );
});
