// What the location picker lists: the rows of a search as the popup draws them, either grouped by
// file under the changeset, the other files and the libraries, or as a short flat list.

import type { LocationRow } from "../../generated/LocationRow.ts";
import { decodeHighlights, eachRange } from "../../lib/highlights.ts";
import { splitQualified } from "../../lib/qualified.ts";
import { match } from "../../lib/speedSearch.ts";
import { rowLabel } from "../../lib/symbolLabel.ts";

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
    return [symbolLabel(row), row.text, row.path].some((text) => match(query, text) !== undefined);
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

/** A stretch of a row's line: coloured as the syntax has it, and `hit` where the match is. */
export interface CodeRun {
  text: string;
  className?: string;
  hit: boolean;
}

/** The row's line cut where its syntax colour changes or the match starts or ends. */
export function codeRuns(row: Pick<LocationRow, "text" | "highlights" | "matchStart" | "matchEnd">): CodeRun[] {
  const { text, matchStart, matchEnd } = row;
  const spans: { from: number; to: number; className?: string }[] = [];
  let at = 0;
  eachRange(decodeHighlights(row.highlights), 0, text.length, (from, to, className) => {
    if (from > at) spans.push({ from: at, to: from });
    spans.push({ from, to, className });
    at = to;
  });
  if (at < text.length) spans.push({ from: at, to: text.length });

  const runs: CodeRun[] = [];
  for (const span of spans) {
    const cuts = [span.from, ...[matchStart, matchEnd].filter((c) => c > span.from && c < span.to), span.to];
    for (let i = 1; i < cuts.length; i++) {
      const [from, to] = [cuts[i - 1], cuts[i]];
      runs.push({
        text: text.slice(from, to),
        className: span.className,
        hit: matchStart < matchEnd && from >= matchStart && to <= matchEnd,
      });
    }
  }
  return runs;
}

/** What separates a container from its member in the language of the file at `path`. */
const separatorOf = (path: string): string => (/\.(rs|c|h|cc|cpp|cxx|hpp)$/i.test(path) ? "::" : ".");

/** The type a Rust `impl` block is for (`impl Backoff for Retry` is `Retry`); any other name as it is. */
const containerName = (container: string): string =>
  /^impl\b/.test(container) ? container.replace(/^impl(<[^>]*>)?\s+/, "").replace(/^.* for /, "") : container;

/**
 * What a place is in, short: the innermost symbol around it with the last segment of its container
 * (`Ledger::commit`, `TaskDispatcher.Trigger`), else just the symbol; the file's name where there is
 * no symbol.
 */
export function symbolLabel(row: Pick<LocationRow, "within" | "path">): string {
  const { within, path } = row;
  const { name, context } = rowLabel(within, path);
  if (!within) return name;
  if (within.name.length < 2) return containerName(name);
  const container = containerName(context);
  return `${splitQualified(container).name || container}${separatorOf(within.path)}${name}`;
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
