// Markdown from a language server to HTML. The text comes from an external process, so it is
// untrusted: markdown-it keeps raw HTML off (`html: false`) and escapes it, and a link's text is
// shown without its destination, since the webview must not navigate and nothing opens it externally.

import MarkdownIt from "markdown-it";

/** Highlighted, already escaped HTML for the inside of a code block, or undefined for plain text. */
export type HighlightCode = (code: string, info: string) => string | undefined;

const md = new MarkdownIt({ html: false, linkify: false, breaks: false });
md.disable("image");
md.renderer.rules.link_open = () => "";
md.renderer.rules.link_close = () => "";

// The highlighter travels in the render environment, so the one parser serves every caller.
md.renderer.rules.fence = (tokens, i, _options, env) => {
  const { content, info } = tokens[i];
  const highlight = (env as { highlight?: HighlightCode } | undefined)?.highlight;
  return `<pre><code>${highlight?.(content, info) ?? md.utils.escapeHtml(content)}</code></pre>\n`;
};

export const renderMarkdown = (text: string, highlight: HighlightCode): string => md.render(text, { highlight });
