// Hover and Mod+click on the deleted lines of a unified diff: base-side text. `unifiedMergeView`
// draws them as block widgets outside the editor's document, so CodeMirror's own position lookups
// do not see them; this maps the pointer to a base line and column from the DOM.

import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { type Extension } from "@codemirror/state";
import { type EditorView, ViewPlugin } from "@codemirror/view";
import { type HoverAnchor, hoverPopups } from "./hoverDom";
import { wordRange } from "./hoverPopup";
import { type SemanticHooks, type TextPosition, gotoOnClick, queryHover } from "./semantic";

/** The text position under a point; Safari only has the older `caretRangeFromPoint`. */
function caretAt(x: number, y: number): { node: Node; offset: number } | undefined {
  const modern: Partial<Document> = document;
  if (modern.caretPositionFromPoint) {
    const p = modern.caretPositionFromPoint(x, y);
    return p ? { node: p.offsetNode, offset: p.offset } : undefined;
  }
  // eslint-disable-next-line @typescript-eslint/no-deprecated -- WebKit before 17.5 (the macOS host) has only this
  const r = document.caretRangeFromPoint(x, y);
  return r ? { node: r.startContainer, offset: r.startOffset } : undefined;
}

/** The base-side 0-based line and UTF-16 column under the pointer, and the line's element, if it is on a deleted line. */
function baseAt(view: EditorView, event: MouseEvent): (TextPosition & { lineEl: Element }) | undefined {
  const target = event.target instanceof Element ? event.target : null;
  const lineEl = target?.closest(".cm-deletedLine");
  const chunkEl = lineEl?.closest(".cm-deletedChunk");
  if (!lineEl || !chunkEl) return undefined;
  const index = [...chunkEl.querySelectorAll(":scope > .cm-deletedLine")].indexOf(lineEl);
  const at = view.posAtDOM(chunkEl);
  const chunk = getChunks(view.state)?.chunks.find((c) => c.fromB === at);
  const caret = caretAt(event.clientX, event.clientY);
  if (index < 0 || !chunk || !caret || !lineEl.contains(caret.node)) return undefined;
  const range = document.createRange();
  range.selectNodeContents(lineEl);
  range.setEnd(caret.node, caret.offset);
  return {
    line: getOriginalDoc(view.state).lineAt(chunk.fromA).number - 1 + index,
    col: range.toString().length,
    lineEl,
  };
}

/** The viewport rectangle of the text between two offsets of an element's text. */
function textRect(el: Element, from: number, to: number) {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  let offset = 0;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const end = offset + (node.nodeValue?.length ?? 0);
    if (from >= offset && from < end) range.setStart(node, from - offset);
    if (to > offset && to <= end) {
      range.setEnd(node, to - offset);
      break;
    }
    offset = end;
  }
  const { left, right, top, bottom } = range.getBoundingClientRect();
  return { left, right, top, bottom };
}

/** The word of a deleted line under the pointer. */
function deletedWordAt(view: EditorView, event: MouseEvent): (HoverAnchor & { at: TextPosition }) | undefined {
  const base = baseAt(view, event);
  const word = base && wordRange(base.lineEl.textContent, base.col);
  if (!base || !word) return undefined;
  const rect = textRect(base.lineEl, word.from, word.to);
  const { clientX: x, clientY: y } = event;
  if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return undefined;
  return { key: `base:${base.line}:${word.from}-${word.to}`, rect, at: { line: base.line, col: word.from } };
}

export function deletedLines(path: string, hooks: SemanticHooks): Extension {
  const goto = ViewPlugin.fromClass(
    class {
      constructor(readonly view: EditorView) {
        view.dom.addEventListener("mousedown", this.down, true);
      }

      down = (event: MouseEvent) => {
        if (gotoOnClick(event, hooks, "base", path, () => baseAt(this.view, event))) event.stopPropagation();
      };

      destroy() {
        this.view.dom.removeEventListener("mousedown", this.down, true);
      }
    },
  );
  return [goto, hoverPopups(deletedWordAt, (anchor, signal) => queryHover(hooks, "base", path, anchor.at, [], signal))];
}
