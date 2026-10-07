// Highlights for the deleted lines of a unified diff. `unifiedMergeView` draws them as widgets
// outside the document and would parse them itself; they are coloured from the base side's
// highlights instead, once the widget's DOM exists.

import { getChunks } from "@codemirror/merge";
import type { Extension } from "@codemirror/state";
import { type EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { type Highlights, pieces } from "../../../lib/highlights";

const PAINTED = "data-highlighted";

/** The text nodes below `line`, in order. */
function textNodes(line: Element): Text[] {
  const out: Text[] = [];
  const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) out.push(node as Text);
  return out;
}

/** Wraps the coloured parts of `node`, which starts at `start` in the base document. */
function paint(node: Text, start: number, highlights: Highlights) {
  const parts = pieces(highlights, start, node.data);
  if (parts.length === 1 && typeof parts[0] === "string") return;
  const fragment = document.createDocumentFragment();
  for (const part of parts) {
    if (typeof part === "string") fragment.append(part);
    else {
      const span = document.createElement("span");
      span.className = part.className;
      span.textContent = part.text;
      fragment.append(span);
    }
  }
  node.replaceWith(fragment);
}

interface Fresh {
  chunk: Element;
  /** Where the chunk's text starts in the base document. */
  from: number;
}

/** Colours the deleted lines of a unified merge view, whose base text is highlighted by `highlights`. */
export function deletedHighlights(highlights: Highlights): Extension {
  if (!highlights.from.length) return [];
  return ViewPlugin.fromClass(
    class {
      constructor(readonly view: EditorView) {
        this.schedule();
      }

      update(update: ViewUpdate) {
        if (update.docChanged || update.viewportChanged) this.schedule();
      }

      /** The widgets are created while the view updates its DOM, so they are looked for in the measure phase after it. */
      schedule() {
        this.view.requestMeasure<Fresh[]>({
          key: this,
          read: (view) => {
            const chunks = getChunks(view.state)?.chunks ?? [];
            const fresh: Fresh[] = [];
            for (const chunk of view.dom.querySelectorAll(`.cm-deletedChunk:not([${PAINTED}])`)) {
              const at = view.posAtDOM(chunk);
              const known = chunks.find((c) => c.fromB === at);
              if (known) fresh.push({ chunk, from: known.fromA });
            }
            return fresh;
          },
          write: (fresh) => {
            for (const { chunk, from } of fresh) {
              chunk.setAttribute(PAINTED, "");
              let at = from;
              for (const line of chunk.querySelectorAll(":scope > .cm-deletedLine")) {
                for (const node of textNodes(line)) {
                  paint(node, at, highlights);
                  at += node.length;
                }
                // The line break between two lines of the base text.
                at += 1;
              }
            }
          },
        });
      }
    },
  );
}
