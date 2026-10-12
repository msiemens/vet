// What the location picker lists: the rows of a search as the popup draws them, either grouped by
// file under the changeset, the other files and the libraries, or as a short flat list.

import type { LocationRow } from "../../generated/LocationRow.ts";
import { match } from "../../lib/speedSearch.ts";

/** Up to this many places the popup lists them flat, two lines each; above, grouped by file. */
export const FLAT_UP_TO = 20;

export type Section = "CHANGESET" | "OTHER FILES" | "LIBRARIES";

export type Item =
  | { kind: "section"; label: Section }
  | { kind: "file"; path: string; status?: string; count: number; external: boolean }
  | { kind: "hit"; row: LocationRow; at: number };

/** The places that survive what the viewer typed and the changeset toggle. */
export function visibleRows(rows: readonly LocationRow[], query: string, onlyChangeset: boolean): LocationRow[] {
  return rows.filter((row) => {
    if (onlyChangeset && (row.external || row.change === "unchanged")) return false;
    if (!query) return true;
    const within = row.within?.name.join("::") ?? "";
    return [within, row.text, row.path].some((text) => match(query, text) !== undefined);
  });
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

  const items: Item[] = [];
  let at = 0;
  for (const label of ["CHANGESET", "OTHER FILES", "LIBRARIES"] as const) {
    if (!blocks[label].length) continue;
    items.push({ kind: "section", label });
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

/** A library's file: the package it belongs to, if the path tells, and the rest of the path. */
export function libraryLabel(path: string): { package?: string; rest: string } {
  const slashed = path.replaceAll("\\", "/");
  const crate = /\/registry\/src\/[^/]+\/(.+)-(\d[^/]*)\/(.+)$/.exec(slashed);
  if (crate) return { package: `${crate[1]} ${crate[2]}`, rest: crate[3] };
  const module = /\/node_modules\/((?:@[^/]+\/)?[^/]+)\/(.+)$/.exec(slashed);
  if (module) return { package: module[1], rest: module[2] };
  return { rest: path };
}
