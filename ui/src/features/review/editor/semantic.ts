// The semantic layer in an editor (SPEC §7): hover, go to definition (Mod+click, and Mod+B as the
// `review.goToDefinition` command, as in the JetBrains keymap) and diagnostics. The editor shows one side's text; `side` says which
// snapshot its positions are in (the target editor, or the base editor of a side-by-side view).
// Diagnostics exist for the target side only.

import { type Diagnostic as LintDiagnostic, linter, lintGutter, setDiagnostics } from "@codemirror/lint";
import { type Extension, Facet, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, tooltips } from "@codemirror/view";
import type { Diagnostic } from "../../../generated/Diagnostic";
import type { Position } from "../../../generated/Position";
import type { Side } from "../../../generated/Side";
import type { TextSpan } from "../../../generated/TextSpan";
import type { HoverAnswer } from "../../../state/backend";
import { isMac } from "../../../state/host";
import type { CaretRect, PickerKind } from "../../../state/pickerTypes";
import { type Span, collapsedRegions, outsideHidden } from "./hiddenRanges";
import { hoverContent } from "./hoverContent";
import { type HoverAnchor, hoverPopups } from "./hoverDom";
import { wordRange } from "./hoverPopup";
import { lineOf, lineStart, offsetAt } from "./positions";

/** What an editor asks of its owner, which knows the view's revision and what to do with an answer. */
export interface SemanticHooks {
  /** Markdown; its text is empty if there is nothing to show or `signal` aborted the request. */
  hover(side: Side, path: string, line: number, col: number, signal?: AbortSignal): Promise<HoverAnswer>;
  /** Goes to the definition; the owner opens the target. */
  definition(side: Side, path: string, line: number, col: number): void;
  /** Finds the implementations or the usages of the symbol at the position; the owner goes to the place or lists them. */
  locations(kind: PickerKind, side: Side, path: string, at: TextPosition, subject: Subject): void;
  /** The target-side diagnostics of the file, empty if there are none or the server is unavailable. */
  diagnostics(path: string): Promise<Diagnostic[]>;
  /** The reviewer put the cursor at the position; the owner shows the symbol it is in. */
  cursor(side: Side, path: string, line: number, col: number): void;
}

/** The same, for a view that does not know its revision. */
export type SemanticSource = {
  [K in keyof SemanticHooks]: SemanticHooks[K] extends (...args: infer A) => infer R
    ? (revision: number, ...args: A) => R
    : never;
};

/** Fixes the revision of a view's requests: an answer to another revision is the backend's to reject. */
export function bindRevision(source: SemanticSource, revision: number): SemanticHooks {
  return {
    hover: (side, path, line, col, signal) => source.hover(revision, side, path, line, col, signal),
    definition: (side, path, line, col) => source.definition(revision, side, path, line, col),
    locations: (kind, side, path, at, subject) => source.locations(revision, kind, side, path, at, subject),
    diagnostics: (path) => source.diagnostics(revision, path),
    cursor: (side, path, line, col) => source.cursor(revision, side, path, line, col),
  };
}

/** A 0-based line and UTF-16 column of a side's text. */
export interface TextPosition {
  line: number;
  col: number;
}

/** What a list of places is about: the name at the caret, and where the caret is in the window. */
export interface Subject {
  name: string;
  anchor: CaretRect;
}

/**
 * Asks for the hover text at a position of `side` and returns what builds its tooltip body, or
 * nothing when there is nothing to show. `diagnostics` are those at the symbol, which the popup
 * shows with the text.
 */
export async function queryHover(
  hooks: SemanticHooks,
  side: Side,
  path: string,
  at: TextPosition,
  diagnostics: readonly Diagnostic[],
  signal?: AbortSignal,
): Promise<(() => HTMLElement) | undefined> {
  const hover = await hooks.hover(side, path, at.line, at.col, signal);
  // An aborted hover is an empty answer, which says nothing and is not an error.
  if (signal?.aborted) return undefined;
  return hoverContent(hover, diagnostics);
}

/** Mod is Cmd on macOS (Ctrl+click is the context menu there) and Ctrl elsewhere. */
const isGotoClick = (e: MouseEvent) => e.button === 0 && (isMac ? e.metaKey : e.ctrlKey);

/** Mod+click goes to the definition at the position `at` finds; answers whether the click was one and has been taken. */
export function gotoOnClick(
  event: MouseEvent,
  hooks: SemanticHooks,
  side: Side,
  path: string,
  at: () => TextPosition | undefined,
): boolean {
  if (!isGotoClick(event)) return false;
  const position = at();
  if (!position) return false;
  event.preventDefault();
  hooks.definition(side, path, position.line, position.col);
  return true;
}

/** How long a reached definition stays marked; the selection alone is not drawn in an unfocused or read-only editor. */
const MARK_MS = 1500;

const setTarget = StateEffect.define<{ from: number; to: number } | null>();
const targetMark = Decoration.mark({ class: "vet-goto" });
const targetField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, tr) {
    marks = marks.map(tr.changes);
    for (const e of tr.effects)
      if (e.is(setTarget))
        marks =
          e.value && e.value.to > e.value.from
            ? Decoration.set([targetMark.range(e.value.from, e.value.to)])
            : Decoration.none;
    return marks;
  },
  provide: (f) => EditorView.decorations.from(f),
});

/** Scrolls the span to the middle and marks it; `select` also selects it and focuses the editor (an editable or text view). */
export function revealSpan(view: EditorView, span: TextSpan, select: boolean) {
  const { doc } = view.state;
  const from = offsetAt(doc, span.start);
  const to = Math.max(from, offsetAt(doc, span.end));
  if (select) view.focus();
  view.dispatch({
    selection: select ? { anchor: from, head: to } : undefined,
    effects: [EditorView.scrollIntoView(from, { y: "center" }), setTarget.of({ from, to })],
  });
  setTimeout(() => {
    if (view.dom.isConnected) view.dispatch({ effects: setTarget.of(null) });
  }, MARK_MS);
}

const setDiagnosticList = StateEffect.define<readonly Diagnostic[]>();
/** The diagnostics the server gave, whole: the ones drawn are those outside the collapsed regions of the moment. */
const diagnosticList = StateField.define<readonly Diagnostic[]>({
  create: () => [],
  update(list, tr) {
    for (const e of tr.effects) if (e.is(setDiagnosticList)) return e.value;
    return list;
  },
});

/** The regions hidden behind an "N unchanged lines" bar. */
export function hiddenRegions(view: EditorView): Span[] {
  const sets = view.state
    .facet(EditorView.decorations)
    .map((source) => (typeof source === "function" ? source(view) : source));
  return collapsedRegions(sets, view.state.doc.length);
}

/** Draws the stored diagnostics: gutter markers and underlines, with the message in a tooltip, but nothing in or on a collapsed region. */
function drawDiagnostics(view: EditorView, stored: readonly Diagnostic[], store: boolean) {
  const { doc } = view.state;
  const hidden = hiddenRegions(view);
  const lint = stored.flatMap((d): LintDiagnostic[] => {
    const from = offsetAt(doc, d.span.start);
    const span = { from, to: Math.max(from, offsetAt(doc, d.span.end)) };
    return outsideHidden(span, hidden).map((piece) => ({
      ...piece,
      severity: d.severity,
      message: d.message,
      source: [d.source, d.code].filter(Boolean).join(" ") || undefined,
    }));
  });
  view.dispatch(setDiagnostics(view.state, lint), store ? { effects: setDiagnosticList.of(stored) } : {});
}

/** Replaces the editor's diagnostics. */
export function showDiagnostics(view: EditorView, diagnostics: readonly Diagnostic[]) {
  drawDiagnostics(view, diagnostics, true);
}

/** Draws the diagnostics again when a region collapses or expands. */
const diagnosticsFollowCollapse = ViewPlugin.fromClass(
  class {
    private key: string;

    constructor(view: EditorView) {
      this.key = this.keyOf(view);
    }

    private keyOf(view: EditorView) {
      return hiddenRegions(view)
        .map((r) => `${r.from}-${r.to}`)
        .join();
    }

    update(u: ViewUpdate) {
      if (!u.transactions.length) return;
      const key = this.keyOf(u.view);
      if (key === this.key) return;
      this.key = key;
      u.view.requestMeasure({
        read: () => null,
        write: (_, view) => drawDiagnostics(view, view.state.field(diagnosticList), false),
        key: this,
      });
    }
  },
);

/** What an editor shows and who answers for it, so commands can act on the editor that has the focus. */
interface SemanticTarget {
  side: Side;
  path: string;
  hooks: SemanticHooks;
}
const semanticTarget = Facet.define<SemanticTarget, SemanticTarget | undefined>({ combine: (values) => values[0] });

const at = (view: EditorView, pos: number): TextPosition => {
  const { doc } = view.state;
  const line = lineOf(doc, pos);
  return { line, col: pos - lineStart(doc, line) };
};

/**
 * The word of the document under the pointer, if it is on text: a hover over a block widget (a
 * deleted chunk, a thread card), the line numbers or blank space is none.
 */
function wordUnderPointer(
  view: EditorView,
  event: MouseEvent,
): (HoverAnchor & { at: TextPosition; endCol: number }) | undefined {
  if (!(event.target instanceof Element && event.target.closest(".cm-line"))) return undefined;
  const { clientX: x, clientY: y } = event;
  const pos = view.posAtCoords({ x, y });
  const edge = pos === null ? null : view.coordsAtPos(pos);
  if (pos === null || !edge) return undefined;
  const hovered = Math.max(x < edge.left ? pos - 1 : pos, 0);
  const line = view.state.doc.lineAt(hovered);
  const word = wordRange(line.text, hovered - line.from);
  const start = word && view.coordsAtPos(line.from + word.from, 1);
  const end = word && view.coordsAtPos(line.from + word.to, -1);
  if (!word || !start || !end) return undefined;
  const rect = {
    left: Math.min(start.left, end.left),
    right: Math.max(start.right, end.right),
    top: Math.min(start.top, end.top),
    bottom: Math.max(start.bottom, end.bottom),
  };
  if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return undefined;
  const lineIndex = line.number - 1;
  return {
    key: `${lineIndex}:${word.from}-${word.to}`,
    rect,
    at: { line: lineIndex, col: word.from },
    endCol: word.to,
  };
}

const before = (a: Position, b: Position) => a.line < b.line || (a.line === b.line && a.col < b.col);

/** The diagnostics the editor shows that touch the word `at`..`endCol`. */
function diagnosticsAt(view: EditorView, at: TextPosition, endCol: number): readonly Diagnostic[] {
  const word = { start: at, end: { line: at.line, col: endCol } };
  return (view.state.field(diagnosticList, false) ?? []).filter(
    (d) => before(d.span.start, word.end) && before(word.start, d.span.end),
  );
}

/** Finds the places of the kind for the word at the caret, which the popup names and opens below. */
function locationsAtCaret(view: EditorView, target: SemanticTarget, kind: PickerKind) {
  const head = view.state.selection.main.head;
  const line = view.state.doc.lineAt(head);
  // A caret just after a name is still on it.
  const col = head - line.from;
  const word = wordRange(line.text, col) ?? wordRange(line.text, col - 1);
  const name = word ? line.text.slice(word.from, word.to) : "";
  const position = at(view, head);
  if (word) position.col = word.from;
  const edge = view.coordsAtPos(head);
  const anchor = edge ? { left: edge.left, top: edge.top, bottom: edge.bottom } : { left: 0, top: 0, bottom: 0 };
  target.hooks.locations(kind, target.side, target.path, position, { name, anchor });
}

/** The editor with the focus, if `active` (the document's active element) is one with the semantic layer. */
export function focusedEditor(active: Element | null) {
  if (!(active instanceof HTMLElement) || !active.classList.contains("cm-content")) return undefined;
  const view = EditorView.findFromDOM(active);
  const target = view?.state.facet(semanticTarget);
  if (!view || !target) return undefined;
  return {
    side: target.side,
    path: target.path,
    selection: !view.state.selection.main.empty,
    selectedText() {
      const { from, to } = view.state.selection.main;
      return view.state.sliceDoc(from, to);
    },
    /** Goes to the definition of the symbol at the caret. */
    goToDefinition() {
      const { line, col } = at(view, view.state.selection.main.head);
      target.hooks.definition(target.side, target.path, line, col);
    },
    goToImplementation() {
      locationsAtCaret(view, target, "implementations");
    },
    showUsages() {
      locationsAtCaret(view, target, "usages");
    },
  };
}

/** Tells the owner where the reviewer puts the cursor; a selection made by code (a reveal) is not the reviewer's. */
function cursorMoves(path: string, side: Side, hooks: SemanticHooks): Extension {
  return EditorView.updateListener.of((u) => {
    const moved = u.transactions.some(
      (tr) => tr.selection && ["select", "input", "delete", "move"].some((event) => tr.isUserEvent(event)),
    );
    if (!moved) return;
    const { line, col } = at(u.view, u.state.selection.main.head);
    hooks.cursor(side, path, line, col);
  });
}

/** Hover, Mod+click and the cursor on `side` of `path`, plus the diagnostics gutter on the target side. */
export function semanticExtensions(path: string, side: Side, hooks: SemanticHooks): Extension[] {
  return [
    targetField,
    cursorMoves(path, side, hooks),
    // A side by side pane clips what it contains; the tooltips of diagnostics live in the body.
    tooltips({ parent: document.body }),
    hoverPopups(wordUnderPointer, (anchor, signal, view) =>
      queryHover(hooks, side, path, anchor.at, diagnosticsAt(view, anchor.at, anchor.endCol), signal),
    ),
    EditorView.domEventHandlers({
      mousedown: (event, view) =>
        gotoOnClick(event, hooks, side, path, () => {
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
          return pos === null ? undefined : at(view, pos);
        }),
    }),
    semanticTarget.of({ side, path, hooks }),
    // The hover popup shows the diagnostics at its symbol, so the squiggles have no tooltip of their own.
    ...(side === "target"
      ? [lintGutter(), linter(null, { tooltipFilter: () => [] }), diagnosticList, diagnosticsFollowCollapse]
      : []),
  ];
}
