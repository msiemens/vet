import assert from "node:assert/strict";
import { test } from "node:test";
import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import type { FileLayout } from "../../../state/reviewModel.ts";
import type { TextChange } from "../../../generated/TextChange.ts";
import { backendDiff } from "./backendDiff.ts";
import { replacesBaseTail } from "./baseTail.ts";
import { sectionLayout } from "./sectionLayout.ts";
import { WrapRows, rowsOf } from "./wrapRows.ts";

// The changes are what `vet_app::editor::text_changes` answers for the texts; its test
// `files_with_and_without_a_final_newline` pins the same numbers.
const change = (fromBase: number, toBase: number, fromTarget: number, toTarget: number): TextChange => ({
  fromBase,
  toBase,
  fromTarget,
  toTarget,
});

interface Case {
  name: string;
  base: string;
  target: string;
  /** The hunk's `[baseStart, baseLen, targetStart, targetLen]`. */
  hunk: [number, number, number, number];
  changes: TextChange[];
  /** Whether the merge extension takes the base's empty last line for deleted. */
  tail: boolean;
  /** The rows of a side by side view as measured in the app, hunk bars included (one row each). */
  split: number;
}

const cases: Case[] = [
  {
    name: "an added file without a final newline",
    base: "",
    target: "alpha\nbravo\ncharlie",
    hunk: [0, 0, 0, 3],
    changes: [change(0, 0, 0, 19)],
    tail: true,
    split: 1 + 3,
  },
  {
    name: "an added file with a final newline",
    base: "",
    target: "alpha\nbravo\ncharlie\n",
    hunk: [0, 0, 0, 3],
    changes: [change(0, 0, 0, 20)],
    tail: false,
    split: 1 + 4,
  },
  {
    name: "a deleted file without a final newline",
    base: "alpha\nbravo\ncharlie",
    target: "",
    hunk: [0, 3, 0, 0],
    changes: [change(0, 19, 0, 0)],
    tail: false,
    split: 2 + 4,
  },
  {
    name: "a deleted file with a final newline",
    base: "alpha\nbravo\ncharlie\n",
    target: "",
    hunk: [0, 3, 0, 0],
    changes: [change(0, 20, 0, 0)],
    tail: false,
    split: 2 + 4,
  },
  {
    name: "lines appended to a file, the last one without a newline",
    base: "alpha\n",
    target: "alpha\nbravo\ncharlie",
    hunk: [1, 0, 1, 2],
    changes: [change(6, 5, 6, 19)],
    tail: false,
    split: 1 + 3,
  },
  {
    name: "the last line of a file gains its newline",
    base: "alpha\nbravo",
    target: "alpha\nbravo\n",
    hunk: [1, 1, 1, 1],
    changes: [change(11, 11, 11, 11)],
    tail: false,
    split: 1 + 3,
  },
  {
    name: "the last line of a file loses its newline",
    base: "alpha\nbravo\n",
    target: "alpha\nbravo",
    hunk: [1, 1, 1, 1],
    changes: [change(11, 11, 11, 11)],
    tail: false,
    split: 1 + 3,
  },
  {
    name: "the only line of a file gains its newline",
    base: "bravo",
    target: "bravo\n",
    hunk: [0, 1, 0, 1],
    changes: [change(5, 5, 5, 5)],
    tail: false,
    split: 1 + 2,
  },
  {
    name: "the only line of a file loses its newline",
    base: "bravo\n",
    target: "bravo",
    hunk: [0, 1, 0, 1],
    changes: [change(5, 5, 5, 5)],
    tail: false,
    split: 1 + 2,
  },
];

const chunksOf = ({ base, target, changes }: Case) =>
  Chunk.build(Text.of(base.split("\n")), Text.of(target.split("\n")), {
    override: backendDiff(base, target, changes),
  });

void test("only a chunk that adds lines to an empty base takes the base's empty last line for deleted", () => {
  for (const c of cases) {
    const chunks = chunksOf(c);
    assert.equal(chunks.length, 1, c.name);
    assert.equal(replacesBaseTail(chunks[0], c.base.length), c.tail, c.name);
  }
});

/** The rows a unified view draws: the target's lines, and the deleted lines of the chunks that delete some. */
function unifiedRows(c: Case, rowsOfLine: (text: string) => number): number {
  const target = Text.of(c.target.split("\n"));
  const base = Text.of(c.base.split("\n"));
  let rows = 0;
  for (let n = 1; n <= target.lines; n++) rows += rowsOfLine(target.line(n).text);
  for (const chunk of chunksOf(c)) {
    if (chunk.toA === chunk.fromA || replacesBaseTail(chunk, base.length)) continue;
    for (let n = base.lineAt(chunk.fromA).number; n <= base.lineAt(chunk.endA).number; n++)
      rows += rowsOfLine(base.line(n).text);
  }
  return rows;
}

const file = (c: Case): FileLayout => {
  const [baseStart, baseLen, targetStart, targetLen] = c.hunk;
  return {
    path: "a.txt",
    baseLines: c.base.split("\n").length,
    targetLines: c.target.split("\n").length,
    hunks: [{ baseStart, baseLen, targetStart, targetLen }],
    nonText: [],
  } as unknown as FileLayout;
};

/** A section in rows, from the height model: one-px rows and bars, no header or gap. */
const modelRows = (c: Case, mode: "unified" | "split", wrapped?: { base: WrapRows; target: WrapRows }) =>
  sectionLayout(
    file(c),
    mode,
    { line: 1, hunk: 1, header: 0, collapsed: 0, gap: 0, preview: 0, diffGap: 0 },
    false,
    undefined,
    wrapped,
  ).height;

void test("the height model counts the rows a unified view draws, with or without a final newline", () => {
  for (const c of cases) assert.equal(modelRows(c, "unified"), 1 + unifiedRows(c, () => 1), c.name);
});

void test("the height model counts the rows of wrapped lines the same way", () => {
  const columns = 3;
  for (const c of cases) {
    const wrapped = { base: new WrapRows(c.base, columns), target: new WrapRows(c.target, columns) };
    const rows = 1 + unifiedRows(c, (text) => rowsOf(text, 0, text.length, columns, 4));
    assert.equal(modelRows(c, "unified", wrapped), rows, c.name);
  }
});

void test("the height model counts the rows a side by side view draws, with or without a final newline", () => {
  for (const c of cases) assert.equal(modelRows(c, "split"), c.split, c.name);
});
