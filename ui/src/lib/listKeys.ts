// The keys of a list, the same in every list: Up and Down move the highlight and do nothing else,
// Space does what a single click does and Enter what a double click does.

/**
 * The attribute of the element that has the focus while a list has it. The command registry leaves
 * the keys of commands that yield to lists (a Space, say) to the list.
 */
export const LIST = "data-list";

export type ListKey = "up" | "down" | "single" | "double";

/** A keydown as the list reads it. */
export interface ListKeyEvent {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** What the key does in a list, if it does anything. A key with a modifier belongs to the commands. */
export function listKey(e: ListKeyEvent): ListKey | undefined {
  if (e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return undefined;
  switch (e.key) {
    case "ArrowUp":
      return "up";
    case "ArrowDown":
      return "down";
    case " ":
      return "single";
    case "Enter":
      return "double";
    default:
      return undefined;
  }
}

/** The index the highlight moves to from `index` in a list of `count` items: it stops at both ends, and starts at the first or last. */
export function moveHighlight(index: number, key: "up" | "down", count: number): number {
  if (count <= 0) return -1;
  if (index < 0) return key === "down" ? 0 : count - 1;
  return Math.min(count - 1, Math.max(0, index + (key === "down" ? 1 : -1)));
}
