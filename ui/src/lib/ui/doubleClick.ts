import { Debounced } from "../debounced.ts";

/** What a plain browser, which cannot tell the system's setting, takes for the double-click time. */
export const FALLBACK_DOUBLE_CLICK_MS = 500;

let systemMs = FALLBACK_DOUBLE_CLICK_MS;

/** Sets the system's double-click time, as the host reads it from the mouse settings. */
export function setDoubleClickMs(ms: number) {
  if (Number.isFinite(ms) && ms > 0) systemMs = ms;
}

/** How long a second click may follow the first and still make a double click. */
export const doubleClickMs = () => systemMs;

/**
 * Tells a single click from a double click. A mouse click that is not followed by a second one
 * within the double-click time runs `single`; the second click of a double click cancels it and
 * runs `double` instead, so a double click never runs `single`. A click from the keyboard (`detail`
 * 0) runs `single` at once. Clicks on different subjects share one gate: the latest click wins.
 */
export class ClickGate<T> {
  private timer = new Debounced();
  private single: (subject: T) => void;
  private double: (subject: T) => void;
  private ms: () => number;

  constructor(single: (subject: T) => void, double: (subject: T) => void, ms: () => number = doubleClickMs) {
    this.single = single;
    this.double = double;
    this.ms = ms;
  }

  click(e: Pick<MouseEvent, "detail">, subject: T) {
    if (e.detail === 0) this.press("single", subject);
    else if (e.detail === 2) this.press("double", subject);
    else {
      this.timer.schedule(this.ms(), () => this.single(subject));
    }
  }

  /** A key does the action at once, and drops a single click that is still waiting. */
  press(action: "single" | "double", subject: T) {
    this.cancel();
    if (action === "single") this.single(subject);
    else this.double(subject);
  }

  /** Drops a single click that is still waiting. */
  cancel() {
    this.timer.cancel();
  }
}
