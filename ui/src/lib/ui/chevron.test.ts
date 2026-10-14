import assert from "node:assert/strict";
import { test } from "node:test";
import { onChevron } from "./chevron.ts";

/** A click target whose ancestors carry `classes`, as `closest` finds them. */
const inside = (...classes: string[]) =>
  ({ target: { closest: (selector: string) => (classes.includes(selector.slice(1)) ? {} : null) } }) as never;

void test("a click on the chevron or inside it is on the chevron", () => {
  assert.equal(onChevron(inside("chev")), true);
});

void test("a click elsewhere in the row is not", () => {
  assert.equal(onChevron(inside("name")), false);
  assert.equal(onChevron(inside()), false);
});

void test("a target that cannot be searched is not the chevron", () => {
  assert.equal(onChevron({ target: null }), false);
  assert.equal(onChevron({ target: {} as EventTarget }), false);
});
