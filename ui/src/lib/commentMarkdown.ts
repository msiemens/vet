// A comment's markdown to HTML. Comments come from other people (and a platform), so they are
// untrusted: markdown-it keeps raw HTML off (`html: false`) and escapes it, images are left as text,
// and it refuses `javascript:` and similar link targets. A link's click is the card's to take: the
// webview must not navigate (`threadCard.ts` opens it in the browser).

import MarkdownIt from "markdown-it";

const md = new MarkdownIt({ html: false, linkify: true, breaks: true });
md.disable("image");

/** The body of a comment as HTML that is safe to insert. */
export const renderComment = (text: string): string => md.render(text);

/**
 * The first line a comment says, as plain text: its first block without the markup, for the one-row
 * card of a resolved thread.
 */
export function commentPreview(text: string): string {
  for (const token of md.parse(text, {})) {
    if (token.type === "inline") {
      return (token.children ?? [])
        .map((child) => {
          if (child.type === "text" || child.type === "code_inline") return child.content;
          return child.type === "softbreak" || child.type === "hardbreak" ? " " : "";
        })
        .join("");
    }
    if (token.type === "fence" || token.type === "code_block") return token.content.split("\n")[0];
  }
  return "";
}
