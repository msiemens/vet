// Height of a file section estimated from the manifest alone, so a column of sections
// can be laid out before any text is loaded (SPEC §10). With `collapse`, unchanged regions
// between hunks become one bar each, mirroring `collapseUnchanged` of `@codemirror/merge`.

import type { FileChange } from "../../../generated/FileChange";

/** Context kept around each hunk, and the smallest unchanged region worth a bar. */
export const COLLAPSE = { margin: 3, minSize: 4 };

/** Row heights in px; they come from the theme tokens. */
export interface Metrics {
  line: number;
  hunk: number;
  header: number;
  collapsed: number;
}

/** Rows and bars an unchanged region of `lines` lines takes, with `context` lines kept at its edges. */
function unchanged(lines: number, context: number): { rows: number; bars: number } {
  return lines - context >= COLLAPSE.minSize ? { rows: context, bars: 1 } : { rows: Math.max(0, lines), bars: 0 };
}

/** `mode` is the editor's `Mode`; spelled out so this module needs no DOM or CodeMirror types. */
export function sectionHeight(file: FileChange, mode: "unified" | "split", m: Metrics, collapse: boolean): number {
  const { margin } = COLLAPSE;
  let y = m.header;
  let end = 0; // where the previous hunk ends in the target, in lines
  const gap = (lines: number, context: number) => {
    const u = collapse ? unchanged(lines, context) : { rows: Math.max(0, lines), bars: 0 };
    y += u.rows * m.line + u.bars * m.collapsed;
  };
  file.hunks.forEach((h, i) => {
    gap(h.targetStart - end, i === 0 ? margin : 2 * margin);
    // Unified shows each deleted line as an extra row; side by side pads the shorter side.
    const rows = mode === "unified" ? h.baseLen + h.targetLen : Math.max(h.baseLen, h.targetLen);
    y += m.hunk + rows * m.line;
    end = h.targetStart + h.targetLen;
  });
  gap(file.targetLines - end, file.hunks.length ? margin : 0);
  return y;
}
