// The semantic layer in an editor (SPEC §7): hover, go to definition (Mod+click and Mod+B, as in
// the JetBrains keymap) and diagnostics. The editor shows one side's text; `side` says which
// snapshot its positions are in (the target editor, or the base editor of a side-by-side view).
// Diagnostics exist for the target side only.

import { type Diagnostic as LintDiagnostic, lintGutter, setDiagnostics } from "@codemirror/lint";
import { type Extension, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  hoverTooltip,
  keymap,
  tooltips,
} from "@codemirror/view";
import type { Diagnostic } from "../../../generated/Diagnostic";
import type { Side } from "../../../generated/Side";
import type { TextSpan } from "../../../generated/TextSpan";
import { nextPaint } from "../../../lib/nextPaint";
import type { HoverText } from "../../../state/backend";
import { isMac } from "../../../state/host";
import { hoverContent } from "./hoverContent";
import { lineOf, lineStart, offsetAt } from "./positions";

/** What an editor asks of its owner, which knows the view's revision and what to do with an answer. */
export interface SemanticHooks {
  /** Markdown; its text is empty if there is nothing to show. */
  hover(side: Side, path: string, line: number, col: number): Promise<HoverText>;
  /** Goes to the definition; the owner opens the target. */
  definition(side: Side, path: string, line: number, col: number): void;
  /** The target-side diagnostics of the file, empty if there are none or the server is unavailable. */
  diagnostics(path: string): Promise<Diagnostic[]>;
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
    hover: (side, path, line, col) => source.hover(revision, side, path, line, col),
    definition: (side, path, line, col) => source.definition(revision, side, path, line, col),
    diagnostics: (path) => source.diagnostics(revision, path),
  };
}

/** How long the pointer rests on a position before the hover query is sent. */
export const HOVER_MS = 50;

/** A 0-based line and UTF-16 column of a side's text. */
export interface TextPosition {
  line: number;
  col: number;
}

/**
 * Asks for the hover text at a position of `side` and returns what builds its tooltip body, or
 * nothing when there is no text. `hoverShown` gets the time from the query to the body's paint
 * once the body has been built and put on screen.
 */
export async function queryHover(
  hooks: SemanticHooks,
  side: Side,
  path: string,
  at: TextPosition,
  hoverShown?: (ms: number) => void,
): Promise<(() => HTMLElement) | undefined> {
  const started = performance.now();
  const hover = await hooks.hover(side, path, at.line, at.col);
  if (!hover.text) return undefined;
  return () => {
    const dom = document.createElement("div");
    dom.className = "vet-hover";
    dom.append(hoverContent(hover));
    if (hoverShown) void nextPaint().then(() => hoverShown(performance.now() - started));
    return dom;
  };
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

/** Replaces the editor's diagnostics: gutter markers and underlines, with the message in a tooltip. */
export function showDiagnostics(view: EditorView, diagnostics: readonly Diagnostic[]) {
  const { doc } = view.state;
  const lint = diagnostics.map((d): LintDiagnostic => {
    const from = offsetAt(doc, d.span.start);
    return {
      from,
      to: Math.max(from, offsetAt(doc, d.span.end)),
      severity: d.severity,
      message: d.message,
      source: [d.source, d.code].filter(Boolean).join(" ") || undefined,
    };
  });
  view.dispatch(setDiagnostics(view.state, lint));
}

/**
 * Hover, Mod+click and Mod+B on `side` of `path`, plus the diagnostics gutter on the target side.
 * `hoverShown` reports the time from the start of a hover query to its tooltip's paint.
 */
export function semanticExtensions(
  path: string,
  side: Side,
  hooks: SemanticHooks,
  hoverShown?: (ms: number) => void,
): Extension[] {
  const at = (view: EditorView, pos: number): TextPosition => {
    const { doc } = view.state;
    const line = lineOf(doc, pos);
    return { line, col: pos - lineStart(doc, line) };
  };
  // `hoverTooltip` asks for the start of a block widget (a deleted chunk, a thread card) under the
  // pointer, and for the line start when the pointer is on the line numbers; neither is text to hover.
  const pointer = ViewPlugin.define((view) => {
    const state = {
      overText: false,
      move: (event: MouseEvent) => {
        state.overText = event.target instanceof Element && event.target.closest(".cm-line") !== null;
      },
      destroy: () => view.dom.removeEventListener("mousemove", state.move),
    };
    view.dom.addEventListener("mousemove", state.move);
    return state;
  });
  return [
    targetField,
    // A side by side pane clips what it contains; the tooltips of hover and diagnostics live in the body.
    tooltips({ parent: document.body }),
    pointer,
    hoverTooltip(
      async (view, pos) => {
        if (!view.plugin(pointer)?.overText) return null;
        const create = await queryHover(hooks, side, path, at(view, pos), hoverShown);
        return create ? { pos, above: true, create: () => ({ dom: create() }) } : null;
      },
      { hoverTime: HOVER_MS },
    ),
    EditorView.domEventHandlers({
      mousedown: (event, view) =>
        gotoOnClick(event, hooks, side, path, () => {
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
          return pos === null ? undefined : at(view, pos);
        }),
    }),
    keymap.of([
      {
        key: "Mod-b",
        run: (view) => {
          const { line, col } = at(view, view.state.selection.main.head);
          hooks.definition(side, path, line, col);
          return true;
        },
      },
    ]),
    ...(side === "target" ? [lintGutter()] : []),
  ];
}
