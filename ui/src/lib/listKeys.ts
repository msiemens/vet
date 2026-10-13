// The keys of a list, the same in every list: the arrows, the Page keys, Home and End move the
// highlight and do nothing else, Space does what a single click does and Enter what a double click does.

/**
 * The attribute of the element that has the focus while a list has it. The command registry leaves
 * the keys of commands that yield to lists (a Space, say) to the list.
 */
export const LIST = "data-list";

/** How many rows the Page keys move. */
export const PAGE = 8;

export type MoveKey = "up" | "down" | "pageUp" | "pageDown" | "first" | "last";
export type ListKey = MoveKey | "single" | "double";

/** A keydown as the list reads it. */
export interface ListKeyEvent {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** A key with no modifier: the only kind a list, a tree or a popup reads for itself. */
export function plainKey(e: Pick<ListKeyEvent, "ctrlKey" | "altKey" | "shiftKey" | "metaKey">): boolean {
  return !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
}

/**
 * What the key does in a list, if it does anything. A key with a modifier belongs to the commands.
 * With `field` the keys come from a text field the list sits beside, where Space and Home and End
 * belong to the text.
 */
export function listKey(e: ListKeyEvent, field = false): ListKey | undefined {
  if (!plainKey(e)) return undefined;
  switch (e.key) {
    case "ArrowUp":
      return "up";
    case "ArrowDown":
      return "down";
    case "PageUp":
      return "pageUp";
    case "PageDown":
      return "pageDown";
    case "Home":
      return field ? undefined : "first";
    case "End":
      return field ? undefined : "last";
    case " ":
      return field ? undefined : "single";
    case "Enter":
      return "double";
    default:
      return undefined;
  }
}

const STEP: Record<"up" | "down" | "pageUp" | "pageDown", number> = { up: -1, down: 1, pageUp: -PAGE, pageDown: PAGE };

/** The index the highlight moves to from `index` in a list of `count` items: it stops at both ends, and starts at the first or last. */
export function moveHighlight(index: number, key: MoveKey, count: number): number {
  if (count <= 0) return -1;
  if (key === "first") return 0;
  if (key === "last") return count - 1;
  if (index < 0) return key === "down" || key === "pageDown" ? 0 : count - 1;
  return Math.min(count - 1, Math.max(0, index + STEP[key]));
}
