// One file's CodeMirror editor(s): unified (`unifiedMergeView`) or side by side
// (`MergeView`), with the review decorations of `reviewState.ts` (SPEC §10).

import { MergeView, uncollapseUnchanged, unifiedMergeView } from "@codemirror/merge";
import { Compartment, EditorState, type Extension, type StateEffect, type StateField } from "@codemirror/state";
import { standardKeymap } from "@codemirror/commands";
import { EditorView, drawSelection, highlightSpecialChars, keymap, lineNumbers } from "@codemirror/view";
import type { BaseDiff } from "../../../generated/BaseDiff";
import type { Diagnostic } from "../../../generated/Diagnostic";
import type { FileReview } from "../../../generated/FileReview";
import type { HunkState } from "../../../generated/HunkState";
import type { Hunk } from "../../../generated/Hunk";
import type { TextChange } from "../../../generated/TextChange";
import type { Side } from "../../../generated/Side";
import type { TextSpan } from "../../../generated/TextSpan";
import type { Highlights } from "../../../lib/highlights";
import type { Mode } from "../../../lib/viewMode";
import { currentLine } from "./currentLine";
import { deletedLines } from "./deletedLines";
import { backendDiff } from "./backendDiff";
import type { BarActions } from "./hunkBar";
import { baseLineOfTarget, lineStart } from "./positions";
import { type SemanticHooks, hiddenRegions, revealSpan, semanticExtensions, showDiagnostics } from "./semantic";
import { deletedHighlights } from "./deletedHighlights";
import { highlightExtension } from "./highlightExtension";
import { longLines } from "./longLines";
import { openOnDoubleClick } from "./openAtLine";
import { type BarMetrics, type ReviewState, deletedChunkStyle, reviewField, sideConfig } from "./reviewState";
import { reportHeight, softWrap } from "./softWrap";

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
  /** A line was double-clicked: the file's text at this 0-based target line. */
  openLine(line: number): void;
}

export interface Handle {
  /** The target-side view. */
  view: EditorView;
  views: EditorView[];
  dispatchAll(effect: StateEffect<unknown>): void;
  /** The review state of each hunk that the target view draws from. */
  drawn(): { hunks: HunkState[]; flagged: boolean[] };
  /** Replaces the target view's diagnostics. */
  showDiagnostics(diagnostics: readonly Diagnostic[]): void;
  /** Scrolls to and marks a span of `side`'s text; a base span has no editor of its own in a unified view and is left to the caller. */
  revealSpan(side: Side, span: TextSpan): void;
  /** Expands the collapsed region that hides a 0-based target line, in both editors of a side by side view; answers whether the line was hidden. */
  expand(line: number): boolean;
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

/** The editors wrap long lines; the owner lays them out by an estimate and hears of the height they measure. */
export interface Wrapping {
  heightChanged(): void;
}

export function mountFile(
  host: HTMLElement,
  path: string,
  loaded: Loaded,
  /** The review state of each hunk. */
  review: FileReview,
  /** The base diffs open under hunk bars. */
  baseDiffs: ReadonlyMap<number, BaseDiff>,
  mode: Mode,
  /** The sizes of a hunk bar and of the base diff under it. */
  barMetrics: BarMetrics,
  /** The characters that fit a row of the code column: a changed line with more is marked. */
  columns: number,
  hooks: Hooks,
  /** Absent: show the whole file. */
  collapseUnchanged?: Collapse,
  /** More extensions of the target view. */
  extra: Extension[] = [],
  /** Absent: long lines scroll the editor sideways. */
  wrapping?: Wrapping,
): Handle {
  const targetField = reviewField(sideConfig(loaded.hunks, "target", hooks, barMetrics), review, baseDiffs);
  const diffConfig = { override: backendDiff(loaded.base, loaded.target, loaded.changes) };
  const wrap: Extension = wrapping ? [softWrap(true), reportHeight(() => wrapping.heightChanged())] : [];
  const target: Extension[] = [
    readOnlySetup,
    currentLine(host, 1),
    wrap,
    targetField,
    longLines(targetField, columns),
    lineNumbers(),
    highlightExtension(loaded.targetHighlights),
    barText,
    // Vet never writes the tree: both sides keep a caret and selection but take no edits.
    EditorState.readOnly.of(true),
    ...extra,
    openOnDoubleClick("target", loaded.hunks, (line) => hooks.openLine(line)),
    ...semanticExtensions(path, "target", hooks),
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
        deletedLines(path, hooks),
      ],
    });
    return handle(view, [view], loaded.hunks, targetField);
  }
  const baseField = reviewField(sideConfig(loaded.hunks, "base", null, barMetrics), review, baseDiffs);
  const base: Extension[] = [
    currentLine(host, 0),
    wrap,
    baseField,
    longLines(baseField, columns),
    lineNumbers(),
    highlightExtension(loaded.baseHighlights),
    barText,
    EditorState.readOnly.of(true),
    openOnDoubleClick("base", loaded.hunks, (line) => hooks.openLine(line)),
    ...semanticExtensions(path, "base", hooks),
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
  return handle(merge.b, [merge.a, merge.b], loaded.hunks, targetField, () => merge.destroy());
}

/** Expands the collapsed region of `view` that hides the 0-based `line`; answers whether there was one. */
function expandAt(view: EditorView, line: number): boolean {
  const pos = lineStart(view.state.doc, line);
  const region = hiddenRegions(view).find((r) => r.from <= pos && pos <= r.to);
  if (!region) return false;
  view.dispatch({ effects: uncollapseUnchanged.of(region.from) });
  return true;
}

function handle(
  view: EditorView,
  views: EditorView[],
  hunks: Hunk[],
  field: StateField<ReviewState>,
  destroy = () => view.destroy(),
): Handle {
  return {
    view,
    views,
    dispatchAll: (effect) => views.forEach((v) => v.dispatch({ effects: effect })),
    drawn: () => {
      const { states, flagged } = view.state.field(field);
      return { hunks: states, flagged };
    },
    showDiagnostics: (diagnostics) => showDiagnostics(view, diagnostics),
    revealSpan: (side, span) => {
      if (side === "target") revealSpan(view, span, true);
      else if (views.length > 1) revealSpan(views[0], span, false);
    },
    expand: (line) => {
      if (!expandAt(view, line)) return false;
      // A manual expand opens both sides' bars; the base side's line is the same text, moved by the hunks above it.
      if (views.length > 1) expandAt(views[0], baseLineOfTarget(hunks, line));
      return true;
    },
    destroy,
  };
}
