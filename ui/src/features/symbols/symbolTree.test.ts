import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedCaller } from "../../generated/ChangedCaller.ts";
import type { Reference } from "../../generated/Reference.ts";
import type { SymbolInfo } from "../../generated/SymbolInfo.ts";
import { affectedBody, calleesBody, callerTree, isTest, reachBody, removalBody, usagesBody } from "./symbolTree.ts";

const span = (line: number) => ({ start: { line, col: 0 }, end: { line, col: 1 } });

function info(path: string, name: string[], line = 0, modules = 0): SymbolInfo {
  return { id: { path, name }, modules, kind: "function", span: span(line), at: { line, col: 0 } };
}

const caller = (symbol: SymbolInfo, depth: number): ChangedCaller => ({ symbol, change: "modified", depth });

function ref(path: string, name: string[] | null, line: number): Reference {
  return {
    location: { path, span: span(line), external: false },
    within: name && { id: { path, name }, modules: 0 },
  };
}

void test("tests are found by the conventions of their language", () => {
  assert.ok(isTest("tests/ledger_flow.rs"));
  assert.ok(isTest("src/Relays/FooTests.cs"));
  assert.ok(isTest("tests/HES.Application.Tests/Foo.cs"));
  assert.ok(isTest("ui/src/a.test.ts"));
  assert.ok(isTest("pkg/a_test.go"));
  assert.ok(isTest("src/retry.rs", ["tests", "next_delay"]));
  assert.ok(!isTest("src/Contest.cs"));
  assert.ok(!isTest("src/retry.rs", ["RetryPolicy"]));
});

void test("callers nest under what they call, production code first", () => {
  const tree = callerTree([
    caller(info("tests/a_test.rs", ["t1"]), 0),
    caller(info("src/a.rs", ["Handler", "run"]), 0),
    caller(info("tests/a_test.rs", ["t2"]), 1),
    caller(info("src/b.rs", ["Service", "go"]), 0),
  ]);
  assert.deepEqual(
    tree.map((e) => e.name),
    ["run", "go", "t1"],
  );
  const [run] = tree;
  assert.deepEqual(
    run.children.map((e) => e.name),
    ["t2"],
  );
  assert.equal(run.count, 1);
  assert.equal(run.muted, false);
  assert.equal(run.children[0].muted, true);
});

void test("a type is named once for a run of its members", () => {
  const tree = callerTree([
    caller(info("src/a.rs", ["Handler", "one"]), 0),
    caller(info("src/a.rs", ["Handler", "two"]), 0),
    caller(info("src/b.rs", ["Other", "three"]), 0),
  ]);
  assert.deepEqual(
    tree.map((e) => e.container),
    ["Handler", undefined, "Other"],
  );
});

void test("usages group by type: one member is a row, several a node, lines of one member share a row", () => {
  const body = usagesBody({
    references: [
      ref("src/a.rs", ["Client", "send"], 24),
      ref("src/a.rs", ["Client", "send"], 30),
      ref("src/b.rs", ["Pool", "get"], 4),
      ref("src/b.rs", ["Pool", "put"], 9),
      ref("src/free.rs", ["helper"], 1),
    ],
  } as never);
  const [client, pool, free] = body.entries;
  assert.equal(client.children.length, 0);
  assert.equal(client.name, "send");
  assert.equal(client.container, "Client");
  assert.equal(client.lines, ":25 :31");
  assert.equal(pool.name, "Pool");
  assert.equal(pool.count, 2);
  assert.deepEqual(
    pool.children.map((e) => e.name),
    ["get", "put"],
  );
  assert.equal(free.container, "free.rs");
  assert.equal(body.count, 5);
});

void test("a reference's glyph is only what the answer knows: a kind for a symbol, none for a name", () => {
  const usage = usagesBody({ references: [ref("src/a.rs", ["A", "m"], 1)] } as never);
  assert.equal(usage.entries[0].glyph, "other");
  const affected = affectedBody([{ ...info("src/b.rs", ["B", "n"], 2), kind: "type" }], []);
  assert.equal(affected.entries[0].glyph, "type");
});

void test("tests sit in a branch after the production usages, not among them", () => {
  const body = usagesBody({
    references: [ref("tests/x.rs", ["check"], 3), ref("src/a.rs", ["A", "m"], 1), ref("src/c.rs", ["C", "n"], 2)],
  } as never);
  assert.deepEqual(
    body.entries.map((e) => e.name),
    ["m", "n"],
  );
  const [branch] = body.tail;
  assert.equal(branch.branch, true);
  assert.equal(branch.count, 1);
  assert.equal(branch.children[0].name, "check");
});

void test("nothing to list is a quiet line, but a gap in the answer is not nothing", () => {
  assert.equal(usagesBody({ references: [], gap: null } as never).quiet, "No usages");
  assert.equal(usagesBody({ references: [], gap: "server died" } as never).quiet, undefined);
  assert.equal(affectedBody([], []).quiet, "No unchanged callers to check");
});

void test("an empty list of callees or affected callers says the analysis is incomplete when the server could not tell", () => {
  const gaps = ["yaml-language-server 1.25.0 does not offer call hierarchy"];
  for (const body of [calleesBody([], gaps), affectedBody([], gaps)]) {
    assert.equal(body.quiet, `Incomplete: ${gaps[0]}`);
  }
  assert.equal(calleesBody([], []).quiet, "No changed callees");
  const listed = affectedBody([info("src/b.rs", ["B", "n"], 2)], gaps);
  assert.equal(listed.quiet, undefined);
  assert.deepEqual(listed.notes, [{ text: `Incomplete: ${gaps[0]}`, warn: true }]);
});

void test("the way in is shown from the entry point down, each step under the one before", () => {
  const step = (name: string) => ({ id: { path: `${name}.rs`, name: [name] }, modules: 0 });
  const body = reachBody({ type: "reached", path: [step("target"), step("mid"), step("main")] });
  assert.equal(body.meta, "main");
  const [top] = body.entries;
  assert.equal(top.name, "main");
  assert.equal(top.children[0].name, "mid");
  assert.equal(top.children[0].children[0].name, "target");
  assert.equal(top.fixed, true);
});

void test("remaining references split into what needs attention (open) and what is handled", () => {
  const site = (line: number, fate: object) => ({ base: ref("src/a.rs", ["A", "m"], line), fate });
  const body = removalBody({
    callSites: [
      site(1, { type: "gone" }),
      site(2, { type: "likelyBroken" }),
      site(3, { type: "unknown", reason: "no server" }),
      site(4, { type: "modified" }),
    ],
    gap: null,
  } as never);
  const [attention, handled] = body.entries;
  assert.equal(attention.open, true);
  assert.deepEqual(
    attention.children.map((e) => e.fate?.label),
    ["likely broken", "unknown"],
  );
  assert.equal(attention.children[1].sub, "a.rs:4 · no server");
  assert.equal(handled.open, undefined);
  assert.equal(handled.count, 2);
});
