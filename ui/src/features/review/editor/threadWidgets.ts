// Thread cards inside a file's editor: block widgets below the line they are anchored to.
// The view feeds them with `setThreadCards`; building and measuring the cards is the host's.

import { type Extension, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, WidgetType } from "@codemirror/view";
import type { CardData, InlineCard } from "./threadPlacement";

/** What the owner of the editor provides for its cards. */
export interface CardHost {
  create(data: CardData): HTMLElement;
  /** The card left the editor. */
  dispose(el: HTMLElement): void;
  /** Expected height until the card is measured: CodeMirror lays out what is off screen by it. */
  height(data: CardData): number;
}

/** Replaces the cards shown. */
export const setThreadCards = StateEffect.define<InlineCard[]>();

class CardWidget extends WidgetType {
  private readonly key: string;

  constructor(
    readonly data: CardData,
    readonly host: CardHost,
  ) {
    super();
    this.key = JSON.stringify(data);
  }

  eq(other: CardWidget): boolean {
    return other.key === this.key;
  }

  get estimatedHeight(): number {
    return this.host.height(this.data);
  }

  toDOM(): HTMLElement {
    return this.host.create(this.data);
  }

  destroy(dom: HTMLElement): void {
    this.host.dispose(dom);
  }

  /** The card handles its own pointer and keys; the editor must not act on them. */
  ignoreEvent(): boolean {
    return true;
  }
}

/** A card spans the view from its left edge, so it reaches back over the gutters (`--vet-card-inset`). */
const gutterInset = ViewPlugin.define((view) => {
  const measure = () =>
    view.requestMeasure({
      read: () => view.dom.querySelector<HTMLElement>(".cm-gutters")?.offsetWidth ?? 0,
      write: (width) => view.dom.style.setProperty("--vet-card-inset", `${width}px`),
      key: gutterInset,
    });
  measure();
  return { update: measure };
});

export function threadCards(host: CardHost): Extension {
  const cards = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(decos, tr) {
      decos = decos.map(tr.changes);
      for (const effect of tr.effects) {
        if (!effect.is(setThreadCards)) continue;
        const doc = tr.state.doc;
        decos = Decoration.set(
          effect.value.map((card) =>
            Decoration.widget({ widget: new CardWidget(card, host), block: true, side: 1 }).range(
              doc.line(Math.min(card.line + 1, doc.lines)).to,
            ),
          ),
          true,
        );
      }
      return decos;
    },
    provide: (field) => EditorView.decorations.from(field),
  });
  return [cards, gutterInset];
}
