// A short time in an element that is built by hand (the thread cards), which is not redrawn by a
// component: the element keeps what it shows, and every minute the ones in the document are rewritten.

import { onMinute } from "./minute.ts";
import { type TimeStyle, shortTime } from "./shortTime.ts";

interface TimeElement {
  dataset: Record<string, string | undefined>;
  textContent: string | null;
}

/** Sets `element` to the short time of `unixSeconds` as of `nowMs`, and remembers it for `refreshTimes`. */
export function showTime(element: TimeElement, unixSeconds: number, style: TimeStyle, nowMs: number) {
  element.dataset.timeAt = String(unixSeconds);
  element.dataset.timeStyle = style;
  element.textContent = shortTime(unixSeconds, nowMs, style);
}

/** Writes the short times under `root` again as of `nowMs`. */
export function refreshTimes(root: { querySelectorAll(selector: string): Iterable<TimeElement> }, nowMs: number) {
  for (const element of root.querySelectorAll("[data-time-at]")) {
    const { timeAt, timeStyle } = element.dataset;
    if (timeAt !== undefined && timeStyle !== undefined)
      showTime(element, Number(timeAt), timeStyle as TimeStyle, nowMs);
  }
}

let listening = false;

/** `element` shows the short time of `unixSeconds` and keeps it right as the minutes pass. */
export function liveTime(element: HTMLElement, unixSeconds: number, style: TimeStyle) {
  showTime(element, unixSeconds, style, Date.now());
  if (listening) return;
  listening = true;
  onMinute(() => refreshTimes(document, Date.now()));
}
