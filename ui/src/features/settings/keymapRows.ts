// The Keymap page's rows: every command with its key, grouped by area and narrowed by a search.

import type { CommandListing } from "../../lib/commands.ts";
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
  ["dev", "Developer"],
];

/** The area of a command: its id up to the first dot. */
export const areaOf = (id: string): string => id.split(".")[0];

const areaLabel = (area: string) =>
  AREAS.find(([a]) => a === area)?.[1] ?? area.charAt(0).toUpperCase() + area.slice(1);

export function keymapGroups(listing: CommandListing[], overrides: Overrides, query: string): KeymapGroup[] {
  const owners = new Map<string, string[]>();
  for (const { id, binding } of listing) {
    if (!binding) continue;
    const key = canonicalSpec(binding.spec);
    owners.set(key, [...(owners.get(key) ?? []), id]);
  }
  const label = new Map(listing.map((c) => [c.id, c.label]));
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);

  const groups = new Map<string, KeymapRow[]>();
  for (const command of listing) {
    const area = areaOf(command.id);
    const haystack = [command.label, command.id, areaLabel(area), command.binding?.label ?? ""].join(" ").toLowerCase();
    if (!words.every((w) => haystack.includes(w))) continue;
    const shared = command.binding ? (owners.get(canonicalSpec(command.binding.spec)) ?? []) : [];
    const row: KeymapRow = {
      ...command,
      changed: command.id in overrides,
      sharedWith: shared.filter((id) => id !== command.id).map((id) => label.get(id) ?? id),
    };
    groups.set(area, [...(groups.get(area) ?? []), row]);
  }

  const rank = (area: string) => {
    const at = AREAS.findIndex(([a]) => a === area);
    return at < 0 ? AREAS.length : at;
  };
  return [...groups]
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([area, rows]) => ({
      area,
      label: areaLabel(area),
      rows: rows.sort((a, b) => a.label.localeCompare(b.label)),
    }));
}
