// One file's CodeMirror editor(s): unified (`unifiedMergeView`) or side by side
// (`MergeView`), with the review decorations of `reviewState.ts` (SPEC §10).

import { MergeView, getChunks, unifiedMergeView } from "@codemirror/merge";
import { Compartment, EditorState, type Extension, type StateEffect } from "@codemirror/state";
import { standardKeymap } from "@codemirror/commands";
import { EditorView, drawSelection, highlightSpecialChars, keymap, lineNumbers } from "@codemirror/view";
import type { Diagnostic } from "../../../generated/Diagnostic";
import type { Hunk } from "../../../generated/Hunk";
import type { TextChange } from "../../../generated/TextChange";
import type { HunkState } from "../../../generated/HunkState";
import type { Side } from "../../../generated/Side";
import type { TextSpan } from "../../../generated/TextSpan";
import type { Highlights } from "../../../lib/highlights";
import type { Mode } from "../../../lib/viewMode";
import { deletedLines } from "./deletedLines";
import { backendDiff } from "./backendDiff";
import type { BarActions } from "./hunkBar";
import { type SemanticHooks, revealSpan, semanticExtensions, showDiagnostics } from "./semantic";
import { deletedHighlights } from "./deletedHighlights";
import { highlightExtension } from "./highlightExtension";
import { deletedChunkStyle, reviewField, sideConfig } from "./reviewState";

export interface Loaded {
  base: string;
  target: string;
  hunks: Hunk[];
  /** The diff, down to the words that differ; highlighting and chunks come from it. */
  changes: TextChange[];
  /** The syntax highlighting of each side's text. */
  baseHighlights: Highlights;
  targetHighlights: Highlights;
}

export interface Hooks extends BarActions, SemanticHooks {
  /** A hover tooltip is on screen; `ms` runs from the start of the hover query to its paint. */
  hoverShown?(ms: number): void;
  /** A key went down in the target editor. */
  keydown?(event: KeyboardEvent): void;
}

export interface Handle {
  /** The target-side view. */
  view: EditorView;
  views: EditorView[];
  dispatchAll(effect: StateEffect<unknown>): void;
  /** Replaces the target view's diagnostics. */
  showDiagnostics(diagnostics: readonly Diagnostic[]): void;
  /** Scrolls to and marks a span of `side`'s text; a base span has no editor of its own in a unified view and is left to the caller. */
  revealSpan(side: Side, span: TextSpan): void;
  chunkCount(): number;
  destroy(): void;
}

/** Unchanged regions collapse to a bar that expands on click: `margin` lines of context stay around each hunk, and a bar hides at least `minSize` lines. */
export interface Collapse {
  margin: number;
  minSize: number;
}

const barText = EditorState.phrases.of({ "$ unchanged lines": "⋯ $ unchanged lines" });

const drawnSelection = new Compartment();
const drawn = drawSelection();

/**
 * The drawn selection and cursor, from the editor's first focus or press on. Until then there is
 * neither to draw, and in WebKit their layers made creating a unified editor several times slower,
 * which the editors mounting during a scroll pay for in dropped frames.
 */
const drawSelectionOnUse: Extension = [
  drawnSelection.of([]),
  EditorView.domEventHandlers({
    focus: (_, view) => use(view),
    mousedown: (_, view) => use(view),
  }),
];

/** Turns the drawn selection on; never takes the event from CodeMirror's own handling. */
function use(view: EditorView): false {
  if (drawnSelection.get(view.state) !== drawn) view.dispatch({ effects: drawnSelection.reconfigure(drawn) });
  return false;
}

/**
 * What a read-only editor needs of CodeMirror's setup: control characters made visible, the drawn
 * selection and cursor, and the caret keys. No undo history, no editing keys and no fallback
 * highlight style (highlights come from the backend).
 */
const readOnlySetup: Extension = [highlightSpecialChars(), drawSelectionOnUse, keymap.of(standardKeymap)];

export function mountFile(
  host: HTMLElement,
  path: string,
  loaded: Loaded,
  /** The review state of each hunk. */
  states: HunkState[],
  mode: Mode,
  /** A hunk bar's height in px (`--h-hunk`). */
  barHeight: number,
  hooks: Hooks,
  /** Absent: show the whole file. */
  collapseUnchanged?: Collapse,
  /** More extensions of the target view. */
  extra: Extension[] = [],
): Handle {
  const targetField = reviewField(sideConfig(loaded.hunks, "target", hooks, barHeight), states);
  const diffConfig = { override: backendDiff(loaded.base, loaded.target, loaded.changes) };
  const shown = (ms: number) => hooks.hoverShown?.(ms);
  const target: Extension[] = [
    readOnlySetup,
    targetField,
    lineNumbers(),
    highlightExtension(loaded.targetHighlights),
    barText,
    // Vet never writes the tree: both sides keep a caret and selection but take no edits.
    EditorState.readOnly.of(true),
    ...extra,
    EditorView.domEventHandlers({ keydown: (event) => (hooks.keydown?.(event), false) }),
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
    return handle(view, [view]);
  }
  const base: Extension[] = [
    reviewField(sideConfig(loaded.hunks, "base", null, barHeight), states),
    lineNumbers(),
    highlightExtension(loaded.baseHighlights),
    barText,
    EditorState.readOnly.of(true),
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
  return handle(merge.b, [merge.a, merge.b], () => merge.destroy());
}

function handle(view: EditorView, views: EditorView[], destroy = () => view.destroy()): Handle {
  return {
    view,
    views,
    dispatchAll: (effect) => views.forEach((v) => v.dispatch({ effects: effect })),
    showDiagnostics: (diagnostics) => showDiagnostics(view, diagnostics),
    revealSpan: (side, span) => {
      if (side === "target") revealSpan(view, span, true);
      else if (views.length > 1) revealSpan(views[0], span, false);
    },
    chunkCount: () => getChunks(view.state)?.chunks.length ?? 0,
    destroy,
  };
}
