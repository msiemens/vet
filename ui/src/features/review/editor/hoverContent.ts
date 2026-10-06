// Language-server text as DOM, shared by every popup that shows it (the editor's hover tooltip,
// the deleted lines' floating one) and the symbol panel's signature. Code is highlighted with the
// editor's own parsers and highlight style, so it takes the same theme tokens.

import { highlightCode } from "@lezer/highlight";
import { StyleModule } from "style-mod";
import { fenceExtension } from "./fenceLanguage";
import { renderMarkdown } from "./hoverMarkdown";
import { highlightStyle, languageOf } from "./language";

/** Code as highlighted nodes; `ext` is a file name or extension, plain text if unknown. */
export function highlightedNodes(code: string, ext: string | undefined): DocumentFragment {
  const out = document.createDocumentFragment();
  const lang = ext === undefined ? undefined : languageOf(ext);
  if (!lang) {
    out.append(code);
    return out;
  }
  // The editors mount the style when they are created; a popup outside one needs it as well.
  if (highlightStyle.module) StyleModule.mount(document, highlightStyle.module);
  const put = (text: string, classes: string) => {
    if (!classes) {
      out.append(text);
      return;
    }
    const span = document.createElement("span");
    span.className = classes;
    span.textContent = text;
    out.append(span);
  };
  highlightCode(code, lang.parser.parse(code), highlightStyle, put, () => out.append("\n"));
  return out;
}

/** The popup body for Markdown hover text of the file at `path`. */
export function hoverContent(markdown: string, path: string): HTMLElement {
  const dom = document.createElement("div");
  dom.className = "vet-md";
  dom.innerHTML = renderMarkdown(markdown, (code, info) => {
    const holder = document.createElement("div");
    holder.append(highlightedNodes(code.replace(/\n$/, ""), fenceExtension(info, path)));
    return holder.innerHTML;
  });
  return dom;
}

/** Svelte action: fills the node with highlighted `code` of the file at `path`. */
export function codeBlock(node: HTMLElement, arg: { code: string; path: string }) {
  const fill = ({ code, path }: typeof arg) => node.replaceChildren(highlightedNodes(code, path));
  fill(arg);
  return { update: fill };
}
