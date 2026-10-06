// Pure position logic of the semantic layer: protocol positions (0-based line, UTF-16 column)
// to editor offsets, and base lines to the target lines a unified diff shows them at.

import type { Hunk } from "../../../generated/Hunk";
import type { Position } from "../../../generated/Position";
import type { SemanticLocation } from "../../../generated/SemanticLocation";

/** What of CodeMirror's `Text` is needed: 1-based lines. */
export interface LineDoc {
  lines: number;
  line(n: number): { from: number; to: number };
}

/** The offset of a protocol position, clamped into the document (an answer may outlive an edit). */
export function offsetAt(doc: LineDoc, at: Position): number {
  if (at.line >= doc.lines) return doc.line(doc.lines).to;
  const line = doc.line(at.line + 1);
  return Math.min(line.from + at.col, line.to);
}

/**
 * The target line a base line is shown at in a unified diff: lines between hunks move by what the
 * hunks before them added or removed, and a line a hunk replaced or deleted maps to the hunk's
 * start (where its deleted lines are drawn).
 */
export function baseLineToTarget(hunks: readonly Hunk[], baseLine: number): number {
  let delta = 0;
  for (const h of hunks) {
    if (baseLine < h.baseStart) break;
    if (baseLine < h.baseStart + h.baseLen) return h.targetStart;
    delta = h.targetStart + h.targetLen - (h.baseStart + h.baseLen);
  }
  return baseLine + delta;
}

/** Several definitions: one inside the repository wins, as only that one can be opened. */
export function pickLocation(locations: readonly SemanticLocation[]): SemanticLocation | undefined {
  return locations.find((l) => !l.external) ?? locations[0];
}
