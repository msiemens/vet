/**
 * How long a single click waits for a second one. The webview does not tell the system's
 * double-click interval, only whether a click is a second one (`detail`), so this is a little
 * under the 500 ms default of macOS and Windows: a slower double click still opens, after the
 * single click's action.
 */
export const DOUBLE_CLICK_MS = 400;

/**
 * Tells a single click from a double click. A mouse click that is not followed by a second one
 * within `ms` runs `single`; the second click of a double click cancels it and runs `double`
 * instead. A click from the keyboard (`detail` 0) runs `single` at once. Clicks on different
 * subjects share one gate: the latest click wins.
 */
export class ClickGate<T> {
  private timer?: ReturnType<typeof setTimeout>;
  private single: (subject: T) => void;
  private double: (subject: T) => void;
  private ms: number;

  constructor(single: (subject: T) => void, double: (subject: T) => void, ms = DOUBLE_CLICK_MS) {
    this.single = single;
    this.double = double;
    this.ms = ms;
  }

  click(e: Pick<MouseEvent, "detail">, subject: T) {
    this.cancel();
    if (e.detail === 0) this.single(subject);
    else if (e.detail === 2) this.double(subject);
    else this.timer = setTimeout(() => this.single(subject), this.ms);
  }

  /** Drops a single click that is still waiting. */
  cancel() {
    clearTimeout(this.timer);
    this.timer = undefined;
  }
}
