import assert from "node:assert/strict";
import { test } from "node:test";
import { avatarColor } from "./avatar.ts";

void test("a name keeps its avatar colour, within the palette", () => {
  assert.equal(avatarColor("/src/ledger-sync"), avatarColor("/src/ledger-sync"));
  for (let i = 0; i < 60; i++) {
    const colour = avatarColor(`/src/project-${i}`);
    assert.ok(Number.isInteger(colour) && colour >= 1 && colour <= 6);
  }
  const used = new Set(Array.from({ length: 60 }, (_, i) => avatarColor(`/src/project-${i}`)));
  assert.ok(used.size > 1, "names spread over the palette");
});

void test("long names do not overflow into a colour outside the palette", () => {
  for (let i = 0; i < 200; i++) {
    const colour = avatarColor(`/Users/someone/Documents/Coding/Privat/project-${i}/é`);
    assert.ok(colour >= 1 && colour <= 6, `name ${i}`);
  }
});
