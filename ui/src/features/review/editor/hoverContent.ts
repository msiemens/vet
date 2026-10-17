// Language-server text as DOM, shared by every popup that shows it (the editor's hover tooltip,
// the deleted lines' floating one). Code is coloured from the highlights the backend sent with
// the text, so it takes the same theme tokens as the editors.
//
// A popup is up to four stacked parts: the diagnostics at the symbol, its signature, its docs
// (the only part that scrolls) and a footer naming the server. An answer that has no text is said
// in one line instead: the server is indexing, or why there is none.
import type { Diagnostic } from "../../../generated/Diagnostic";
import { renderMarkdown } from "../../../lib/hoverMarkdown.ts";

import { iconSvg } from "../../../lib/ui/icons.ts";
import { severitySvg } from "../../../lib/ui/severity.ts";
import type { HoverAnswer } from "./semantic";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function icon(svg: string): HTMLElement {
  const node = el("span", "vet-hover-icon");
  node.innerHTML = svg;
  return node;
}

/** `text` with its backticked spans as code. */
function withCode(text: string): Node[] {
  return text.split(/`([^`]+)`/).map((part, i) => (i % 2 ? el("code", "", part) : document.createTextNode(part)));
}

const ORIGIN_NOTE = {
  new: "new in this change",
  existing: "existing: the base has it too",
  pending: "",
  unknown: "",
} as const;

function diagnosticRow(d: Diagnostic): HTMLElement {
  const row = el("div", "vet-hover-diagnostic");
  row.append(icon(severitySvg(d.severity)));
  const lines = el("div", "vet-hover-diagnostic-text");
  const message = el("div", "");
  message.append(...withCode(d.message));
  lines.append(message);
  const source = [d.source, d.code, ORIGIN_NOTE[d.origin]].filter(Boolean).join(" · ");
  if (source) lines.append(el("div", "vet-hover-detail", source));
  row.append(lines);
  return row;
}

/** The Markdown as the signature (the code blocks it opens with: the path of the item, then the item) and the docs after it. */
function sections(text: string, code: HoverAnswer["code"]): { signature?: HTMLElement; docs?: HTMLElement } {
  const parsed = el("div", "");
  parsed.innerHTML = renderMarkdown(text, code);
  const nodes = [...parsed.children];
  const lead = nodes.findIndex((n) => n.tagName !== "PRE");
  const signature = nodes.slice(0, lead < 0 ? nodes.length : lead);
  let docs = nodes.slice(signature.length);
  if (signature.length && docs[0]?.tagName === "HR") docs = docs.slice(1);
  const part = (className: string, children: Element[]) => {
    if (!children.length) return undefined;
    const section = el("div", `vet-md ${className}`);
    section.append(...children);
    return section;
  };
  const sig = part("vet-hover-signature", signature);
  // A path above the item is a note on it, not part of its signature.
  for (const above of sig ? [...sig.children].slice(0, -1) : []) above.classList.add("vet-hover-path");
  return { signature: sig, docs: part("vet-hover-docs", docs) };
}

function footer(answer: HoverAnswer): HTMLElement | undefined {
  if (!answer.server) return undefined;
  const foot = el("div", "vet-hover-footer");
  foot.append(el("span", "", answer.server.name));
  if (answer.definitionKey) foot.append(el("span", "", `Go to definition ${answer.definitionKey}`));
  return foot;
}

function notice(svg: string, text: string): HTMLElement {
  const dom = el("div", "vet-hover vet-hover-notice");
  dom.append(icon(svg), el("span", "", text));
  return dom;
}

/** "rust-analyzer is indexing (62 %)". */
function indexingText(name: string, fraction: number | undefined): string {
  return `${name} is indexing${fraction === undefined ? "" : ` (${Math.round(fraction * 100)} %)`}`;
}

/**
 * What builds the popup for a hover answer and the `diagnostics` at the symbol, or nothing if
 * there is nothing to say: a server with no text for a symbol and no reason is silent, and so is
 * a file type no server is there for and a server that does not offer hover.
 */
export function hoverContent(answer: HoverAnswer, diagnostics: readonly Diagnostic[]): (() => HTMLElement) | undefined {
  if (!answer.text && !diagnostics.length) {
    const { server, unavailable } = answer;
    if (server?.indexing)
      return () => notice(iconSvg("timeline", 14, 1.6), indexingText(server.name, server.indexing?.fraction));
    if (unavailable && unavailable.kind !== "unsupported" && unavailable.kind !== "notOffered") {
      return () => notice(severitySvg("error"), unavailable.message);
    }
    return undefined;
  }
  return () => {
    const dom = el("div", "vet-hover");
    for (const d of diagnostics) dom.append(diagnosticRow(d));
    const { signature, docs } = sections(answer.text, answer.code);
    if (signature) dom.append(signature);
    if (docs) {
      const body = el("div", "vet-hover-body");
      body.append(docs);
      dom.append(body);
    }
    const foot = docs && footer(answer);
    if (foot) dom.append(foot);
    return dom;
  };
}

/** Fades the end of the docs while more of them lie below the visible part. Needs the popup to be laid out. */
export function fadeOnOverflow(popup: HTMLElement) {
  const body = popup.querySelector(".vet-hover-body");
  const docs = body?.firstElementChild;
  if (!body || !docs) return;
  const update = () => body.toggleAttribute("data-more", docs.scrollHeight - docs.scrollTop - docs.clientHeight > 1);
  docs.addEventListener("scroll", update);
  update();
}
