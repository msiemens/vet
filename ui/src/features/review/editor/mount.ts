// One file's CodeMirror editor(s): unified (`unifiedMergeView`) or side by side
// (`MergeView`), with the review decorations of `reviewState.ts` (SPEC §10).

import { Change, MergeView, diff, getChunks, presentableDiff, unifiedMergeView } from "@codemirror/merge";
import { EditorState, type Extension, type StateEffect, type StateField } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { minimalSetup } from "codemirror";
import type { Diagnostic } from "../../../generated/Diagnostic";
import type { Hunk } from "../../../generated/Hunk";
import type { Mark } from "../../../generated/Mark";
import type { HunkState } from "../../../generated/HunkState";
import type { Side } from "../../../generated/Side";
import type { TextSpan } from "../../../generated/TextSpan";
import { deletedLines } from "./deletedLines";
import type { BarActions } from "./hunkBar";
import { type SemanticHooks, revealSpan, semanticExtensions, showDiagnostics } from "./semantic";
import { syntax } from "./language";
import { type ReviewState, deletedChunkStyle, reviewField, sideConfig } from "./reviewState";

export type Mode = "unified" | "split";

export interface Loaded {
  base: string;
  target: string;
  hunks: Hunk[];
  marks: Mark[];
}

export interface Hooks extends BarActions, SemanticHooks {
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
        return presentableDiff(a.slice(fromA, toA), b.slice(fromB, toB)).map(
          (c) => new Change(c.fromA + fromA, c.toA + fromA, c.fromB + fromB, c.toB + fromB),
        );
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
  /** Replaces the target view's diagnostics. */
  showDiagnostics(diagnostics: readonly Diagnostic[]): void;
  /** Scrolls to and marks a span of `side`'s text; a base span has no editor of its own in a unified view and is left to the caller. */
  revealSpan(side: Side, span: TextSpan): void;
  /** Review decorations (changed lines, marks, bars) on the target view; for integrity checks. */
  decorationCount(): number;
  chunkCount(): number;
  destroy(): void;
}

/** Unchanged regions collapse to a bar that expands on click: `margin` lines of context stay around each hunk, and a bar hides at least `minSize` lines. */
export interface Collapse {
  margin: number;
  minSize: number;
}

const barText = EditorState.phrases.of({ "$ unchanged lines": "⋯ $ unchanged lines" });

export function mountFile(
  host: HTMLElement,
  path: string,
  loaded: Loaded,
  review: Review,
  mode: Mode,
  hooks: Hooks,
  /** Absent: show the whole file. */
  collapseUnchanged?: Collapse,
): Handle {
  const targetField = reviewField(
    sideConfig(loaded.hunks, "target", review.symbols, hooks),
    loaded.marks,
    review.states,
  );
  const diffConfig = { override: backendDiff(loaded) };
  const shown = (ms: number) => hooks.hoverShown(ms);
  const target: Extension[] = [
    minimalSetup,
    targetField,
    lineNumbers(),
    syntax(path),
    barText,
    EditorView.domEventHandlers({ keydown: (event) => (hooks.keydown(event), false) }),
    ...semanticExtensions(path, "target", hooks, shown),
  ];
  if (mode === "unified") {
    const view = new EditorView({
      parent: host,
      doc: loaded.target,
      extensions: [
        ...target,
        unifiedMergeView({
          original: loaded.base,
          mergeControls: false,
          gutter: true,
          highlightChanges: true,
          collapseUnchanged,
          diffConfig,
        }),
        deletedChunkStyle(targetField),
        deletedLines(path, hooks, shown),
      ],
    });
    return handle(view, [view], targetField);
  }
  const base: Extension[] = [
    reviewField(sideConfig(loaded.hunks, "base", review.symbols, null), [], review.states),
    lineNumbers(),
    syntax(path),
    barText,
    EditorState.readOnly.of(true),
    EditorView.editable.of(false),
    ...semanticExtensions(path, "base", hooks, shown),
  ];
  const merge = new MergeView({
    parent: host,
    a: { doc: loaded.base, extensions: base },
    b: { doc: loaded.target, extensions: target },
    gutter: true,
    highlightChanges: true,
    collapseUnchanged,
    diffConfig,
  });
  return handle(merge.b, [merge.a, merge.b], targetField, () => merge.destroy());
}

function handle(
  view: EditorView,
  views: EditorView[],
  field: StateField<ReviewState>,
  destroy = () => view.destroy(),
): Handle {
  return {
    view,
    views,
    dispatchAll: (effect) => views.forEach((v) => v.dispatch({ effects: effect })),
    showDiagnostics: (diagnostics) => showDiagnostics(view, diagnostics),
    revealSpan: (side, span) => {
      if (side === "target") revealSpan(view, span, true);
      else if (views.length > 1) revealSpan(views[0], span, false);
    },
    decorationCount: () => view.state.field(field).decos.size,
    chunkCount: () => getChunks(view.state)?.chunks.length ?? 0,
    destroy,
  };
}
