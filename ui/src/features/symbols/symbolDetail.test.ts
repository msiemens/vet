import assert from "node:assert/strict";
import { test } from "node:test";
import { containerOf, describeFate, rowTitle } from "./symbolDetail.ts";

void test("fates read as words, the broken one as the bad tone", () => {
  assert.equal(describeFate({ type: "likelyBroken" }).tone, "bad");
  const moved = describeFate({
    type: "bindingChanged",
    now: [{ path: "u.rs", span: { start: { line: 9, col: 0 }, end: { line: 9, col: 3 } }, external: false }],
  });
  assert.deepEqual([moved.label, moved.detail], ["binds elsewhere now", "u.rs:10"]);
  assert.equal(describeFate({ type: "unknown", reason: "no server" }).detail, "no server");
});

void test("a symbol's container is the names above it, none at the top level", () => {
  assert.equal(containerOf({ path: "a.rs", name: ["Policy", "Backoff", "next"] }), "Policy::Backoff");
  assert.equal(containerOf({ path: "a.ts", name: ["Policy", "Backoff", "next"] }), "Policy.Backoff");
  assert.equal(containerOf({ path: "a.rs", name: ["run"] }), "");
});

void test("a row's tooltip gives the whole name, then where it is with a 1-based line", () => {
  const id = { path: "src/Device.cs", name: ["HES.Domain", "Device", "Create(int) : Device"] };
  assert.equal(rowTitle(id, id.path, { line: 13, col: 4 }), "HES.Domain.Device.Create\nsrc/Device.cs:14");
  assert.equal(rowTitle(null, "src/a.rs", { line: 0, col: 0 }), "src/a.rs:1");
  assert.equal(rowTitle(id, id.path), "HES.Domain.Device.Create\nsrc/Device.cs");
});
