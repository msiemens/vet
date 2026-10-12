// The comment threads of a file's text view (`TextView`): a card below the target line of each range
// thread of the file, as a unified diff shows them, and the form for a new one. A thread on the base
// side sits at the target line the backend maps it to. The editor measures its own cards, so unlike
// `ThreadLayer` there is no height model to keep.

import { type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import type { CommentAt } from "../../../generated/CommentAt";
import type { Thread } from "../../../generated/Thread";
import { lineEnd } from "./positions";
import { buildCard } from "./threadCard";
import { type ThreadListeners, threadActions } from "./threadLayer";
import { type FileViewDraft, type InlineCard, cardHeight, fileViewCards } from "./threadPlacement";
import { setThreadCards, threadCards } from "./threadWidgets";

export class TextThreads {
  private threads: Thread[] = [];
  private draft?: FileViewDraft;
  private opened = new Set<number>();
  private cards: InlineCard[] = [];
  private view?: EditorView;
  private destroyed = false;
  private actions;

  constructor(
    private path: string,
    /** The revision of the text on view. */
    private revision: number,
    private listeners: ThreadListeners,
    /** The height of a text row in px, which estimates a card until the editor measures it. */
    private line: number,
  ) {
    this.actions = threadActions(listeners, revision, {
      cancelDraft: () => this.setDraft(undefined),
      toggle: (thread) => {
        if (!this.opened.delete(thread)) this.opened.add(thread);
        this.show();
      },
    });
  }

  /** The editor's cards extension. */
  extension(): Extension {
    return threadCards({
      create: (card) => buildCard(card, this.actions),
      dispose: () => undefined,
      height: (card) => cardHeight(card, { line: this.line }),
    });
  }

  /** The editor to draw the cards in; it shows those there are. */
  attach(view: EditorView) {
    this.view = view;
    view.dispatch({ effects: setThreadCards.of(this.cards) });
  }

  destroy() {
    this.destroyed = true;
    this.view = undefined;
  }

  /** Threads of another revision are not shown; the view of that revision gets them. */
  setThreads(threads: Thread[], revision: number) {
    if (revision !== this.revision) return;
    this.threads = threads;
    this.show();
  }

  private show() {
    this.cards = fileViewCards(this.threads, this.path, (thread) => this.opened.has(thread), this.draft);
    this.view?.dispatch({ effects: setThreadCards.of(this.cards) });
  }

  /** The 0-based line the thread's card sits below; none if the file's text does not show it. */
  lineOf(thread: Thread): number | undefined {
    return this.cards.find((c) => c.thread?.id === thread.id)?.line;
  }

  /** Opens the form for a comment where the backend says it attaches. */
  commentOn(at: CommentAt) {
    // A newer revision replaces the view, and the request fails then.
    void this.listeners.commentTarget(this.revision, this.path, at).then(
      ({ target, line }) => {
        if (!this.destroyed && target.type === "range" && line !== null) this.setDraft({ target, line });
      },
      () => undefined,
    );
  }

  private setDraft(draft: FileViewDraft | undefined) {
    this.draft = draft;
    this.show();
    const view = this.view;
    if (draft && view)
      view.dispatch({
        effects: EditorView.scrollIntoView(lineEnd(view.state.doc, draft.line), { y: "nearest", yMargin: 160 }),
      });
  }
}
