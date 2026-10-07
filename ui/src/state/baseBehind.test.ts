import assert from "node:assert/strict";
import { test } from "node:test";
import type { Base } from "../generated/Base.ts";
import { behindText } from "./baseBehind.ts";

const base = (choice: Base["choice"], behind?: number) => ({ choice, behind }) as Base;

void test("only a chosen branch that has moved is behind its remote", () => {
  assert.equal(behindText(base({ type: "branch", name: "master" }, 11)), "11 behind origin/master");
  assert.equal(behindText(base({ type: "branch", name: "master" })), undefined);
  assert.equal(behindText(base({ type: "mergeBase", reference: "master" }, 3)), undefined);
  assert.equal(behindText(undefined), undefined);
});
