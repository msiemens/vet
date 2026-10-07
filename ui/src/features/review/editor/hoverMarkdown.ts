// Markdown from a language server to HTML. The text comes from an external process, so it is
// untrusted: markdown-it keeps raw HTML off (`html: false`) and escapes it, and a link's text is
// shown without its destination, since the webview must not navigate and nothing opens it externally.
// Code is coloured from the highlights the backend sends for each fenced block, never parsed here.

import MarkdownIt from "markdown-it";
import { decodeHighlights, pieces } from "./highlights.ts";

/** The highlights of each fenced code block of a text, in order. */
type CodeHighlights = readonly (readonly number[])[];

const md = new MarkdownIt({ html: false, linkify: false, breaks: false });
md.disable("image");
md.renderer.rules.link_open = () => "";
md.renderer.rules.link_close = () => "";

/** Escaped HTML for `code`, with the ranges `highlights` colours wrapped in spans. */
export function highlightedHtml(code: string, highlights: readonly number[]): string {
  return pieces(decodeHighlights(highlights), 0, code)
    .map((p) =>
      typeof p === "string"
        ? md.utils.escapeHtml(p)
        : `<span class="${p.className}">${md.utils.escapeHtml(p.text)}</span>`,
    )
    .join("");
}

// The highlights travel in the render environment, so the one parser serves every caller; the
// n-th block drawn takes the n-th entry.
interface Fences {
  code: CodeHighlights;
  next: number;
}

md.renderer.rules.fence = (tokens, i, _options, env) => {
  const state = env as unknown as Fences;
  const highlights = state.code[state.next++] ?? [];
  return `<pre><code>${highlightedHtml(tokens[i].content.replace(/\n$/, ""), highlights)}</code></pre>\n`;
};

export const renderMarkdown = (text: string, code: CodeHighlights): string => md.render(text, { code, next: 0 });

/** The fenced code blocks of a text with their highlights, in order. */
export function fencedBlocks(text: string, code: CodeHighlights): { code: string; highlights: readonly number[] }[] {
  return md
    .parse(text, {})
    .filter((token) => token.type === "fence")
    .map((token, i) => ({ code: token.content.replace(/\n$/, ""), highlights: code[i] ?? [] }));
}
