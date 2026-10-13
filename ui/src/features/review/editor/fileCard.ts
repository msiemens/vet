// The change card of a changed line in the File view: for a modified line the old line with the
// replaced words marked, and a footer. The footer's left is reserved for where the line came from
// (version, commit and author), which needs a per-line blame and is not filled yet; its right says
// the hunk's review state. One card at a time; a press outside it, Escape or a scroll closes it.

import type { HunkState } from "../../../generated/HunkState";
import { el } from "./dom";
import { type OldLine, segments } from "./fileLayout";

export const STATE_TEXT: Record<HunkState, string> = {
  unseen: "Unseen",
  seen: "Seen",
  seenAgainstOldBase: "Seen against an older base",
  informational: "Nothing left to review",
  unapplied: "Did not apply to the new base",
};

export interface CardContent {
  /** The old text of a modified line; none for an added one. */
  old: OldLine[];
  state: HunkState;
}

/** Where the card goes: below the line's box, left-aligned with `left`. */
export interface CardAnchor {
  left: number;
  top: number;
  bottom: number;
}

let close: (() => void) | undefined;

export function closeChangeCard() {
  close?.();
}

function build(content: CardContent): HTMLElement {
  const card = el("div", "surface vet-fcard");
  card.setAttribute("role", "dialog");
  if (content.old.length) {
    const old = el("div", "vet-fcard-old");
    for (const line of content.old) {
      const row = el("div", "vet-fcard-line");
      const text = el("span", "vet-fcard-text");
      for (const part of segments(line.text, line.marks)) {
        if (part.marked) text.append(el("span", "vet-fcard-changed", part.text));
        else text.append(part.text);
      }
      row.append(el("span", "vet-fcard-sign", "−"), text);
      old.append(row);
    }
    card.append(old);
  }
  const footer = el("div", "vet-fcard-footer");
  // Reserved for the version, commit and author that introduced the line.
  footer.append(
    el("span", "vet-fcard-origin"),
    el("span", `vet-fcard-state vet-fcard-${content.state}`, STATE_TEXT[content.state]),
  );
  card.append(footer);
  return card;
}

/** Shows the card for `content` at `anchor`, replacing the one that is open. */
export function showChangeCard(anchor: CardAnchor, content: CardContent) {
  close?.();
  const card = build(content);
  document.body.append(card);
  const left = Math.min(anchor.left, window.innerWidth - card.offsetWidth - 4);
  const below = anchor.bottom + 2;
  const top = below + card.offsetHeight > window.innerHeight - 4 ? anchor.top - card.offsetHeight - 2 : below;
  card.style.left = `${Math.max(4, left)}px`;
  card.style.top = `${Math.max(4, top)}px`;

  const onPress = (e: Event) => e.target instanceof Node && !card.contains(e.target) && close?.();
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    close?.();
  };
  const onScroll = (e: Event) => e.target instanceof Node && !card.contains(e.target) && close?.();
  window.addEventListener("pointerdown", onPress, true);
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("scroll", onScroll, true);
  window.addEventListener("resize", dismiss);
  function dismiss() {
    window.removeEventListener("pointerdown", onPress, true);
    window.removeEventListener("keydown", onKey, true);
    window.removeEventListener("scroll", onScroll, true);
    window.removeEventListener("resize", dismiss);
    card.remove();
    if (close === dismiss) close = undefined;
  }
  close = dismiss;
}
