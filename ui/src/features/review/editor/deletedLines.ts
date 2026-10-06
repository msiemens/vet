// Hover and Mod+click on the deleted lines of a unified diff: base-side text. `unifiedMergeView`
// draws them as block widgets outside the editor's document, so CodeMirror's own hover and
// position lookups do not see them; this maps the pointer to a base line and column from the DOM.

import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { type EditorView, ViewPlugin } from "@codemirror/view";
import { nextPaint } from "../../../lib/nextPaint";
import { hoverContent } from "./hoverContent";
import { isGotoClick, type SemanticHooks } from "./semantic";

const HOVER_MS = 50;

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

/** The base-side 0-based line and UTF-16 column under the pointer, if it is on a deleted line. */
function baseAt(view: EditorView, event: MouseEvent): { line: number; col: number } | undefined {
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
  return { line: getOriginalDoc(view.state).lineAt(chunk.fromA).number - 1 + index, col: range.toString().length };
}

export function deletedLines(path: string, hooks: SemanticHooks, hoverShown: (ms: number) => void) {
  return ViewPlugin.fromClass(
    class {
      timer?: ReturnType<typeof setTimeout>;
      tip?: HTMLElement;
      /** Identifies the latest pointer position, so a slow answer for an earlier one is dropped. */
      seq = 0;

      constructor(readonly view: EditorView) {
        view.dom.addEventListener("mousemove", this.move);
        view.dom.addEventListener("mouseleave", this.hide);
        view.dom.addEventListener("mousedown", this.down, true);
      }

      hide = () => {
        clearTimeout(this.timer);
        this.seq++;
        this.tip?.remove();
        this.tip = undefined;
      };

      move = (event: MouseEvent) => {
        this.hide();
        if (!(event.target instanceof Element && event.target.closest(".cm-deletedLine"))) return;
        const { clientX: x, clientY: y } = event;
        const seq = this.seq;
        this.timer = setTimeout(() => void this.show(event, x, y, seq), HOVER_MS);
      };

      async show(event: MouseEvent, x: number, y: number, seq: number) {
        const at = baseAt(this.view, event);
        if (!at) return;
        const started = performance.now();
        const text = await hooks.hover("base", path, at.line, at.col);
        if (!text || seq !== this.seq) return;
        const tip = document.createElement("div");
        tip.className = "cm-tooltip vet-hover vet-hover-floating";
        tip.append(hoverContent(text, path));
        tip.style.left = `${x}px`;
        tip.style.top = `${y - 8}px`;
        document.body.append(tip);
        this.tip = tip;
        void nextPaint().then(() => hoverShown(performance.now() - started));
      }

      down = (event: MouseEvent) => {
        this.hide();
        if (!isGotoClick(event)) return;
        const at = baseAt(this.view, event);
        if (!at) return;
        event.preventDefault();
        event.stopPropagation();
        hooks.definition("base", path, at.line, at.col);
      };

      destroy() {
        this.hide();
        this.view.dom.removeEventListener("mousemove", this.move);
        this.view.dom.removeEventListener("mouseleave", this.hide);
        this.view.dom.removeEventListener("mousedown", this.down, true);
      }
    },
  );
}
