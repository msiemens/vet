// Types of cdp.mjs for the TypeScript that imports it.

export interface Tab {
  send(method: string, params?: Record<string, unknown>): Promise<unknown>;
  /** What the page logged. */
  console: { type: string; text: string }[];
  /** The exceptions the page threw, as text. */
  exceptions: string[];
  /** Evaluates an expression in the page (a promise is awaited) and returns its JSON value. */
  evaluate(expression: string): Promise<unknown>;
  navigate(url: string): Promise<void>;
  screenshot(
    path: string,
    clip?: { x: number; y: number; width: number; height: number },
  ): Promise<void>;
  /** type: mouseMoved | mousePressed | mouseReleased | mouseWheel */
  mouse(
    type: string,
    x: number,
    y: number,
    extra?: Record<string, unknown>,
  ): Promise<unknown>;
}

export function withTab<T>(
  fn: (tab: Tab) => Promise<T>,
  options?: { width?: number; height?: number; port?: number },
): Promise<T>;
