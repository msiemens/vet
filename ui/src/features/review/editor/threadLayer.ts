// The comment threads of a `ChangesetView`: which cards each section shows, where the form for a
// new thread is, and the height all of that adds. A thread card's height depends on its text, so it
// is measured once drawn: a section counts every card at the height it last had (`counted`) and
// grows by the difference when a card reports another. The view only wires it to its sections.

import { type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import type { FileChange } from "../../../generated/FileChange";
import type { Thread } from "../../../generated/Thread";
import type { ThreadTarget } from "../../../generated/ThreadTarget";
import type { Handle } from "./mount";
import { lineEnd, targetLine } from "./positions";
import type { ThreadSpace } from "./sectionLayout";
import { type ThreadActions, buildCard } from "./threadCard";
import {
  type CardData,
  type InlineCard,
  type Placement,
  cardHeight,
  cardKey,
  heightKey,
  placeThreads,
} from "./threadPlacement";
import { type CardHost, setThreadCards, threadCards } from "./threadWidgets";

/** What the owner asks for; each resolves whether the request went through. `revision` is the view's. */
export type ThreadListeners = Omit<ThreadActions, "create" | "cancelDraft" | "toggle"> & {
  create(revision: number, target: ThreadTarget, body: string): Promise<boolean>;
};

/** The part of a section the layer keeps. */
export interface ThreadState {
  placement: Placement;
  /** Cards below lines: the placed threads and the form for a new one. */
  inline: InlineCard[];
  /** Cards of the file's own threads, between the change cards and the first hunk. */
  blockEl: HTMLElement;
  blockCards: CardData[];
  /** What `blockEl` was built from. */
  blockSignature: string;
  /** Height of the block, which `--vet-threads` reports to the body's layout. */
  block: number;
  /** The height each card adds to the section's height, by `cardKey`. */
  counted: Map<string, number>;
}

export const threadState = (blockEl: HTMLElement): ThreadState => ({
  placement: { file: [], inline: [] },
  inline: [],
  blockEl,
  blockCards: [],
  blockSignature: "",
  block: 0,
  counted: new Map(),
});

export interface ThreadSection extends ThreadState {
  file: FileChange;
  el: HTMLElement;
  top: number;
  height: number;
  handle?: Handle;
}

export interface ThreadHost {
  readonly revision: number;
  readonly sections: ThreadSection[];
  readonly root: HTMLElement;
  readonly line: number;
  /** Lays the section out again, counting its cards as `known` heights say. */
  estimate(s: ThreadSection, known: Map<string, number>): void;
  /** Sets a section's height; the view keeps the scroll position when it is above the viewport. */
  resize(s: ThreadSection, height: number): void;
}

/** A new thread's form, in the section of `path` (the view's name for the file). */
interface Draft {
  path: string;
  target: Extract<ThreadTarget, { type: "range" | "file" }>;
}

const sum = (heights: Iterable<number>) => [...heights].reduce((total, h) => total + h, 0);

export class ThreadLayer {
  private threads: Thread[] = [];
  private draft?: Draft;
  private opened = new Set<number>();
  /** Measured heights by `heightKey`; they estimate a card of an unmounted section. */
  private heights = new Map<string, number>();
  /** The cards on screen, so a measured size finds its section (none: the lead). */
  private drawn = new WeakMap<Element, { card: CardData; section?: ThreadSection }>();
  private sizes = new ResizeObserver((entries) => entries.forEach((e) => this.onCardSize(e)));
  private destroyed = false;
  /** Cards above the first section, for the changeset's own threads: only the Changes stream has them. */
  readonly lead?: HTMLElement;
  private leadSignature = "";
  private leadCounted = new Map<string, number>();
  leadHeight = 0;
  private actions: ThreadActions;

  constructor(
    private host: ThreadHost,
    listeners: ThreadListeners,
    withLead: boolean,
  ) {
    if (withLead) {
      this.lead = document.createElement("div");
      this.lead.className = "vet-thread-block vet-lead";
    }
    this.actions = {
      create: async (target, body) => {
        const sent = await listeners.create(host.revision, target, body);
        if (sent) this.setDraft(undefined);
        return sent;
      },
      reply: listeners.reply,
      setStatus: listeners.setStatus,
      remove: listeners.remove,
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

  extension(s: ThreadSection): Extension {
    return threadCards(this.hostFor(s));
  }

  /** Gives a freshly mounted editor its cards. */
  mounted(s: ThreadSection) {
    if (s.inline.length) s.handle?.dispatchAll(setThreadCards.of(s.inline));
  }

  /** What a section's cards add to its layout; a card counts at its `known` height, else as measured before, else as estimated. */
  space(s: ThreadSection, known: Map<string, number>): ThreadSpace {
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
  count(s: ThreadSection, space: ThreadSpace, drawn: boolean[], known: Map<string, number>) {
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

  private show() {
    const byPath = new Map<string, Thread[]>();
    const changeset: Thread[] = [];
    for (const thread of this.threads) {
      if (thread.kind === "changeset") changeset.push(thread);
      else if (thread.path !== null) byPath.set(thread.path, [...(byPath.get(thread.path) ?? []), thread]);
    }
    const open = (thread: Thread) => this.opened.has(thread.id);
    for (const s of this.host.sections) {
      const { file } = s;
      const placement = placeThreads(byPath.get(file.path) ?? []);
      const inline: InlineCard[] = placement.inline.map((c) => ({ ...c, open: open(c.thread) }));
      const blockCards: CardData[] = placement.file.map((thread) => ({ thread, open: open(thread) }));
      const draft = this.draft?.path === file.path ? this.draft.target : undefined;
      if (draft?.type === "file") blockCards.push({ draft });
      else if (draft?.type === "range") {
        const end = targetLine(file.hunks, draft.side, draft.start + draft.len - 1);
        // The sort is stable: the form lands below the threads already on that line.
        inline.push({ draft, line: Math.min(end, Math.max(0, file.targetLines - 1)) });
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
    if (this.lead)
      this.drawLead(
        this.lead,
        changeset.map((thread) => ({ thread, open: open(thread) })),
      );
  }

  /** A form being typed in stays as it is: a block is only redrawn when its cards changed. */
  private drawBlock(s: ThreadSection) {
    const signature = JSON.stringify(s.blockCards);
    if (signature === s.blockSignature) return;
    s.blockSignature = signature;
    for (const child of s.blockEl.children) this.sizes.unobserve(child);
    s.blockEl.replaceChildren(...s.blockCards.map((card) => this.draw(card, s)));
  }

  private drawLead(lead: HTMLElement, cards: CardData[]) {
    const signature = JSON.stringify(cards);
    if (signature === this.leadSignature) return;
    this.leadSignature = signature;
    for (const child of lead.children) this.sizes.unobserve(child);
    lead.replaceChildren(...cards.map((card) => this.draw(card)));
    this.leadCounted = new Map(cards.map((card) => [cardKey(card), this.heightOf(card)]));
    this.moveLead(sum(this.leadCounted.values()) - this.leadHeight);
  }

  /** The sections below follow the lead's height, and so does the viewer's place in them. */
  private moveLead(delta: number) {
    if (Math.abs(delta) < 0.5) return;
    this.leadHeight += delta;
    for (const s of this.host.sections) s.top += delta;
    if (this.host.root.scrollTop > 0) this.host.root.scrollTop += delta;
  }

  /** Editor widgets and blocks build and measure their cards the same way. */
  private draw(card: CardData, section?: ThreadSection): HTMLElement {
    const el = buildCard(card, this.actions);
    this.drawn.set(el, { card, section });
    this.sizes.observe(el);
    return el;
  }

  private hostFor(s: ThreadSection): CardHost {
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
    const counted = s ? s.counted : this.leadCounted;
    const delta = height - (counted.get(key) ?? 0);
    if (Math.abs(delta) < 0.5) return;
    counted.set(key, height);
    if (!s) this.moveLead(delta);
    else {
      if (s.blockCards.some((card) => cardKey(card) === key)) {
        s.block += delta;
        s.el.style.setProperty("--vet-threads", `${s.block}px`);
      }
      this.host.resize(s, s.height + delta);
    }
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

  commentOnFile(s: ThreadSection) {
    const { file } = s;
    const side = file.status === "deleted" ? "base" : "target";
    this.setDraft({
      path: file.path,
      target: { type: "file", side, path: side === "base" ? (file.oldPath ?? file.path) : file.path },
    });
  }

  /** A hunk's target lines, or its base lines for a pure deletion. */
  commentOnHunk(s: ThreadSection, hunk: number) {
    const h = s.file.hunks[hunk];
    if (h.targetLen > 0) this.commentOnLines(s, "target", h.targetStart, h.targetLen);
    else this.commentOnLines(s, "base", h.baseStart, h.baseLen);
  }

  /** `len` lines from line `start` (0-based) of a side, kept inside the file. */
  commentOnLines(s: ThreadSection, side: "base" | "target", start: number, len: number) {
    const { file } = s;
    const lines = side === "base" ? file.baseLines : file.targetLines;
    if (lines <= 0) return;
    start = Math.min(start, lines - 1);
    len = Math.max(1, Math.min(len, lines - start));
    const path = side === "base" ? (file.oldPath ?? file.path) : file.path;
    this.setDraft({ path: file.path, target: { type: "range", side, path, start, len } });
  }
}
