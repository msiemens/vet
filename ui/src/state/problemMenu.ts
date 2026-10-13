// The Problems panel's Group and filter menu: what it offers and what a choice changes.

import type { MenuItem } from "../lib/ui/menuItem.ts";
import type { Filters, Grouping } from "./problemRows.ts";

const GROUPINGS: { id: Grouping; label: string }[] = [
  { id: "origin", label: "Origin, then file" },
  { id: "file", label: "File" },
  { id: "severity", label: "Severity" },
];

const GROUP = "group:";
const UNSEEN = "only:unseen";
const CHANGED = "only:changed";

/** Grouping by origin needs the origins to be known (`settled`). */
export function groupMenu(grouping: Grouping, filters: Pick<Filters, "unseenOnly" | "changedOnly">, settled: boolean) {
  const items: MenuItem[] = [
    { id: "heading:group", label: "Group by", heading: true },
    ...GROUPINGS.map(({ id, label }) => ({
      id: GROUP + id,
      label,
      checked: grouping === id,
      radio: true,
      enabled: id !== "origin" || settled,
    })),
    { id: "heading:only", label: "Show only", heading: true },
    { id: UNSEEN, label: "Problems in hunks not seen", checked: filters.unseenOnly },
    { id: CHANGED, label: "Problems on changed lines", checked: filters.changedOnly },
  ];
  return items;
}

/** Whether the menu's settings differ from the panel's defaults. */
export const menuChanged = (grouping: Grouping, f: Pick<Filters, "unseenOnly" | "changedOnly">) =>
  f.unseenOnly || f.changedOnly || grouping !== "origin";

/** What choosing `id` does to the grouping and the two filters. */
export function chooseGroupItem(
  id: string,
  grouping: Grouping,
  f: Pick<Filters, "unseenOnly" | "changedOnly">,
): { grouping: Grouping; unseenOnly: boolean; changedOnly: boolean } {
  const next = { grouping, unseenOnly: f.unseenOnly, changedOnly: f.changedOnly };
  if (id.startsWith(GROUP)) next.grouping = id.slice(GROUP.length) as Grouping;
  else if (id === UNSEEN) next.unseenOnly = !f.unseenOnly;
  else if (id === CHANGED) next.changedOnly = !f.changedOnly;
  return next;
}
