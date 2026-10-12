// A read-only view of one file's text in the target snapshot (the File view of a changed file, and
// Jump to Source): a CodeMirror editor with syntax highlighting, line numbers, the semantic layer
// (hover, go to definition, diagnostics) and, for a changed file, its changes drawn in the text
// (`fileChanges`), or a small placeholder when the file has no text to show.

import { EditorState, type StateField, Text } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import type { FileChange } from "../../../generated/FileChange";
import type { FileReview } from "../../../generated/FileReview";
import type { HunkState } from "../../../generated/HunkState";
import type { NoTextReason } from "../../../generated/NoTextReason";
import type { Thread } from "../../../generated/Thread";
import type { TextSpan } from "../../../generated/TextSpan";
import { errorText } from "../../../lib/errorText";
import { type Highlights, decodeHighlights } from "../../../lib/highlights";
import type { SoftWrap } from "../../../lib/softWrap";
import { iconSvg } from "../../../lib/ui/icons";
import { type MarkLayout, type ScrollMark, Scrollbar } from "../../../lib/ui/scrollbar";
import { currentLine } from "./currentLine";
import { closeChangeCard, showChangeCard } from "./fileCard";
import { type EditorSelection, fileViewComment, hunkComment } from "./commentAnchor";
import {
  type ChangesSource,
  type ChangesState,
  changesField,
  fileChanges,
  setCurrentHunk,
  setPeek,
  setStates,
} from "./fileChanges";
import { hunkAtLine, oldLinesOf, stepHunk } from "./fileLayout";
import { type FileSource, StaleRevision } from "./fileSource";
import { highlightExtension } from "./highlightExtension";
import { hunkToolbar } from "./hunkToolbar";
import type { Loaded } from "./mount";
import { lineOf, lineStart } from "./positions";
import { markKind } from "./scrollMarks";
import { type SemanticHooks, revealSpan, semanticExtensions, showDiagnostics } from "./semantic";
import { readMetrics } from "./sectionHeights";
import { softWrap, wraps } from "./softWrap";
import { TextThreads } from "./textThreads";
import type { ThreadListeners } from "./threadLayer";

/** What the owner does for the file's hunks. */
export interface TextActions {
  toggleSeen(hunk: number): void;
  toggleFlag(hunk: number): void;
}

/** The threads a text shows: those of this revision, and what its cards ask of the owner. */
export interface TextThreadsInit {
  revision: number;
  listeners: ThreadListeners;
}

const NO_TEXT: Record<NoTextReason, string> = {
  binary: "Binary file: there is no text to show.",
  unavailable: "The content of this file is not available: it is over the size limit, or it could not be read.",
  notAFile: "Not a regular file: a symlink or a submodule has no text.",
};

/** What a changed file's text draws its changes from. */
interface Drawn {
  source: ChangesSource;
  field: StateField<ChangesState>;
}

export class TextView {
  private view?: EditorView;
  private destroyed = false;
  /** The 0-based line to show once the text has arrived. */
  private line?: number;
  /** The span to show once the text has arrived. */
  private span?: TextSpan;
  /** The hunk to go to once the text has arrived. */
  private hunk?: number;
  /** The review state of each of the file's hunks, as the backend last sent it. */
  private states: HunkState[];
  /** Which of the file's hunks are flagged for later review. */
  private flags: boolean[];
  private drawn?: Drawn;
  private scrollbar?: Scrollbar;
  /** The reviewer was in the editor, or navigation put its caret on a line: comments go on its selection. */
  private entered = false;
  private cards: TextThreads;

  /**
   * Fills `root` with the file's text from `source`. A reply of another revision is dropped:
   * the owner builds a new view when the manifest moves.
   */
  constructor(
    private root: HTMLElement,
    private path: string,
    private source: FileSource,
    private hooks: SemanticHooks,
    /** Which files wrap their long lines; a change builds the view anew. */
    private wrap: SoftWrap,
    threads: TextThreadsInit,
    /** The changeset's change to the file, if it changes it: its hunks are drawn, and the owner acts on them. */
    private changes?: { file: FileChange; actions: TextActions },
  ) {
    this.states = changes?.file.review.hunks ?? [];
    this.flags = changes?.file.review.flagged ?? [];
    root.replaceChildren();
    root.classList.add("vet-text");
    // Thread cards size themselves to the view's width (`cqw`), not to the width of the longest line.
    root.style.containerType = "inline-size";
    this.cards = new TextThreads(path, threads.revision, threads.listeners, readMetrics(root).line);
    const fail = (error: unknown) => {
      if (!this.destroyed && !(error instanceof StaleRevision)) this.placeholder(errorText(error));
    };
    if (changes?.file.hunks.length) {
      // The changes come with the text, and the old lines the removals and the cards show.
      source.contents(changes.file).then((loaded) => {
        if (!this.destroyed) this.mountEditor(loaded.target, loaded.targetHighlights, loaded);
      }, fail);
      return;
    }
    source.text(path).then((content) => {
      if (this.destroyed) return;
      if (content.type === "text") this.mountEditor(content.text, decodeHighlights(content.highlights));
      else this.placeholder(NO_TEXT[content.reason]);
    }, fail);
  }

  private mountEditor(text: string, highlights: Highlights, loaded?: Loaded) {
    const drawn = loaded && this.changes && this.draw(this.changes, loaded);
    this.drawn = drawn;
    this.view = new EditorView({
      parent: this.root,
      doc: text,
      extensions: [
        softWrap(wraps(this.path, this.wrap)),
        ...(drawn
          ? [
              ...fileChanges(drawn.source, drawn.field),
              hunkToolbar(drawn.source, drawn.field),
              EditorView.updateListener.of((u) => {
                if (
                  u.heightChanged ||
                  u.viewportChanged ||
                  u.docChanged ||
                  u.startState.field(drawn.field) !== u.state.field(drawn.field)
                )
                  this.scrollbar?.refresh();
              }),
            ]
          : [lineNumbers()]),
        currentLine(),
        this.cards.extension(),
        highlightExtension(highlights),
        // Read-only but focusable, so the caret and Mod+B work.
        EditorState.readOnly.of(true),
        EditorView.domEventHandlers({
          focus: () => {
            this.entered = true;
            return false;
          },
        }),
        semanticExtensions(this.path, "target", this.hooks),
      ],
    });
    const view = this.view;
    this.cards.attach(view);
    this.scrollbar = new Scrollbar(view.scrollDOM, { marks: drawn && (() => this.markLayout(view, drawn)) });
    this.root.after(this.scrollbar.dom);
    if (this.span) this.goToSpan(this.span);
    else if (this.hunk !== undefined) this.goToHunk(this.hunk);
    else if (this.line !== undefined) this.goToLine(this.line);
    this.refreshDiagnostics();
  }

  /** Where the hunks lie in the editor's content, for the marks in its scroll bar. */
  private markLayout(view: EditorView, { source, field }: Drawn): MarkLayout {
    const { states } = view.state.field(field);
    const { doc } = view.state;
    const marks = source.hunks.map((h, i): ScrollMark => {
      const first = view.lineBlockAt(lineStart(doc, h.targetStart));
      const removal = h.targetLen === 0;
      const last = removal ? first : view.lineBlockAt(lineStart(doc, h.targetStart + h.targetLen - 1));
      // Lines removed at the end lie below the last line.
      const top = removal && h.targetStart >= doc.lines ? first.bottom : first.top;
      return { top, height: removal ? 0 : last.bottom - first.top, kind: markKind(states[i], removal) };
    });
    return { total: view.contentHeight, marks };
  }

  private draw({ file, actions }: { file: FileChange; actions: TextActions }, loaded: Loaded): Drawn {
    const comment = (hunk: number) => this.cards.commentOn(hunkComment(file.hunks, hunk, this.selection()));
    const source: ChangesSource = {
      hunks: file.hunks,
      changes: loaded.changes,
      base: Text.of(loaded.base.split(/\r\n?|\n/)),
      actions: {
        toggleSeen: (hunk) => actions.toggleSeen(hunk),
        toggleFlag: (hunk) => actions.toggleFlag(hunk),
        comment,
      },
      card: (view, line) => this.showCard(view, line),
    };
    return { source, field: changesField(source, this.states, this.flags) };
  }

  /** The hunks' review states and flags changed. */
  applyReview({ hunks, flagged }: FileReview) {
    this.states = hunks;
    this.flags = flagged;
    if (this.drawn) this.view?.dispatch({ effects: setStates.of({ states: hunks, flags: flagged }) });
  }

  /** The file has hunks to step through. */
  get hasHunks(): boolean {
    return !!this.changes?.file.hunks.length;
  }

  /** The hunk the caret is in, or that was chosen last; -1 for none. */
  get current(): number {
    const { view, drawn } = this;
    return (view && drawn && view.state.field(drawn.field).current) ?? -1;
  }

  /** Puts the caret on the hunk's first line (below the wedge, for a removal, whose lines then show) and centres it. */
  goToHunk(hunk: number) {
    this.hunk = hunk;
    const { view, drawn } = this;
    if (!view || !drawn) return;
    const h = drawn.source.hunks[hunk];
    const pos = lineStart(view.state.doc, h.targetStart);
    this.entered = true;
    view.dispatch({
      selection: { anchor: pos },
      effects: [
        EditorView.scrollIntoView(pos, { y: "center" }),
        setCurrentHunk.of(hunk),
        ...(h.targetLen === 0 ? [setPeek.of({ hunk, open: true })] : []),
      ],
    });
  }

  /** Goes to the hunk `delta` stops from the current one, or from the caret when there is none. */
  step(delta: number) {
    const { view, drawn } = this;
    if (!view || !drawn) return;
    const caret = lineOf(view.state.doc, view.state.selection.main.head);
    const to = stepHunk(drawn.source.hunks, this.current, caret, delta);
    if (to >= 0) this.goToHunk(to);
  }

  /** Flips the current hunk; answers whether there was one. */
  toggleCurrent(): boolean {
    const hunk = this.current;
    if (hunk < 0 || !this.drawn) return false;
    this.drawn.source.actions.toggleSeen(hunk);
    return true;
  }

  /** Flips the flag of the current hunk; answers whether there was one. */
  toggleFlagCurrent(): boolean {
    const hunk = this.current;
    if (hunk < 0 || !this.drawn) return false;
    this.drawn.source.actions.toggleFlag(hunk);
    return true;
  }

  /** Marks the current hunk seen again if it is seen against an old base; answers whether there was such a hunk. */
  reconfirmCurrent(): boolean {
    const hunk = this.current;
    if (hunk < 0 || this.states[hunk] !== "seenAgainstOldBase" || !this.drawn) return false;
    this.drawn.source.actions.toggleSeen(hunk);
    return true;
  }

  /** The editor's selection once the reviewer was in it. */
  private selection(): EditorSelection | undefined {
    const { view } = this;
    if (!view || !this.entered) return undefined;
    const { from, to } = view.state.selection.main;
    return { side: "target", doc: view.state.doc, from, to };
  }

  /** Asks for a comment on the selection, the caret's line or the current hunk; answers whether there was somewhere to comment. */
  commentHere(): boolean {
    const { drawn } = this;
    const at = drawn && fileViewComment(drawn.source.hunks, this.current, this.selection());
    if (!at) return false;
    this.cards.commentOn(at);
    return true;
  }

  /** The threads were read again. */
  setThreads(threads: Thread[], revision: number) {
    this.cards.setThreads(threads, revision);
  }

  /** Puts the caret on the line the thread's card sits below (the first, for the file's own); answers whether the text shows the thread. */
  goToThread(thread: Thread): boolean {
    const line = this.cards.lineOf(thread);
    if (line === undefined) return false;
    this.goToLine(Math.max(0, line));
    return true;
  }

  /** Opens the change card of the caret's line; answers whether the line is a changed one. */
  showCaretCard(): boolean {
    const { view } = this;
    if (!view) return false;
    return this.showCard(view, lineOf(view.state.doc, view.state.selection.main.head));
  }

  private showCard(view: EditorView, line: number): boolean {
    const { drawn } = this;
    if (!drawn) return false;
    const { layout } = view.state.field(drawn.field);
    const hunk = hunkAtLine(layout, line);
    if (hunk === undefined) return false;
    view.dispatch({ effects: setCurrentHunk.of(hunk) });
    const block = view.lineBlockAt(lineStart(view.state.doc, line));
    const top = view.documentTop + block.top;
    showChangeCard(
      { left: view.contentDOM.getBoundingClientRect().left, top, bottom: top + block.height },
      {
        old: oldLinesOf(drawn.source.hunks[hunk], line, drawn.source.changes, drawn.source.base),
        state: this.states[hunk],
      },
    );
    return true;
  }

  /** Asks for the file's diagnostics again (the first time, or after the servers changed). */
  refreshDiagnostics() {
    const view = this.view;
    if (!view) return;
    void this.hooks.diagnostics(this.path).then((list) => {
      if (!this.destroyed) showDiagnostics(view, list);
    });
  }

  /** Puts the caret on a 0-based line and centres it. */
  goToLine(line: number) {
    this.line = line;
    this.entered = true;
    const view = this.view;
    if (!view) return;
    const from = lineStart(view.state.doc, line);
    view.dispatch({ selection: { anchor: from }, effects: EditorView.scrollIntoView(from, { y: "center" }) });
  }

  /** The 0-based line the caret is on; none until the text has arrived. */
  caretLine(): number | undefined {
    const view = this.view;
    return view && lineOf(view.state.doc, view.state.selection.main.head);
  }

  /** Selects and marks a span, and centres it. */
  goToSpan(span: TextSpan) {
    this.span = span;
    this.entered = true;
    if (this.view) revealSpan(this.view, span, true);
  }

  private placeholder(message: string) {
    const el = document.createElement("div");
    el.className = "vet-notext";
    el.innerHTML = iconSvg("file", 16, 1.6);
    el.append(message);
    this.root.replaceChildren(el);
  }

  destroy() {
    this.destroyed = true;
    this.cards.destroy();
    closeChangeCard();
    this.scrollbar?.destroy();
    this.view?.destroy();
    this.root.classList.remove("vet-text");
    this.root.style.containerType = "";
    this.root.replaceChildren();
  }
}
