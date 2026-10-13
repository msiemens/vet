// How a list answers the mouse and the keyboard, the same in every list: a single click and Space
// do the item's own action, a double click and Enter "open" it, and Up and Down only move the
// highlight. A single click waits for the double-click time, so a double click never does the
// single action first. A list whose items have one action has no double click: a click does it.

import { tick } from "svelte";
import { LIST, listKey, moveHighlight } from "../listKeys.ts";
import { ClickGate } from "./doubleClick.ts";

export interface ListNavOptions<T> {
  /** The items in order, as listed now. */
  items: () => readonly T[];
  /** The item's own action. */
  single: (item: T) => void;
  /** Opens the item. Without it `single` is all there is: a click and Enter do it at once. */
  double?: (item: T) => void;
  /** The highlighted item's index, -1 for none; the list owns it, so it can draw it. */
  highlight: { get: () => number; set: (index: number) => void };
  /** Brings the item at `index` into view. */
  reveal?: (index: number) => void;
  /** The element that takes the keys and has the focus while the list does; a click moves the focus to it. */
  element?: () => HTMLElement | undefined;
  /** The double-click time; the system's by default. */
  ms?: () => number;
  /** The keys come from a text field beside the list: Space, Home and End stay the field's. */
  field?: boolean;
  /** The item the highlight starts on when the list takes the focus with none highlighted, -1 for none. */
  start?: () => number;
}

export class ListNav<T> {
  private options: ListNavOptions<T>;
  private gate: ClickGate<T>;

  constructor(options: ListNavOptions<T>) {
    this.options = options;
    this.gate = new ClickGate(options.single, options.double ?? options.single, options.ms);
  }

  /** The attributes of the element that takes the keys: one stop in the tab order, and a list for the commands. */
  readonly attributes = { [LIST]: "", tabindex: 0 };

  /** A click on the item at `index`; the highlight goes there and the list takes the focus, as a row does not always. */
  click(e: Pick<MouseEvent, "detail">, index: number) {
    const item = this.options.items()[index];
    this.options.highlight.set(index);
    this.options.element?.()?.focus({ preventScroll: true });
    if (item === undefined) return;
    if (!this.options.double) {
      if (e.detail < 2) this.options.single(item);
    } else this.gate.click(e, item);
  }

  /** A key in the list; answers whether it was the list's, and then the caller takes the event. */
  keydown(e: Parameters<typeof listKey>[0]): boolean {
    const key = listKey(e, this.options.field);
    if (!key) return false;
    const { items, highlight, reveal } = this.options;
    if (key !== "single" && key !== "double") {
      const to = moveHighlight(highlight.get(), key, items().length);
      highlight.set(to);
      reveal?.(to);
      return true;
    }
    const item = items()[highlight.get()];
    if (item !== undefined) this.gate.press(key, item);
    return true;
  }

  /** The pointer is over the item at `index`: it takes the highlight, if the pointer moved. A list that scrolls under a pointer at rest makes the browser repeat the pointer's last move with no movement. */
  hover(e: Pick<MouseEvent, "movementX" | "movementY">, index: number) {
    if (e.movementX !== 0 || e.movementY !== 0) this.options.highlight.set(index);
  }

  /** The list took the focus: with nothing highlighted the highlight starts where the list says. */
  focusin() {
    const { highlight, start } = this.options;
    if (highlight.get() < 0 && start) highlight.set(start());
  }

  /** Drops a single click that is still waiting. */
  cancel() {
    this.gate.cancel();
  }
}

/** Scrolls the highlighted row (`.cur`) of `list` into view once the rows have been drawn. */
export async function revealCurrent(list: HTMLElement | undefined): Promise<void> {
  await tick();
  list?.querySelector(".cur")?.scrollIntoView({ block: "nearest" });
}
