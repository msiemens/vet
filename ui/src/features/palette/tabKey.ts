import type { KeyLike } from "../../lib/keymap";

/**
 * Which way a key press changes the popup's tab: 1 for the next, -1 for the previous, 0 for none.
 * Tab and Ctrl+Tab (with Shift, backwards) do it; Ctrl is Ctrl on every platform, as in JetBrains.
 */
export function tabStep(e: KeyLike): 1 | -1 | 0 {
  if (e.key !== "Tab" || e.altKey || e.metaKey) return 0;
  return e.shiftKey ? -1 : 1;
}
