// The state of one list's speed search: the typed query, and the keys that edit it. The list owns
// it, filters what it shows by `query` and renders a `SpeedSearchField` while it is active.

import { tick } from "svelte";
import { SEARCHING, speedSearchKey } from "../speedSearch";

export class SpeedSearch {
  query = $state("");

  get active(): boolean {
    return this.query !== "";
  }

  /** Spread on the element that has the focus while searching, so command keys leave typing to the search. */
  get attributes(): Record<string, string | undefined> {
    return { [SEARCHING]: this.active ? "" : undefined };
  }

  clear() {
    this.query = "";
  }

  /** A keydown that types, deletes or clears; answers whether it was the search's, and then takes the event. */
  type(e: KeyboardEvent): boolean {
    const next = speedSearchKey(this.query, e);
    if (next === undefined) return false;
    this.query = next;
    e.preventDefault();
    e.stopPropagation();
    return true;
  }

  /**
   * Typing, and Up and Down between the focusable `items` of `list` while searching, for a list that
   * lists only its matches. A list whose focused item the filter removed focuses its first match.
   */
  keydown(e: KeyboardEvent, list: HTMLElement | undefined, items: string) {
    if (!list) return;
    if (this.active && (e.key === "ArrowDown" || e.key === "ArrowUp") && !e.altKey && !e.ctrlKey && !e.metaKey) {
      const all = [...list.querySelectorAll<HTMLElement>(items)];
      const at = all.indexOf(document.activeElement as HTMLElement);
      const to = (at + (e.key === "ArrowDown" ? 1 : -1) + all.length) % all.length;
      e.preventDefault();
      e.stopPropagation();
      all[to]?.focus();
      return;
    }
    if (!this.type(e)) return;
    void tick().then(() => {
      if (!list.contains(document.activeElement)) list.querySelector<HTMLElement>(items)?.focus();
    });
  }
}
