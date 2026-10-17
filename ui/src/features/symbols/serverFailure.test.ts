import assert from "node:assert/strict";
import { test } from "node:test";
import type { Gap } from "../../generated/Gap.ts";
import type { Unavailable } from "../../generated/Unavailable.ts";
import { sharedFailure } from "./serverFailure.ts";
import { affectedBody, calleesBody, callersBody, reachBody, usagesBody, type Body } from "./symbolTree.ts";

const failed = (message: string, kind: Unavailable["kind"] = "serverFailed"): Unavailable => ({
  kind,
  label: "Language server failed",
  message,
  language: "rust",
});
const gap = (why: Unavailable): Gap => ({ type: "failed", what: "incoming calls", why });

const START = failed("rust-analyzer did not start\nerror: no default toolchain");

const words = (body: Body): string => [...body.notes.map((n) => n.text), body.quiet ?? ""].join("\n");

void test("a server that fails to start is stated once, and no section repeats it", () => {
  const calls = { callers: [], callees: [], affected: [], gaps: [gap(START)] } as never;
  const answers = {
    signature: { unavailable: START },
    calls,
    usages: { references: [], gap: START },
    reach: { reach: { type: "unknown", gaps: [gap(START)] } },
  } as never;
  const shared = sharedFailure(answers);
  assert.equal(shared?.message, START.message);

  const bodies = [
    callersBody(calls, shared),
    calleesBody([], [gap(START)], shared),
    affectedBody([], [gap(START)], shared),
    usagesBody({ references: [], gap: START } as never, shared),
    reachBody({ type: "unknown", gaps: [gap(START)] }, shared),
  ];
  for (const body of bodies) {
    assert.equal(body.hidden, true);
    assert.doesNotMatch(words(body), /did not start/);
  }
});

void test("a section that fails for its own reason keeps its note", () => {
  const other = failed("references timed out", "queryFailed");
  const shared = sharedFailure({ calls: { gaps: [gap(START)] }, usages: { gap: other } } as never);
  assert.equal(shared?.message, START.message);

  const usages = usagesBody({ references: [], gap: other } as never, shared);
  assert.notEqual(usages.hidden, true);
  assert.match(words(usages), /references timed out/);

  const mixed = calleesBody([], [gap(START), gap(other)], shared);
  assert.match(words(mixed), /references timed out/);
  assert.doesNotMatch(words(mixed), /did not start/);
});

void test("without a server failure nothing is hidden", () => {
  const other = failed("query broke", "queryFailed");
  assert.equal(sharedFailure({ usages: { gap: other } } as never), undefined);
  assert.match(words(usagesBody({ references: [], gap: other } as never, undefined)), /query broke/);
});
