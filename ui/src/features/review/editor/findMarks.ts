// The matches of a find as one editor draws them: a mark on each match in the document, the
// current one stronger; the matches on a unified diff's deleted lines, which are widgets outside
// the document and are marked in their DOM; and on each "N unchanged lines" bar the number of
// matches it hides. What to mark is pushed in by the view that searched (`setFindMarks`).

import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { type Extension, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { NO_MARKS, type ViewMarks } from "./findView";
import { lineStart } from "./positions";
import { hiddenRegions } from "./semantic";

export const setFindMarks = StateEffect.define<ViewMarks>();

interface Drawn {
  marks: ViewMarks;
  /** The ranges as document positions. */
  spans: { from: number; to: number }[];
  decos: DecorationSet;
}

const className = (current: boolean, dim: boolean) =>
  `vet-find${current ? " vet-find-current" : ""}${dim ? " vet-find-dim" : ""}`;

const findField = StateField.define<Drawn>({
  create: () => ({ marks: NO_MARKS, spans: [], decos: Decoration.none }),
  update(value, tr) {
    for (const e of tr.effects) {
      if (!e.is(setFindMarks)) continue;
      const { doc } = tr.state;
      const spans = e.value.ranges.map((r) => ({
        from: lineStart(doc, r.line) + r.from,
        to: lineStart(doc, r.line) + r.to,
      }));
      const decos = Decoration.set(
        e.value.ranges.map((r, i) =>
          Decoration.mark({ class: className(r.current, e.value.dim) }).range(spans[i].from, spans[i].to),
        ),
        true,
      );
      return { marks: e.value, spans, decos };
    }
    return value;
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
});

const MARK = "vet-find";

/** Takes the marks out of a deleted line, leaving its text. */
function unmark(line: Element) {
  const marks = line.querySelectorAll(`mark.${MARK}`);
  for (const mark of marks) mark.replaceWith(...mark.childNodes);
  if (marks.length) line.normalize();
}

/** The text nodes below `line`, in order. */
function textNodes(line: Element): Text[] {
  const out: Text[] = [];
  const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) out.push(node as Text);
  return out;
}

/** Wraps the text of `line` between two offsets in marks, one per text node it spans. */
function wrap(line: Element, from: number, to: number, cls: string) {
  let at = 0;
  for (const node of textNodes(line)) {
    const end = at + node.length;
    const [a, b] = [Math.max(from, at) - at, Math.min(to, end) - at];
    at = end;
    if (b <= a) continue;
    const piece = node.splitText(a);
    piece.splitText(b - a);
    const mark = document.createElement("mark");
    mark.className = cls;
    piece.replaceWith(mark);
    mark.append(piece);
  }
}

/** Marks the matches in the deleted lines the view draws. */
const deletedMarks = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {
      this.schedule();
    }

    update(u: ViewUpdate) {
      const marked = u.startState.field(findField) !== u.state.field(findField);
      if (marked || u.docChanged || u.viewportChanged) this.schedule();
    }

    /** The widgets are created while the view updates its DOM, so they are looked for in the measure phase after it. */
    schedule() {
      this.view.requestMeasure({
        key: this,
        read: () => null,
        write: (_, view) => {
          const { marks } = view.state.field(findField);
          const chunks = getChunks(view.state)?.chunks ?? [];
          for (const chunkEl of view.dom.querySelectorAll(".cm-deletedChunk")) {
            const chunk = chunks.find((c) => c.fromB === view.posAtDOM(chunkEl));
            const first = chunk ? getOriginalDoc(view.state).lineAt(chunk.fromA).number - 1 : -1;
            [...chunkEl.querySelectorAll(":scope > .cm-deletedLine")].forEach((line, i) => {
              unmark(line);
              for (const m of marks.removed)
                if (m.line === first + i) wrap(line, m.from, m.to, className(m.current, marks.dim));
            });
          }
        },
      });
    }
  },
);

const GAP = "vet-find-gap";

/** Adds to each "N unchanged lines" bar how many matches it hides. */
const gapBadges = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {
      this.schedule();
    }

    update(u: ViewUpdate) {
      const marked = u.startState.field(findField) !== u.state.field(findField);
      if (marked || u.docChanged || u.viewportChanged || u.transactions.length) this.schedule();
    }

    schedule() {
      this.view.requestMeasure({
        key: this,
        read: () => null,
        write: (_, view) => {
          const { spans } = view.state.field(findField);
          const regions = hiddenRegions(view);
          for (const bar of view.dom.querySelectorAll(".cm-collapsedLines")) {
            bar.querySelector(`.${GAP}`)?.remove();
            const region = regions.find((r) => r.from === view.posAtDOM(bar));
            const n = region ? spans.filter((s) => s.from >= region.from && s.to <= region.to).length : 0;
            if (!n) continue;
            const badge = document.createElement("span");
            badge.className = GAP;
            badge.textContent = n === 1 ? "1 match" : `${n} matches`;
            bar.append(badge);
          }
        },
      });
    }
  },
);

/** The marks of a find in an editor; `deleted` also marks the deleted lines of a unified diff. */
export function findMarks(deleted: boolean): Extension {
  return [findField, gapBadges, ...(deleted ? [deletedMarks] : [])];
}
