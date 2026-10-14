// What a result set lists: the places that pass the filters, grouped by file, as the flat list of
// rows the panel draws.

import { compareText } from "../lib/format.ts";
import type { FoundFile, Place } from "./foundPlace.ts";

export interface Filters {
  /** Typed in the tool: the places whose symbol, line or path contain it. */
  query: string;
  /** Only places in files the changeset changes. */
  changeset: boolean;
  /** Only places in hunks not seen (an old base counts as not seen). */
  notSeen: boolean;
}

export const NO_FILTERS: Filters = { query: "", changeset: false, notSeen: false };

export const narrowing = (f: Filters): boolean => f.query.trim() !== "" || f.changeset || f.notSeen;

export function visible(places: readonly Place[], f: Filters): Place[] {
  const query = f.query.trim().toLowerCase();
  return places.filter((p) => {
    if (f.changeset && p.file.status === undefined) return false;
    if (f.notSeen && p.review !== "unseen" && p.review !== "old") return false;
    return !query || [p.within ?? "", p.text, p.file.path].some((text) => text.toLowerCase().includes(query));
  });
}

/** A row of the list: a file's header, and under it its places. */
export type Item =
  | { kind: "file"; depth: 0; key: string; file: FoundFile; count: number; notSeen: number; open: boolean }
  | { kind: "place"; depth: 1; key: string; place: Place };

const notSeen = (p: Place) => p.review === "unseen" || p.review === "old";

/**
 * The files with their places: the files with a place on a changed line first, libraries last, each
 * group by path; a file's places in line order. A file in `folded` lists its header alone.
 */
export function itemsOf(places: readonly Place[], folded: ReadonlySet<string>): Item[] {
  const byFile = new Map<string, Place[]>();
  for (const p of places) byFile.set(p.file.path, [...(byFile.get(p.file.path) ?? []), p]);
  const tier = (inFile: Place[]) => (inFile[0].file.external ? 2 : inFile.some((p) => p.changed) ? 0 : 1);
  const files = [...byFile.values()].sort((a, b) => tier(a) - tier(b) || compareText(a[0].file.path, b[0].file.path));
  return files.flatMap((inFile) => {
    const { file } = inFile[0];
    const key = `f:${file.path}`;
    const open = !folded.has(key);
    const header: Item = {
      kind: "file",
      depth: 0,
      key,
      file,
      count: inFile.length,
      notSeen: inFile.filter(notSeen).length,
      open,
    };
    if (!open) return [header];
    const ordered = [...inFile].sort((a, b) => a.line - b.line || a.col - b.col);
    return [header, ...ordered.map((place): Item => ({ kind: "place", depth: 1, key: place.id, place }))];
  });
}
