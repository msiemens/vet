// The comment threads of a `ChangesetView`: which cards each section shows, where the form for a
// new thread is, and the height all of that adds. A thread card's height depends on its text, so it
// is measured once drawn: a section counts every card at the height it last had (`counted`) and
// grows by the difference when a card reports another. The view only wires it to its sections.

import { type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import type { CommentAt } from "../../../generated/CommentAt";
import type { Thread } from "../../../generated/Thread";
import type { ThreadTarget } from "../../../generated/ThreadTarget";
import { lineEnd } from "./positions";
import type { Section, ThreadState } from "./section";
import type { ThreadSpace } from "./sectionLayout";
import { type ThreadActions, buildCard } from "./threadCard";
import { type CardData, type InlineCard, cardHeight, cardKey, heightKey, placeThreads } from "./threadPlacement";
import { type CardHost, setThreadCards, threadCards } from "./threadWidgets";

/** What the owner asks for; each resolves whether the request went through. `revision` is the view's. */
export type ThreadListeners = Omit<ThreadActions, "create" | "cancelDraft" | "toggle"> & {
  create(revision: number, target: ThreadTarget, body: string): Promise<boolean>;
  /** Where a comment on a part of the file at `path` attaches, and the target line its form sits below. */
  commentTarget(revision: number, path: string, at: CommentAt): Promise<{ target: ThreadTarget; line: number | null }>;
};

export const threadState = (blockEl: HTMLElement): ThreadState => ({
  placement: { file: [], inline: [] },
  inline: [],
  blockEl,
  blockCards: [],
  blockSignature: "",
  block: 0,
  counted: new Map(),
});

export interface ThreadHost {
  readonly revision: number;
  readonly sections: Section[];
  readonly line: number;
  /** Lays the section out again, counting its cards as `known` heights say. */
  estimate(s: Section, known: Map<string, number>): void;
  /** Sets a section's height; the view keeps the scroll position when it is above the viewport. */
  resize(s: Section, height: number): void;
}

/** A new thread's form, in the section of `path` (the view's name for the file). */
interface Draft {
  path: string;
  target: Extract<ThreadTarget, { type: "range" | "file" }>;
  /** For a range: the target line its form sits below, where the backend places it. */
  line?: number;
}

const sum = (heights: Iterable<number>) => [...heights].reduce((total, h) => total + h, 0);

export class ThreadLayer {
  private threads: Thread[] = [];
  private draft?: Draft;
  private opened = new Set<number>();
  /** Measured heights by `heightKey`; they estimate a card of an unmounted section. */
  private heights = new Map<string, number>();
  /** The cards on screen, so a measured size finds its section. */
  private drawn = new WeakMap<Element, { card: CardData; section: Section }>();
  private sizes = new ResizeObserver((entries) => entries.forEach((e) => this.onCardSize(e)));
  private destroyed = false;
  private actions: ThreadActions;

  constructor(
    private host: ThreadHost,
    private listeners: ThreadListeners,
  ) {
    this.actions = {
      create: async (target, body) => {
        const sent = await listeners.create(host.revision, target, body);
        if (sent) this.setDraft(undefined);
        return sent;
      },
      reply: listeners.reply,
      setStatus: listeners.setStatus,
      remove: listeners.remove,
      editMessage: listeners.editMessage,
      removeMessage: listeners.removeMessage,
      keepEdit: listeners.keepEdit,
      platformName: listeners.platformName,
      confirm: listeners.confirm,
      cancelDraft: () => this.setDraft(undefined),
      toggle: (thread) => {
        if (!this.opened.delete(thread)) this.opened.add(thread);
        this.show();
      },
    };
  }

  destroy() {
    this.destroyed = true;
    this.sizes.disconnect();
  }

  extension(s: Section): Extension {
    return threadCards(this.hostFor(s));
  }

  /** Gives a freshly mounted editor its cards. */
  mounted(s: Section) {
    if (s.inline.length) s.handle?.dispatchAll(setThreadCards.of(s.inline));
  }

  /** What a section's cards add to its layout; a card counts at its `known` height, else as measured before, else as estimated. */
  space(s: Section, known: Map<string, number>): ThreadSpace {
    const height = (card: CardData) => known.get(cardKey(card)) ?? this.heightOf(card);
    return {
      block: sum(s.blockCards.map(height)),
      inline: s.inline.map((card) => ({ line: card.line, height: height(card) })),
    };
  }

  /**
   * Records what `space` put into the section's height. A card below a collapsed region counts only
   * once the region is expanded and the card drawn, so `known` carries those over.
   */
  count(s: Section, space: ThreadSpace, drawn: boolean[], known: Map<string, number>) {
    s.block = space.block;
    s.counted = new Map(s.blockCards.map((card) => [cardKey(card), this.known(card, known)]));
    s.inline.forEach((card, i) => {
      if (drawn[i] || known.has(cardKey(card))) s.counted.set(cardKey(card), space.inline[i].height);
    });
    s.el.style.setProperty("--vet-threads", `${s.block}px`);
  }

  private known(card: CardData, known: Map<string, number>) {
    return known.get(cardKey(card)) ?? this.heightOf(card);
  }

  private heightOf(card: CardData): number {
    return this.heights.get(heightKey(card)) ?? cardHeight(card, { line: this.host.line });
  }

  /** Threads of another revision are not shown; the view of that revision gets them. */
  setThreads(threads: Thread[], revision: number) {
    if (revision !== this.host.revision) return;
    this.threads = threads;
    this.show();
  }

  /** The changeset's own threads are not about code and live in the Comments tool window only. */
  private show() {
    const byPath = new Map<string, Thread[]>();
    for (const thread of this.threads) {
      if (thread.kind !== "changeset" && thread.path !== null)
        byPath.set(thread.path, [...(byPath.get(thread.path) ?? []), thread]);
    }
    const open = (thread: Thread) => this.opened.has(thread.id);
    for (const s of this.host.sections) {
      const { file } = s;
      const placement = placeThreads(byPath.get(file.path) ?? []);
      const inline: InlineCard[] = placement.inline.map((c) => ({ ...c, open: open(c.thread) }));
      const blockCards: CardData[] = placement.file.map((thread) => ({ thread, open: open(thread) }));
      const drafted = this.draft?.path === file.path ? this.draft : undefined;
      const draft = drafted?.target;
      if (draft?.type === "file") blockCards.push({ draft });
      else if (draft?.type === "range") {
        // The sort is stable: the form lands below the threads already on that line.
        inline.push({ draft, line: drafted?.line ?? 0 });
        inline.sort((a, b) => a.line - b.line);
      }
      if (!inline.length && !blockCards.length && !s.inline.length && !s.counted.size) {
        s.placement = placement;
        continue;
      }
      const old = s.counted;
      const before = sum(old.values());
      s.placement = placement;
      s.inline = inline;
      s.blockCards = blockCards;
      this.host.estimate(s, old);
      this.drawBlock(s);
      s.handle?.dispatchAll(setThreadCards.of(inline));
      this.host.resize(s, s.height + sum(s.counted.values()) - before);
    }
  }

  /** A form being typed in stays as it is: a block is only redrawn when its cards changed. */
  private drawBlock(s: Section) {
    const signature = JSON.stringify(s.blockCards);
    if (signature === s.blockSignature) return;
    s.blockSignature = signature;
    for (const child of s.blockEl.children) this.sizes.unobserve(child);
    s.blockEl.replaceChildren(...s.blockCards.map((card) => this.draw(card, s)));
  }

  /** Editor widgets and blocks build and measure their cards the same way. */
  private draw(card: CardData, section: Section): HTMLElement {
    const el = buildCard(card, this.actions);
    this.drawn.set(el, { card, section });
    this.sizes.observe(el);
    return el;
  }

  private hostFor(s: Section): CardHost {
    return {
      create: (card) => this.draw(card, s),
      dispose: (el) => this.sizes.unobserve(el),
      height: (card) => this.heightOf(card),
    };
  }

  private onCardSize(entry: ResizeObserverEntry) {
    const drawn = this.drawn.get(entry.target);
    const height = entry.borderBoxSize[0]?.blockSize ?? 0;
    // Zero is a card taken out of the document.
    if (!drawn || this.destroyed || height === 0) return;
    const key = cardKey(drawn.card);
    this.heights.set(heightKey(drawn.card), height);
    const s = drawn.section;
    const delta = height - (s.counted.get(key) ?? 0);
    if (Math.abs(delta) < 0.5) return;
    s.counted.set(key, height);
    if (s.blockCards.some((card) => cardKey(card) === key)) {
      s.block += delta;
      s.el.style.setProperty("--vet-threads", `${s.block}px`);
    }
    this.host.resize(s, s.height + delta);
  }

  private setDraft(draft: Draft | undefined) {
    this.draft = draft;
    this.show();
    if (draft?.target.type !== "range") return;
    // The form sits below the range, which may be far from the viewer; an editor draws only what is near the viewport.
    const s = this.host.sections.find((s) => s.file.path === draft.path);
    const form = s?.inline.find((card) => card.draft);
    const doc = s?.handle?.view.state.doc;
    if (form && doc && s.handle)
      s.handle.view.dispatch({
        effects: EditorView.scrollIntoView(lineEnd(doc, form.line), { y: "nearest", yMargin: 160 }),
      });
  }

  commentOnFile(s: Section) {
    this.commentOn(s, { type: "file" });
  }

  commentOnHunk(s: Section, index: number) {
    this.commentOn(s, { type: "hunk", index });
  }

  /** A stop of the file's hunk navigation: a change card comments on the file, a hunk on itself. */
  commentOnStop(s: Section, index: number) {
    if (index < s.cards.length) this.commentOnFile(s);
    else this.commentOnHunk(s, index - s.cards.length);
  }

  /**
   * Where a thread is shown: the index of its section and the line of its card, if it has one in
   * the editor; none if it is not in this view, as the changeset's own never are.
   */
  find(thread: Thread): { index: number; line?: number } | undefined {
    if (thread.kind === "changeset") return undefined;
    const index = thread.path === null ? -1 : this.host.sections.findIndex((s) => s.file.path === thread.path);
    if (index < 0) return undefined;
    const card = this.host.sections[index].placement.inline.find((c) => c.thread.id === thread.id);
    return { index, line: card?.line };
  }

  /** Opens the form for a comment where the backend says it attaches. */
  commentOn({ file }: Section, at: CommentAt) {
    // A newer revision replaces the view, and the request fails then.
    void this.listeners.commentTarget(this.host.revision, file.path, at).then(
      ({ target, line }) => {
        if (!this.destroyed && target.type !== "changeset")
          this.setDraft({ path: file.path, target, line: line ?? undefined });
      },
      () => undefined,
    );
  }
}
