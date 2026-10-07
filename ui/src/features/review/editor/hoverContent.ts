// Language-server text as DOM, shared by every popup that shows it (the editor's hover tooltip,
// the deleted lines' floating one) and the symbol panel's signature. Code is coloured from the
// highlights the backend sent with the text, so it takes the same theme tokens as the editors.

import type { HoverText } from "../../../state/backend";
import { fencedBlocks, highlightedHtml, renderMarkdown } from "./hoverMarkdown.ts";

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

/**
 * The definition out of a hover: servers put the container (a module path) in a code block of
 * its own before the signature, and prose after it, so it is the last block that is not empty.
 * Text without a block is shown whole, uncoloured.
 */
export function signatureOf(hover: HoverText): Snippet {
  const blocks = fencedBlocks(hover.text, hover.code).filter((block) => block.code.trim());
  return blocks.at(-1) ?? { code: hover.text.trim(), highlights: [] };
}

/** Svelte action: fills the node with the coloured `snippet`. */
export function codeBlock(node: HTMLElement, snippet: Snippet) {
  const fill = ({ code, highlights }: Snippet) => (node.innerHTML = highlightedHtml(code, highlights));
  fill(snippet);
  return { update: fill };
}
