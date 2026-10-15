// Plain DOM builders for the editor's own widgets (headers, hunk bars, cards), which are not Svelte.
// The generic `el` and `iconButton` are shared with the comment cards (`lib/ui/dom.ts`).

import type { EditorView } from "@codemirror/view";
import { el } from "../../../lib/ui/dom.ts";

export { el, iconButton } from "../../../lib/ui/dom.ts";

/**
 * Whether the editor takes the keys: its content is the document's active element. Unlike
 * `EditorView.hasFocus`, this does not also ask whether the window has the focus.
 */
export const takesKeys = (view: EditorView): boolean => view.contentDOM === document.activeElement;

/** The text button that marks what was seen against an old base seen again. */
export function reconfirmButton(label: string, onclick: () => void): HTMLButtonElement {
  const button = el("button", "ctl vet-reconfirm vet-review-action", "Re-confirm");
  button.type = "button";
  button.title = label;
  button.addEventListener("click", onclick);
  return button;
}
