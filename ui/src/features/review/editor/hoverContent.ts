// Language-server text as DOM, shared by every popup that shows it (the editor's hover tooltip,
// the deleted lines' floating one) and the symbol panel's signature. Code is coloured from the
// highlights the backend sent with the text, so it takes the same theme tokens as the editors.
import type { HoverText } from "../../../state/backend";
import { highlightedHtml, renderMarkdown } from "./hoverMarkdown.ts";

/** The popup body for Markdown hover text. */
export function hoverContent(hover: HoverText): HTMLElement {
  const dom = document.createElement("div");
  dom.className = "vet-md";
  dom.innerHTML = renderMarkdown(hover.text, hover.code);
  return dom;
}

/** Code and its highlights; `highlights` is empty for plain text. */
export interface Snippet {
  code: string;
  highlights: readonly number[];
}

/** Svelte action: fills the node with the coloured `snippet`. */
export function codeBlock(node: HTMLElement, snippet: Snippet) {
  const fill = ({ code, highlights }: Snippet) => (node.innerHTML = highlightedHtml(code, highlights));
  fill(snippet);
  return { update: fill };
}
