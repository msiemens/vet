import { highlightedHtml } from "./hoverMarkdown";

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
