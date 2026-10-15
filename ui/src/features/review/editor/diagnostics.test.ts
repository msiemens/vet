import assert from "node:assert/strict";
import { test } from "node:test";
import { forEachDiagnostic, type Diagnostic as LintDiagnostic, linter, setDiagnostics } from "@codemirror/lint";
import { EditorState } from "@codemirror/state";
import type { Diagnostic } from "../../../generated/Diagnostic.ts";
import { collapsedField, setCollapsed } from "./collapse.ts";
import { diagnosticList, diagnosticMarks, drawDiagnostics, lintDecorations, setDiagnosticList } from "./diagnostics.ts";

const DOC = "aaa\nbbb\nccc\nddd\neee";

/** A diagnostic on the 0-based `line`, columns `from`..`to`. */
const diagnostic = (line: number, from: number, to: number, severity: "error" | "warning"): Diagnostic => ({
  span: { start: { line, col: from }, end: { line, col: to } },
  severity,
  message: `${severity} here`,
  source: "test",
  code: null,
  origin: "new",
});

const classes = (marks: ReturnType<typeof drawDiagnostics>, length: number) => {
  const found: string[] = [];
  marks.between(0, length, (from, to, deco) => {
    found.push(`${from}-${to}:${(deco.spec as { class?: string }).class}`);
  });
  return found;
};

void test("a diagnostic inside a collapsed region is not drawn; one outside it is", () => {
  const doc = EditorState.create({ doc: DOC }).doc;
  const list = [diagnostic(2, 0, 3, "error")]; // line 2, "ccc", at 8..11
  // No collapse: the mark is on line 2.
  assert.deepEqual(classes(drawDiagnostics(list, [], doc), doc.length), ["8-11:cm-lintRange cm-lintRange-error"]);
  // A region hiding line 2 draws nothing.
  assert.equal(drawDiagnostics(list, [{ from: 8, to: 15 }], doc).size, 0);
  // A region above it leaves the mark as it is.
  assert.deepEqual(classes(drawDiagnostics(list, [{ from: 0, to: 7 }], doc), doc.length), [
    "8-11:cm-lintRange cm-lintRange-error",
  ]);
});

void test("the drawn marks follow a collapse the state changed, with no update of their own", () => {
  let state = EditorState.create({
    doc: DOC,
    extensions: [linter(null), diagnosticList, diagnosticMarks, collapsedField],
  });
  state = state.update({ effects: setDiagnosticList.of([diagnostic(2, 0, 3, "warning")]) }).state;
  assert.deepEqual(classes(state.field(diagnosticMarks), state.doc.length), ["8-11:cm-lintRange cm-lintRange-warning"]);
  // Collapsing over the diagnostic hides its mark; expanding brings it back.
  state = state.update({ effects: setCollapsed.of([{ from: 8, to: 15 }]) }).state;
  assert.equal(state.field(diagnosticMarks).size, 0);
  state = state.update({ effects: setCollapsed.of([]) }).state;
  assert.deepEqual(classes(state.field(diagnosticMarks), state.doc.length), ["8-11:cm-lintRange cm-lintRange-warning"]);
});

void test("the merged marks match the ones @codemirror/lint draws, overlaps and points included", () => {
  const doc = "aaa\nbbb\nccc\nddd\neee\nfff";
  const list: LintDiagnostic[] = [
    { from: 1, to: 12, severity: "warning", message: "w" },
    { from: 8, to: 20, severity: "error", message: "e" },
    { from: 8, to: 11, severity: "info", message: "i" },
    { from: 22, to: 22, severity: "hint", message: "point" },
  ];
  const base = EditorState.create({ doc, extensions: [linter(null)] });
  const state = base.update(setDiagnostics(base, list)).state;
  const lint: string[] = [];
  forEachDiagnostic(state, (d, from, to) => lint.push(`${from}-${to}:${d.message}`));
  const owned: string[] = [];
  const seen = new Set<LintDiagnostic>();
  lintDecorations(list, base.doc).between(0, base.doc.length, (_, __, deco) => {
    for (const d of (deco.spec as { diagnostics?: readonly LintDiagnostic[] }).diagnostics ?? []) {
      if (seen.has(d)) continue;
      seen.add(d);
      owned.push(`${d.from}-${d.to}:${d.message}`);
    }
  });
  assert.deepEqual(owned.sort(), lint.sort());
});
