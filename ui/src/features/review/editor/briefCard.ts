// A comment thread as a card for reading, in a panel outside the editors: where it is, the first
// message and the last with a line for those between, and no way to answer or change it. A resolved
// thread is the folded row of the diff's card, or its open card with the folded row's chevron.
// Plain DOM, built from the pieces of `threadCard.ts`.

import type { Thread } from "../../../generated/Thread.ts";
import type { ThreadMessage } from "../../../generated/ThreadMessage.ts";
import { count, noun } from "../../../lib/format.ts";
import { splitPath } from "../../../lib/paths.ts";
import { iconSvg } from "../../../lib/ui/icons.ts";
import { briefMessages, needsShowAll } from "./briefThread.ts";
import { el, iconButton } from "./dom.ts";
import { avatar, foldedCard, markdownBody, messageMeta, slot } from "./threadCard.ts";

/** What a brief card needs of its owner. */
export interface BriefOptions {
  /** The file and the line suffix (`:12`) the thread is on; none for the changeset's own. */
  place?: { path: string; suffix: string };
  /** Whether the viewer has not read the message. */
  isNew(message: ThreadMessage): boolean;
  /** Opens a resolved thread, or folds it again; the owner redraws it. */
  toggle(): void;
  /** Shows the thread in the review. */
  openThread(): void;
  /** Shows a link of a comment in the browser. */
  openLink(url: string): void;
}

/** Where the thread is, as the button that shows it in the review. */
function placeButton(brief: BriefOptions): HTMLElement {
  if (!brief.place) return el("span", "vet-thread-place", "Changeset");
  const { path, suffix } = brief.place;
  const button = el("button", "flat vet-thread-place", splitPath(path).name + suffix);
  button.type = "button";
  button.title = path;
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    brief.openThread();
  });
  return button;
}

/** A message: the first lines of its text, and "Show all" when there are more. */
function briefMessage(thread: Thread, message: ThreadMessage, brief: BriefOptions): HTMLElement {
  const fresh = brief.isNew(message);
  const row = el("div", fresh ? "vet-msg vet-msg-new" : "vet-msg");
  const meta = messageMeta(thread, message);
  if (fresh) {
    const dot = el("span", "vet-new-dot");
    dot.title = "New since you last looked";
    meta.append(dot);
  }
  const body = markdownBody(message.body, brief);
  row.append(avatar(message.author, message.avatar), meta, body);
  if (needsShowAll(message.body)) {
    body.classList.add("vet-msg-clamp");
    const more = el("button", "flat vet-msg-showall", "Show all");
    more.type = "button";
    more.addEventListener("click", (e) => {
      e.stopPropagation();
      body.classList.remove("vet-msg-clamp");
      more.remove();
    });
    row.append(more);
  }
  return row;
}

/** The card's top row: for a resolved thread the chevron that folds it, then the place and what is new. */
function topRow(thread: Thread, brief: BriefOptions): HTMLElement {
  const row = el("div", "vet-thread-notes");
  const resolved = thread.status === "resolved";
  if (resolved) {
    row.append(iconButton("Hide the thread", "open", () => brief.toggle(), "vet-thread-btn", 10, 2));
  }
  row.append(placeButton(brief), el("span", "vet-thread-grow"));
  const fresh = thread.messages.filter((m) => brief.isNew(m)).length;
  if (fresh > 0) {
    const mark = el("span", "vet-thread-new");
    mark.append(el("span", "vet-new-dot"), `${fresh} new`);
    row.append(mark);
  }
  const by = thread.platform?.resolvedBy;
  const state = resolved ? (by ? `Resolved by ${by}` : "Resolved") : thread.messages[0]?.mine ? "yours · Open" : "Open";
  row.append(el("span", "vet-thread-by", state));
  return row;
}

export function buildBriefCard(thread: Thread, open: boolean, brief: BriefOptions): HTMLElement {
  const resolved = thread.status === "resolved";
  if (resolved && !open) {
    const place = placeButton(brief);
    return foldedCard(thread, () => brief.toggle(), place);
  }
  const card = el("div", "vet-thread-card vet-thread-brief");
  card.dataset.thread = String(thread.id);
  card.title = "Show the thread in the review";
  card.addEventListener("click", () => {
    // Selecting text to copy it is not a click on the card.
    if (!window.getSelection()?.toString()) brief.openThread();
  });
  card.append(topRow(thread, brief));
  const { shown, between } = briefMessages(thread.messages);
  shown.forEach((message, i) => {
    if (i === 1 && between > 0) {
      const line = el("div", "vet-thread-more");
      line.innerHTML = iconSvg("closed", 9, 2);
      line.append(`${count(between)} more ${noun(between, "reply", "replies")}`);
      card.append(line);
    }
    card.append(briefMessage(thread, message, brief));
  });
  return slot(card);
}
