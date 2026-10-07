// Language-server text as DOM, shared by every popup that shows it (the editor's hover tooltip,
// the deleted lines' floating one). Code is coloured from the highlights the backend sent with
// the text, so it takes the same theme tokens as the editors.
import { renderMarkdown } from "../../../lib/hoverMarkdown";
import type { HoverText } from "../../../state/backend";

/** The popup body for Markdown hover text. */
export function hoverContent(hover: HoverText): HTMLElement {
  const dom = document.createElement("div");
  dom.className = "vet-md";
  dom.innerHTML = renderMarkdown(hover.text, hover.code);
  return dom;
}
