// The Keymap page's rows: every command with its key, grouped by area and narrowed by a search.

import type { CommandListing } from "../../lib/commands.ts";
import { compareText } from "../../lib/format.ts";
import { canonicalSpec } from "../../lib/keymap.ts";
import type { Overrides } from "../../lib/keymapEdit.ts";

export interface KeymapRow extends CommandListing {
  /** The viewer changed its key. */
  changed: boolean;
  /** Other commands that have the same key. */
  sharedWith: string[];
}

export interface KeymapGroup {
  area: string;
  label: string;
  rows: KeymapRow[];
}

/** A line of the Keymap table: an area's own line, or one of its commands. */
export type KeymapEntry =
  { kind: "group"; area: string; label: string; count: number; open: boolean } | { kind: "row"; row: KeymapRow };

/**
 * The table's lines top down: each group's line, and under an open one its commands. A search opens
 * every group it found something in, whatever the viewer folded.
 */
export function keymapEntries(
  groups: readonly KeymapGroup[],
  folded: ReadonlySet<string>,
  searching: boolean,
): KeymapEntry[] {
  return groups.flatMap(({ area, label, rows }): KeymapEntry[] => {
    const open = searching || !folded.has(area);
    return [
      { kind: "group", area, label, count: rows.length, open },
      ...(open ? rows.map((row): KeymapEntry => ({ kind: "row", row })) : []),
    ];
  });
}

/** The areas in the order the page shows them; others follow alphabetically. */
const AREAS: [area: string, label: string][] = [
  ["file", "File"],
  ["edit", "Edit"],
  ["view", "View"],
  ["review", "Review"],
  ["path", "Files and folders"],
  ["tab", "Tabs"],
  ["thread", "Comments"],
  ["app", "Application"],
];

/** The area of a command: its id up to the first dot. */
export const areaOf = (id: string): string => id.split(".")[0];

const areaLabel = (area: string) =>
  AREAS.find(([a]) => a === area)?.[1] ?? area.charAt(0).toUpperCase() + area.slice(1);

export function keymapGroups(listing: CommandListing[], overrides: Overrides, query: string): KeymapGroup[] {
  const owners = new Map<string, string[]>();
  for (const { id, binding, alsoBindings } of listing) {
    for (const bound of [binding, ...alsoBindings]) {
      if (!bound) continue;
      const key = canonicalSpec(bound.spec);
      owners.set(key, [...(owners.get(key) ?? []), id]);
    }
  }
  const label = new Map(listing.map((c) => [c.id, c.label]));
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);

  const groups = new Map<string, KeymapRow[]>();
  for (const command of listing) {
    const area = areaOf(command.id);
    const keys = [command.binding, ...command.alsoBindings].filter((b) => b !== undefined);
    const haystack = [command.label, command.id, areaLabel(area), ...keys.map((b) => b.label)].join(" ").toLowerCase();
    if (!words.every((w) => haystack.includes(w))) continue;
    const shared = keys.flatMap((b) => owners.get(canonicalSpec(b.spec)) ?? []);
    const row: KeymapRow = {
      ...command,
      changed: command.id in overrides,
      sharedWith: [...new Set(shared.filter((id) => id !== command.id))].map((id) => label.get(id) ?? id),
    };
    groups.set(area, [...(groups.get(area) ?? []), row]);
  }

  const rank = (area: string) => {
    const at = AREAS.findIndex(([a]) => a === area);
    return at < 0 ? AREAS.length : at;
  };
  return [...groups]
    .sort(([a], [b]) => rank(a) - rank(b) || compareText(a, b))
    .map(([area, rows]) => ({
      area,
      label: areaLabel(area),
      rows: rows.sort((a, b) => compareText(a.label, b.label)),
    }));
}
