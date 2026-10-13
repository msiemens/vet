import assert from "node:assert/strict";
import { test } from "node:test";
import { type SearchText, compile, lineHits } from "./query.ts";

const q = (text: string, over: Partial<SearchText> = {}): SearchText => ({
  text,
  matchCase: false,
  wholeWord: false,
  regex: false,
  ...over,
});

function hits(query: SearchText, line: string) {
  const c = compile(query);
  if (c.type !== "ok") throw new Error(`the query did not compile: ${c.type}`);
  return lineHits(c.re, line).map((h) => line.slice(h.from, h.to));
}

void test("a literal is not a pattern", () => {
  assert.deepEqual(hits(q("a.b"), "a.b axb"), ["a.b"]);
  assert.deepEqual(hits(q("(x)"), "f(x)"), ["(x)"]);
});

void test("case matters only when asked", () => {
  assert.deepEqual(hits(q("Retry"), "retry RETRY"), ["retry", "RETRY"]);
  assert.deepEqual(hits(q("Retry", { matchCase: true }), "retry Retry"), ["Retry"]);
});

void test("a whole word does not match inside a longer one", () => {
  assert.deepEqual(hits(q("delay", { wholeWord: true }), "delay max_delay delays delay"), ["delay", "delay"]);
  assert.deepEqual(hits(q("max|min", { regex: true, wholeWord: true }), "max maximum min"), ["max", "min"]);
});

// The same queries are run by Find in Files' matcher (`crates/vet-infra/src/search.rs`): what a plain
// query finds must not depend on which of the two runs it.
const PARITY: [string, { matchCase: boolean; wholeWord: boolean; regex: boolean }, string, string[]][] = [
  [
    "über",
    { matchCase: false, wholeWord: true, regex: false },
    "über Über überall xüber über",
    ["über", "Über", "über"],
  ],
  ["Straße", { matchCase: true, wholeWord: true, regex: false }, "Straße Straßen", ["Straße"]],
  ["é", { matchCase: false, wholeWord: true, regex: false }, "café é", ["é"]],
  ["delay", { matchCase: false, wholeWord: true, regex: false }, "max_delay delay delays", ["delay"]],
  ["ÜBER", { matchCase: false, wholeWord: false, regex: false }, "über", ["über"]],
  ["ab|cd", { matchCase: true, wholeWord: true, regex: true }, "ab abc cd xcd", ["ab", "cd"]],
  ["日本", { matchCase: false, wholeWord: true, regex: false }, "日本語 日本", ["日本"]],
  ["a.b", { matchCase: true, wholeWord: false, regex: false }, "a.b axb", ["a.b"]],
  ["a b", { matchCase: false, wholeWord: true, regex: false }, "a b, a bc", ["a b"]],
  [".foo", { matchCase: false, wholeWord: true, regex: false }, "x.foo .foo", [".foo"]],
];

void test("plain queries find what Find in Files' matcher finds", () => {
  for (const [text, options, line, expected] of PARITY)
    assert.deepEqual(hits(q(text, options), line), expected, `${text} in ${line}`);
});

void test("a regular expression is a pattern, and an empty match is none", () => {
  assert.deepEqual(hits(q("next_\\w+", { regex: true }), "next_delay(next_x)"), ["next_delay", "next_x"]);
  assert.deepEqual(hits(q("x*", { regex: true }), "abc"), []);
});

void test("an invalid pattern says why, without the pattern", () => {
  const c = compile(q("(", { regex: true }));
  assert.deepEqual(c.type === "error" && !c.message.startsWith("Invalid regular expression"), true);
  assert.equal(compile(q("")).type, "none");
});
