import type { Attachment } from "svelte/attachments";

/**
 * A timeout that can be held: `fire` runs `ms` after the last `release` (or the start) unless it is
 * held meanwhile. A notice that closes itself holds its timer while the viewer reads or selects it.
 */
export class HoldTimer {
  private handle: ReturnType<typeof setTimeout> | undefined;
  private done = false;
  private readonly fire: () => void;
  private readonly ms: number;

  constructor(fire: () => void, ms: number) {
    this.fire = fire;
    this.ms = ms;
    this.arm();
  }

  hold() {
    this.disarm();
  }

  release() {
    if (this.done) return;
    this.disarm();
    this.arm();
  }

  /** Ends the timer for good; `fire` does not run. */
  stop() {
    this.done = true;
    this.disarm();
  }

  private arm() {
    this.handle = setTimeout(() => {
      this.done = true;
      this.fire();
    }, this.ms);
  }

  private disarm() {
    clearTimeout(this.handle);
    this.handle = undefined;
  }
}

/**
 * Calls `hold` while the pointer is over the element, a press that began in it is still down, or
 * text in it is selected; `release` once none of these is.
 */
export function holdWhileUsed(hold: () => void, release: () => void): Attachment<HTMLElement> {
  return (element) => {
    let over = false;
    let pressed = false;
    let selected = false;
    let held = false;

    const update = () => {
      const want = over || pressed || selected;
      if (want === held) return;
      held = want;
      if (want) hold();
      else release();
    };
    const syncSelection = () => {
      const selection = document.getSelection();
      selected =
        selection !== null &&
        !selection.isCollapsed &&
        (element.contains(selection.anchorNode) || element.contains(selection.focusNode));
      update();
    };
    const enter = () => {
      over = true;
      update();
    };
    const leave = () => {
      over = false;
      update();
    };
    const press = () => {
      pressed = true;
      update();
    };
    const lift = () => {
      pressed = false;
      update();
    };

    element.addEventListener("pointerenter", enter);
    element.addEventListener("pointerleave", leave);
    element.addEventListener("pointerdown", press);
    window.addEventListener("pointerup", lift);
    window.addEventListener("pointercancel", lift);
    document.addEventListener("selectionchange", syncSelection);
    return () => {
      element.removeEventListener("pointerenter", enter);
      element.removeEventListener("pointerleave", leave);
      element.removeEventListener("pointerdown", press);
      window.removeEventListener("pointerup", lift);
      window.removeEventListener("pointercancel", lift);
      document.removeEventListener("selectionchange", syncSelection);
    };
  };
}
