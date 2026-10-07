// One file's CodeMirror editor(s): unified (`unifiedMergeView`) or side by side
// (`MergeView`), with the review decorations of `reviewState.ts` (SPEC §10).

import { MergeView, getChunks, unifiedMergeView } from "@codemirror/merge";
import { EditorState, type Extension, type StateEffect, type StateField } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { minimalSetup } from "codemirror";
import type { Diagnostic } from "../../../generated/Diagnostic";
import type { Hunk } from "../../../generated/Hunk";
import type { Mark } from "../../../generated/Mark";
import type { TextChange } from "../../../generated/TextChange";
import type { HunkState } from "../../../generated/HunkState";
import type { Side } from "../../../generated/Side";
import type { TextSpan } from "../../../generated/TextSpan";
import type { Mode } from "../../../lib/viewMode";
import { deletedLines } from "./deletedLines";
import { backendDiff } from "./backendDiff";
import type { BarActions } from "./hunkBar";
import { type SemanticHooks, revealSpan, semanticExtensions, showDiagnostics } from "./semantic";
import { deletedHighlights } from "./deletedHighlights";
import { highlightExtension } from "./highlightExtension";
import type { Highlights } from "./highlights";
import { type ReviewState, deletedChunkStyle, reviewField, sideConfig } from "./reviewState";

export interface Loaded {
  base: string;
  target: string;
  hunks: Hunk[];
  /** The diff, down to the words that differ; highlighting and chunks come from it. */
  changes: TextChange[];
  /** The syntax highlighting of each side's text. */
  baseHighlights: Highlights;
  targetHighlights: Highlights;
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
  /** More extensions of the target view. */
  extra: Extension[] = [],
): Handle {
  const targetField = reviewField(
    sideConfig(loaded.hunks, "target", review.symbols, hooks),
    loaded.marks,
    review.states,
  );
  const diffConfig = { override: backendDiff(loaded.base, loaded.target, loaded.changes) };
  const shown = (ms: number) => hooks.hoverShown(ms);
  const target: Extension[] = [
    minimalSetup,
    targetField,
    lineNumbers(),
    highlightExtension(loaded.targetHighlights),
    barText,
    ...extra,
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
          syntaxHighlightDeletions: false,
          collapseUnchanged,
          diffConfig,
        }),
        deletedHighlights(loaded.baseHighlights),
        deletedChunkStyle(targetField),
        deletedLines(path, hooks, shown),
      ],
    });
    return handle(view, [view], targetField);
  }
  const base: Extension[] = [
    reviewField(sideConfig(loaded.hunks, "base", review.symbols, null), [], review.states),
    lineNumbers(),
    highlightExtension(loaded.baseHighlights),
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
