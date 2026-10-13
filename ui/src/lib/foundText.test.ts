import assert from "node:assert/strict";
import { test } from "node:test";
import { codeRuns, libraryLabel, segments, trimmed } from "./foundText.ts";
import { decodeHighlights } from "./highlights.ts";

void test("a line is cut where its colours change and where the match is, in UTF-16 units", () => {
  // `let` is a keyword (0..3), `foo` a function (10..13) after "é😀", which is three units.
  const runs = codeRuns({
    text: "let é😀 = foo();",
    highlights: [0, 3, 0, 7, 3, 5],
    matchStart: 10,
    matchEnd: 13,
  });
  assert.deepEqual(
    runs.map((r) => [r.text, r.className, r.hit]),
    [
      ["let", "syn-keyword", false],
      [" é😀 = ", undefined, false],
      ["foo", "syn-function", true],
      ["();", undefined, false],
    ],
  );
});

void test("a match inside a coloured stretch splits it and keeps its colour", () => {
  const runs = codeRuns({ text: "next_delay(3)", highlights: [0, 10, 5], matchStart: 5, matchEnd: 8 });
  assert.deepEqual(
    runs.map((r) => [r.text, r.className, r.hit]),
    [
      ["next_", "syn-function", false],
      ["del", "syn-function", true],
      ["ay", "syn-function", false],
      ["(3)", undefined, false],
    ],
  );
  assert.deepEqual(codeRuns({ text: "", highlights: [], matchStart: 0, matchEnd: 0 }), []);
});

void test("a library's file is named by its package where the path tells", () => {
  assert.deepEqual(
    libraryLabel("/home/u/.cargo/registry/src/index.crates.io-6f17d22bba15001f/backoff-0.4.0/src/exponential.rs"),
    {
      package: "backoff 0.4.0",
      rest: "src/exponential.rs",
    },
  );
  assert.deepEqual(libraryLabel("/p/node_modules/@scope/pkg/dist/index.d.ts"), {
    package: "@scope/pkg",
    rest: "dist/index.d.ts",
  });
  assert.deepEqual(libraryLabel("/usr/lib/rustlib/src/core/option.rs"), {
    rest: "/usr/lib/rustlib/src/core/option.rs",
  });
});

void test("a row leaves out the line's indent, and the matches' columns follow", () => {
  const line = (text: string, hits: { from: number; to: number }[], textStart = 0) => ({ text, textStart, hits });
  assert.deepEqual(trimmed(line("    let delay = 1;", [{ from: 8, to: 13 }])), {
    text: "let delay = 1;",
    hits: [{ from: 4, to: 9 }],
  });
  // A cut line starts at a column of its own.
  assert.deepEqual(trimmed(line("elay = 1;", [{ from: 101, to: 105 }], 100)).hits, [{ from: 1, to: 5 }]);
});

void test("matches cut a coloured stretch, and the current one is told apart", () => {
  // "let" is a keyword: [gap 0, length 3, kind 0].
  const highlights = decodeHighlights([0, 3, 0]);
  const hits = [
    { from: 2, to: 9 },
    { from: 9, to: 10 },
  ];
  assert.deepEqual(segments("let delay;", 0, highlights, hits, 1), [
    { text: "le", className: "syn-keyword" },
    { text: "t", className: "syn-keyword", find: "hit" },
    { text: " delay", className: undefined, find: "hit" },
    { text: ";", className: undefined, find: "current" },
  ]);
});
