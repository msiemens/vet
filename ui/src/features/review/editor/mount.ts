// One file's CodeMirror editor(s): unified (`unifiedMergeView`) or side by side
// (`MergeView`), with the review decorations of `reviewState.ts` (SPEC §10).

import { Change, MergeView, diff, getChunks, presentableDiff, unifiedMergeView } from "@codemirror/merge";
import { EditorState, type Extension, type StateEffect, type StateField } from "@codemirror/state";
import { EditorView, hoverTooltip, lineNumbers } from "@codemirror/view";
import { minimalSetup } from "codemirror";
import type { Hunk } from "../../../generated/Hunk";
import type { Mark } from "../../../generated/Mark";
import { nextPaint } from "../../../lib/nextPaint";
import type { BarActions, HunkState } from "./hunkBar";
import { syntax } from "./language";
import { type ReviewState, deletedChunkStyle, reviewField, sideConfig } from "./reviewState";

export type Mode = "unified" | "split";

export interface Loaded {
  base: string;
  target: string;
  hunks: Hunk[];
  marks: Mark[];
}

export interface Hooks extends BarActions {
  hover(path: string, line: number, col: number): Promise<string>;
  /** A hover tooltip is on screen; `ms` runs from the start of the hover query to its paint. */
  hoverShown(ms: number): void;
  keydown(event: KeyboardEvent): void;
}

/** Per-hunk review display state of one file. */
export interface Review {
  states: HunkState[];
  symbols: (string | undefined)[];
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
  /** Review decorations (changed lines, marks, bars) on the target view; for integrity checks. */
  decorationCount(): number;
  chunkCount(): number;
  destroy(): void;
}

export function mountFile(host: HTMLElement, path: string, loaded: Loaded, review: Review, mode: Mode, hooks: Hooks): Handle {
  const targetField = reviewField(sideConfig(loaded.hunks, "target", review.symbols, hooks), loaded.marks, review.states);
  const diffConfig = { override: backendDiff(loaded) };
  const target: Extension[] = [
    minimalSetup,
    targetField,
    lineNumbers(),
    syntax(path),
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
      extensions: [...target, unifiedMergeView({ original: loaded.base, mergeControls: false, gutter: true, highlightChanges: true, diffConfig }), deletedChunkStyle(targetField)],
    });
    return handle(view, [view], targetField);
  }
  const base: Extension[] = [
    reviewField(sideConfig(loaded.hunks, "base", review.symbols, null), [], review.states),
    lineNumbers(),
    syntax(path),
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
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
