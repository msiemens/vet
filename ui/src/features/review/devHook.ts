// What a test driving the app in a browser reads of the review screen. Only the dev server has it.

import type { Rendered } from "./editor/rendered";

declare global {
  interface Window {
    __vet?: { rendered(): Rendered };
  }
}

/** Offers `rendered` as `window.__vet.rendered()` in the dev build; returns the function that takes it away. */
export function exposeRendered(rendered: () => Rendered): () => void {
  if (!import.meta.env.DEV) return () => undefined;
  window.__vet = { rendered };
  return () => delete window.__vet;
}
