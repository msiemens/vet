// What the location picker lists: the rows of a search as the popup draws them, either grouped by
// file under the changeset, the other files and the libraries (usages), or as a flat list
// (implementations).

import type { LocationRow } from "../../generated/LocationRow.ts";
import { match } from "../../lib/speedSearch.ts";
import { placeLabel } from "../../lib/symbolLabel.ts";

export type Section = "CHANGESET" | "OTHER FILES" | "LIBRARIES";

export type Item =
  | { kind: "section"; label: Section }
  | { kind: "file"; path: string; status?: string; count: number; external: boolean }
  | { kind: "hit"; row: LocationRow; at: number };

/** Places in changed hunks come first, then those in changed files, then other files, then libraries. */
const tier = (row: LocationRow): number =>
  row.external ? 3 : row.change === "hunk" ? 0 : row.change === "file" ? 1 : 2;

/**
 * The places that survive what the viewer typed and the changeset toggle, in the order a flat list
 * shows them: by tier, then by path and line.
 */
export function visibleRows(rows: readonly LocationRow[], query: string, onlyChangeset: boolean): LocationRow[] {
  return rows
    .filter((row) => {
      if (onlyChangeset && (row.external || row.change === "unchanged")) return false;
      if (!query) return true;
      return [symbolLabel(row), row.text, row.path].some((text) => match(query, text) !== undefined);
    })
    .sort((a, b) => tier(a) - tier(b) || a.path.localeCompare(b.path) || a.line - b.line || a.col - b.col);
}

/**
 * Where the searched `symbol` is in the row's text, for a list of implementations, where the place
 * found is the implementing type and the name searched for stands elsewhere in the line; the row's
 * own match where the name does not occur.
 */
export function matchedRow(row: LocationRow, symbol: string): LocationRow {
  const word = new RegExp(
    `(?<![\\p{L}\\p{N}_])${symbol.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?![\\p{L}\\p{N}_])`,
    "u",
  );
  const found = word.exec(row.text);
  return found ? { ...row, matchStart: found.index, matchEnd: found.index + symbol.length } : row;
}

/**
 * The places grouped by file: the changeset's files first, those with a place in a changed hunk before
 * the others; then the other files; then the libraries. A file lists its places in line order. `at`
 * numbers the places in the order shown, which is the order the keys move through them.
 */
export function groupRows(rows: readonly LocationRow[], statusOf: (path: string) => string | undefined): Item[] {
  const files = new Map<string, LocationRow[]>();
  for (const row of rows) files.set(row.path, [...(files.get(row.path) ?? []), row]);
  const blocks: Record<Section, { path: string; rows: LocationRow[]; hunk: boolean }[]> = {
    CHANGESET: [],
    "OTHER FILES": [],
    LIBRARIES: [],
  };
  for (const [path, inFile] of files) {
    const first = inFile[0];
    const section: Section = first.external
      ? "LIBRARIES"
      : inFile.some((r) => r.change !== "unchanged")
        ? "CHANGESET"
        : "OTHER FILES";
    blocks[section].push({
      path,
      rows: [...inFile].sort((a, b) => a.line - b.line || a.col - b.col),
      hunk: inFile.some((r) => r.change === "hunk"),
    });
  }
  blocks.CHANGESET.sort((a, b) => Number(b.hunk) - Number(a.hunk) || a.path.localeCompare(b.path));
  blocks["OTHER FILES"].sort((a, b) => a.path.localeCompare(b.path));
  blocks.LIBRARIES.sort((a, b) => a.path.localeCompare(b.path));

  // The changeset's label tells it from the other sections; with nothing to tell it from, it is left out.
  const lone = blocks.CHANGESET.length > 0 && !blocks["OTHER FILES"].length && !blocks.LIBRARIES.length;
  const items: Item[] = [];
  let at = 0;
  for (const label of ["CHANGESET", "OTHER FILES", "LIBRARIES"] as const) {
    if (!blocks[label].length) continue;
    if (!lone) items.push({ kind: "section", label });
    for (const file of blocks[label]) {
      items.push({
        kind: "file",
        path: file.path,
        status: label === "CHANGESET" ? statusOf(file.path) : undefined,
        count: file.rows.length,
        external: label === "LIBRARIES",
      });
      for (const row of file.rows) items.push({ kind: "hit", row, at: at++ });
    }
  }
  return items;
}

/** The header's count: `1,340`, with how many of them are listed when the answer was cut. */
export function countText(total: number, listed: number): { count: string; cut?: string } {
  const format = (n: number) => n.toLocaleString("en-US");
  return total > listed ? { count: format(total), cut: `first ${format(listed)}` } : { count: format(total) };
}

/**
 * Whether a popup that may grow to `max` opens below the caret (`anchor`, in a viewport `viewport`
 * high): when it fits there at full height, else on the roomier side. Nothing about the content
 * enters, so the side is the same for "Searching…", every result list and every filter of it.
 */
export function opensBelow(anchor: { top: number; bottom: number }, viewport: number, max: number, gap = 12): boolean {
  const below = viewport - anchor.bottom - gap;
  return below >= max || below >= anchor.top - gap;
}

/**
 * What a place is in, short: the innermost symbol around it with its container, outside the
 * namespaces (`Ledger::commit`, `TaskDispatcher.Trigger`), else just the symbol; the file's name
 * where there is no symbol.
 */
export const symbolLabel = (row: Pick<LocationRow, "within">): string => placeLabel(row.within);
