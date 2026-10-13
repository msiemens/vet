// Shift tapped twice in quick succession, with nothing in between: Search Everywhere's key. Shift held
// while another key goes down (a capital letter, a selection by arrow) is a modifier and never counts,
// which is also why typing cannot trigger it.

interface ShiftKeyEvent {
  key: string;
  repeat?: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  isComposing?: boolean;
}

/** How long after the first tap the second must come up, in milliseconds. */
export const DOUBLE_TAP_MS = 350;

export class ShiftTap {
  private pending = false;
  private last?: number;

  /** Feed every keydown. */
  keydown(e: ShiftKeyEvent) {
    if (e.key !== "Shift") {
      this.cancel();
      return;
    }
    if (e.repeat) return;
    this.pending = !(e.ctrlKey || e.altKey || e.metaKey || e.isComposing);
    if (!this.pending) this.last = undefined;
  }

  /** Feed every keyup with the time in ms; answers whether this is the second tap. */
  keyup(e: Pick<ShiftKeyEvent, "key">, now: number): boolean {
    if (e.key !== "Shift" || !this.pending) return false;
    this.pending = false;
    if (this.last !== undefined && now - this.last <= DOUBLE_TAP_MS) {
      this.last = undefined;
      return true;
    }
    this.last = now;
    return false;
  }

  /** What was tapped no longer counts: the pointer was used, or the window lost the focus. */
  cancel() {
    this.pending = false;
    this.last = undefined;
  }
}
