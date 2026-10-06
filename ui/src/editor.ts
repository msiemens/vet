// One file's CodeMirror editor(s): unified (`unifiedMergeView`) or side by side
// (`MergeView`), with review-state decorations on every changed line (including
// the deleted lines of the unified view) and inline mark decorations from the
// backend (SPEC §10).

import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { yaml } from "@codemirror/lang-yaml";
import { HighlightStyle, StreamLanguage, syntaxHighlighting } from "@codemirror/language";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { Change, MergeView, diff, getChunks, presentableDiff, unifiedMergeView } from "@codemirror/merge";
import { tags as t } from "@lezer/highlight";
import { EditorState, StateEffect, StateField, type Extension, type Range, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, hoverTooltip, lineNumbers } from "@codemirror/view";
import { minimalSetup } from "codemirror";
import type { Hunk } from "./generated/Hunk";
import type { Mark } from "./generated/Mark";
import type { MarkKind } from "./generated/MarkKind";
import { nextPaint } from "./metrics";

export type Mode = "unified" | "split";

/** Syntax highlighting is part of what the spike measures; unknown file types stay plain. */
function language(path: string): Extension {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  switch (ext) {
    case "rs": return rust();
    case "py": case "pyi": return python();
    case "ts": case "mts": case "cts": return javascript({ typescript: true });
    case "tsx": return javascript({ typescript: true, jsx: true });
    case "js": case "mjs": case "cjs": return javascript();
    case "jsx": return javascript({ jsx: true });
    case "json": return json();
    case "md": case "markdown": return markdown();
    case "yaml": case "yml": return yaml();
    case "toml": return StreamLanguage.define(toml);
    case "sh": case "bash": return StreamLanguage.define(shell);
    case "css": return css();
    case "html": case "htm": case "svg": return html();
    default: return [];
  }
}

const highlightStyle = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier], color: "#cc7832" },
  { tag: [t.string, t.special(t.string)], color: "#6a8759" },
  { tag: [t.number, t.bool, t.null], color: "#6897bb" },
  { tag: [t.comment, t.docComment], color: "#808080", fontStyle: "italic" },
  { tag: [t.typeName, t.className, t.namespace], color: "#e8bf6a" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], color: "#ffc66d" },
  { tag: [t.propertyName, t.definition(t.variableName)], color: "#9876aa" },
  { tag: [t.attributeName, t.meta, t.annotation, t.tagName], color: "#bbb529" },
  { tag: [t.heading, t.strong], fontWeight: "bold", color: "#ffc66d" },
  { tag: [t.link, t.url], color: "#589df6" },
]);

const syntax = (path: string): Extension => [language(path), syntaxHighlighting(highlightStyle)];

export interface Loaded {
  base: string;
  target: string;
  hunks: Hunk[];
  marks: Mark[];
}

export interface Hooks {
  hover(path: string, line: number, col: number): Promise<string>;
  /** A hover tooltip is on screen; `ms` runs from the start of the hover query to its paint. */
  hoverShown(ms: number): void;
  keydown(event: KeyboardEvent): void;
}

export const setSeen = StateEffect.define<boolean[]>();
export const setCurrent = StateEffect.define<number | null>();

const lineDeco = {
  seen: Decoration.line({ class: "vet-seen" }),
  unseen: Decoration.line({ class: "vet-unseen" }),
  seenCurrent: Decoration.line({ class: "vet-seen vet-current" }),
  unseenCurrent: Decoration.line({ class: "vet-unseen vet-current" }),
};
const markDeco: Record<MarkKind, Decoration> = {
  symbol: Decoration.mark({ class: "vet-m-symbol" }),
  reachability: Decoration.mark({ class: "vet-m-reach" }),
  note: Decoration.mark({ class: "vet-m-note" }),
};

interface Span {
  from: number;
  to: number;
  empty: boolean;
  /** Start of the line where this hunk sits in the document: where a unified view places a deleted block. */
  at: number;
}

interface ReviewState {
  spans: Span[];
  marks: { from: number; to: number; kind: MarkKind }[];
  seen: boolean[];
  current: number | null;
  decos: DecorationSet;
}

const stateClass = (state: { seen: boolean[]; current: number | null }, i: number) =>
  `${state.seen[i] ? "seen" : "unseen"}${state.current === i ? "Current" : ""}` as keyof typeof lineDeco;

function build(state: Omit<ReviewState, "decos">, doc: Text): DecorationSet {
  const out: Range<Decoration>[] = [];
  state.spans.forEach((span, i) => {
    if (span.empty) return;
    const deco = lineDeco[stateClass(state, i)];
    for (let pos = span.from; pos <= span.to; ) {
      const line = doc.lineAt(pos);
      out.push(deco.range(line.from));
      pos = line.to + 1;
    }
  });
  for (const m of state.marks) if (m.from < m.to) out.push(markDeco[m.kind].range(m.from, m.to));
  return Decoration.set(out, true);
}

/**
 * Review-state decorations for one side. `lines` are `[start, length]` 0-based
 * line ranges (one per hunk); positions are mapped through edits, so decorations
 * follow their content.
 */
function reviewField(lines: [number, number][], marks: Mark[], seen: boolean[]): StateField<ReviewState> {
  return StateField.define<ReviewState>({
    create(state) {
      const doc = state.doc;
      const spans = lines.map(([start, len]): Span => {
        const at = start < doc.lines ? doc.line(start + 1).from : doc.length;
        if (len === 0 || start >= doc.lines) return { from: 0, to: 0, empty: true, at };
        return { from: at, to: doc.line(Math.min(start + len, doc.lines)).to, empty: false, at };
      });
      const placed = marks.flatMap((m) => {
        if (m.line >= doc.lines) return [];
        const line = doc.line(m.line + 1);
        return [{ from: Math.min(line.from + m.from, line.to), to: Math.min(line.from + m.to, line.to), kind: m.kind }];
      });
      const base = { spans, marks: placed, seen, current: null };
      return { ...base, decos: build(base, doc) };
    },
    update(value, tr) {
      let next = value;
      if (tr.docChanged) {
        const map = (pos: number, assoc: -1 | 1) => tr.changes.mapPos(pos, assoc);
        next = {
          ...next,
          spans: next.spans.map((s) => ({ ...s, at: map(s.at, 1), ...(s.empty ? {} : { from: map(s.from, -1), to: map(s.to, 1) }) })),
          marks: next.marks.map((m) => ({ ...m, from: map(m.from, -1), to: map(m.to, 1) })),
          decos: next.decos.map(tr.changes),
        };
      }
      let rebuild = false;
      for (const effect of tr.effects) {
        if (effect.is(setSeen)) (next = { ...next, seen: effect.value }), (rebuild = true);
        else if (effect.is(setCurrent)) (next = { ...next, current: effect.value }), (rebuild = true);
      }
      return rebuild ? { ...next, decos: build(next, tr.state.doc) } : next;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
}

/**
 * The unified view renders deleted lines as block widgets that carry no line
 * decorations, so they get the review-state classes of their hunk after each
 * update. A widget belongs to the hunk whose `at` is the widget's position.
 */
function deletedChunkStyle(field: StateField<ReviewState>): Extension {
  const style = (view: EditorView) => {
    const state = view.state.field(field);
    const byPos = new Map(state.spans.map((s, i) => [s.at, i]));
    for (const el of view.dom.querySelectorAll<HTMLElement>(".cm-deletedChunk")) {
      el.classList.remove("vet-seen", "vet-unseen", "vet-current");
      const i = byPos.get(view.posAtDOM(el));
      if (i !== undefined) el.classList.add(...lineDeco[stateClass(state, i)].spec.class.split(" "));
    }
  };
  return ViewPlugin.define((view) => {
    // Widgets are created after plugin updates, so style them in the measure phase.
    const schedule = () => view.requestMeasure({ read: () => null, write: () => style(view), key: field });
    schedule();
    return { update: schedule };
  });
}

/**
 * Feeds the backend's hunks to the merge extensions so highlighting and review
 * decorations share one diff. The merge code calls this with the whole documents
 * at build time and afterwards only with slices around local edits. Anything but
 * the original pair falls back to CodeMirror's diff: after an edit the backend's
 * hunks no longer describe the document, and only the edited region is re-diffed.
 * CodeMirror still merges changes under 3 characters apart (a blank line) into
 * one chunk, so a chunk can span two backend hunks.
 */
function backendDiff(loaded: Loaded): (a: string, b: string) => readonly Change[] {
  const starts = (text: string) => {
    const out = [0];
    for (let i = text.indexOf("\n"); i >= 0; i = text.indexOf("\n", i + 1)) out.push(i + 1);
    return out;
  };
  let changes: Change[] | undefined;
  return (a, b) => {
    if (a !== loaded.base || b !== loaded.target) return diff(a, b);
    if (!changes) {
      const [sa, sb] = [starts(a), starts(b)];
      changes = loaded.hunks.flatMap((h) => {
        const [fromA, toA] = [sa[h.baseStart] ?? a.length, sa[h.baseStart + h.baseLen] ?? a.length];
        const [fromB, toB] = [sb[h.targetStart] ?? b.length, sb[h.targetStart + h.targetLen] ?? b.length];
        if (!h.baseLen || !h.targetLen) return [new Change(fromA, toA, fromB, toB)];
        // The hunk's boundaries are the backend's; the changes inside it are CodeMirror's, for intra-line highlighting.
        return presentableDiff(a.slice(fromA, toA), b.slice(fromB, toB)).map((c) => new Change(c.fromA + fromA, c.toA + fromA, c.fromB + fromB, c.toB + fromB));
      });
    }
    return changes;
  };
}

export interface Handle {
  /** The editable target-side view. */
  view: EditorView;
  views: EditorView[];
  dispatchAll(effect: StateEffect<unknown>): void;
  /** Review decorations (changed lines plus marks) on the target view; for integrity checks. */
  decorationCount(): number;
  chunkCount(): number;
  destroy(): void;
}

export function mountFile(host: HTMLElement, path: string, loaded: Loaded, seen: boolean[], mode: Mode, hooks: Hooks): Handle {
  const targetField = reviewField(
    loaded.hunks.map((h) => [h.targetStart, h.targetLen]),
    loaded.marks,
    seen,
  );
  const diffConfig = { override: backendDiff(loaded) };
  const target: Extension[] = [
    minimalSetup,
    lineNumbers(),
    syntax(path),
    targetField,
    EditorView.domEventHandlers({ keydown: (event) => (hooks.keydown(event), false) }),
    hoverTooltip(
      async (view, pos) => {
        const started = performance.now();
        const line = view.state.doc.lineAt(pos);
        const text = await hooks.hover(path, line.number - 1, pos - line.from);
        return {
          pos,
          above: true,
          create: () => {
            const dom = document.createElement("div");
            dom.className = "vet-hover";
            dom.textContent = text;
            void nextPaint().then(() => hooks.hoverShown(performance.now() - started));
            return { dom };
          },
        };
      },
      { hoverTime: 50 },
    ),
  ];
  if (mode === "unified") {
    const view = new EditorView({
      parent: host,
      doc: loaded.target,
      extensions: [
        ...target,
        unifiedMergeView({ original: loaded.base, mergeControls: false, gutter: true, highlightChanges: true, diffConfig }),
        deletedChunkStyle(targetField),
      ],
    });
    return handle(view, [view], targetField);
  }
  const base: Extension[] = [
    lineNumbers(),
    syntax(path),
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
    reviewField(
      loaded.hunks.map((h) => [h.baseStart, h.baseLen]),
      [],
      seen,
    ),
  ];
  const merge = new MergeView({ parent: host, a: { doc: loaded.base, extensions: base }, b: { doc: loaded.target, extensions: target }, gutter: true, highlightChanges: true, diffConfig });
  return handle(merge.b, [merge.a, merge.b], targetField, () => merge.destroy());
}

function handle(view: EditorView, views: EditorView[], field: StateField<ReviewState>, destroy = () => view.destroy()): Handle {
  return {
    view,
    views,
    dispatchAll: (effect) => views.forEach((v) => v.dispatch({ effects: effect })),
    decorationCount: () => view.state.field(field).decos.size,
    chunkCount: () => getChunks(view.state)?.chunks.length ?? 0,
    destroy,
  };
}
