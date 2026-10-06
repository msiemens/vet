// The rows of one file's diff in the approval dialog: its hunks with a little context.

import type { ApprovalFile } from "../../generated/ApprovalFile";

export type Row =
  | { kind: "ctx" | "add" | "del"; n: number; text: string }
  /** Unchanged lines left out. */
  | { kind: "gap" };

export const CONTEXT = 2;
/** A file with more rows is cut; the dialog says so. */
export const MAX_ROWS = 400;

function lines(text: string | null): string[] {
  if (!text) return [];
  const all = text.split("\n");
  if (all.at(-1) === "") all.pop();
  return all;
}

export function lineCounts(file: ApprovalFile): { added: number; removed: number } {
  return {
    added: file.hunks.reduce((sum, h) => sum + h.targetLen, 0),
    removed: file.hunks.reduce((sum, h) => sum + h.baseLen, 0),
  };
}

/** `undefined` if there is no text to show (binary, over the capture cap). */
export function approvalRows(file: ApprovalFile): { rows: Row[]; truncated: boolean } | undefined {
  if ((file.base === null && file.change !== "added") || (file.target === null && file.change !== "removed")) {
    return undefined;
  }
  const base = lines(file.base);
  const target = lines(file.target);
  const rows: Row[] = [];
  const context = (from: number, to: number) => {
    for (let i = from; i < to; i++) rows.push({ kind: "ctx", n: i + 1, text: target[i] });
  };

  if (!file.hunks.length) return { rows, truncated: false };
  let cursor = 0;
  for (const [index, hunk] of file.hunks.entries()) {
    const start = hunk.targetStart;
    if (index === 0) {
      if (start > CONTEXT) rows.push({ kind: "gap" });
      context(Math.max(0, start - CONTEXT), start);
    } else if (start - cursor <= 2 * CONTEXT) {
      context(cursor, start);
    } else {
      context(cursor, cursor + CONTEXT);
      rows.push({ kind: "gap" });
      context(start - CONTEXT, start);
    }
    for (let i = 0; i < hunk.baseLen; i++)
      rows.push({ kind: "del", n: hunk.baseStart + i + 1, text: base[hunk.baseStart + i] });
    for (let i = 0; i < hunk.targetLen; i++) rows.push({ kind: "add", n: start + i + 1, text: target[start + i] });
    cursor = start + hunk.targetLen;
  }
  const tail = Math.min(target.length, cursor + CONTEXT);
  context(cursor, tail);
  if (tail < target.length) rows.push({ kind: "gap" });

  const truncated = rows.length > MAX_ROWS;
  return { rows: truncated ? rows.slice(0, MAX_ROWS) : rows, truncated };
}
