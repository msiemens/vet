// What the window reports to the backend's log: uncaught errors, unhandled rejections and failed
// requests, a few at a time. The backend writes them to the log file, which the webview has none of.

import type { LogLevel } from "../generated/LogLevel";
import { isAbort } from "../lib/abort.ts";
import { errorText } from "../lib/errorText.ts";

export type Report = (level: LogLevel, message: string, context: string) => void;

/** At most `max` entries per `windowMs`: an error in a render loop must not fill the log. */
export class LogBudget {
  private since = Number.NEGATIVE_INFINITY;
  private used = 0;
  private max: number;
  private windowMs: number;

  constructor(max = 10, windowMs = 10_000) {
    this.max = max;
    this.windowMs = windowMs;
  }

  /** Whether an entry at `now` (milliseconds) may be sent. */
  admit(now: number): boolean {
    if (now - this.since >= this.windowMs) {
      this.since = now;
      this.used = 0;
    }
    if (this.used >= this.max) return false;
    this.used++;
    return true;
  }
}

/** The most of a stack that is sent. */
const STACK_CHARS = 1500;

/** An error with the top of its stack, which says where it came from. */
function describe(error: unknown): string {
  const stack = error instanceof Error ? error.stack : undefined;
  return stack ? stack.slice(0, STACK_CHARS) : errorText(error);
}

/** The script position an `error` event names, if any. */
function position({ filename, lineno, colno }: ErrorEvent): string {
  return filename ? ` at ${filename}:${String(lineno)}:${String(colno)}` : "";
}

/**
 * Reports what `target` (the window) sees go uncaught. A request given up on is no error. Returns
 * the function that stops reporting.
 */
export function reportUncaught(report: Report, target: EventTarget = window): () => void {
  const onError = (event: Event) => {
    const error: unknown = (event as ErrorEvent).error;
    // Without an error object (null for a ResizeObserver loop) only the message says what happened.
    if (error !== undefined && error !== null) {
      report("error", describe(error), "uncaught error");
      return;
    }
    const message = (event as ErrorEvent).message || "an error event without a message";
    // The browser delivers the remaining notifications in the next frame: nothing is lost.
    const level = message.startsWith("ResizeObserver loop") ? "warn" : "error";
    report(level, message + position(event as ErrorEvent), "uncaught error");
  };
  const onRejection = (event: Event) => {
    const reason: unknown = (event as PromiseRejectionEvent).reason;
    if (isAbort(reason)) return;
    // A promise rejected with nothing carries no stack: say so instead of printing "null".
    const message =
      reason === undefined || reason === null ? `a promise rejected with ${String(reason)}` : describe(reason);
    report("error", message, "unhandled rejection");
  };
  target.addEventListener("error", onError);
  target.addEventListener("unhandledrejection", onRejection);
  return () => {
    target.removeEventListener("error", onError);
    target.removeEventListener("unhandledrejection", onRejection);
  };
}

/** Reports a request that failed, unless it was given up on. Only its kind is named: its content stays out of the log. */
export function reportFailedRequest(report: Report, kind: string, error: unknown): void {
  if (!isAbort(error)) report("warn", errorText(error), `request ${kind}`);
}
