// A comment thread as plain DOM: the card an editor widget draws and the brief card a panel reads,
// with the pieces both use (a message's markdown, meta, avatar and a folded thread's row). Shared by
// the review editor and the queue, so neither reaches into the other; nothing here imports Svelte.

import type { Thread } from "../../generated/Thread";
import type { ThreadMessage } from "../../generated/ThreadMessage";
import { avatarColor } from "../avatar.ts";
import { commentPreview, renderComment } from "../commentMarkdown.ts";
import { count, noun } from "../format.ts";
import { splitPath } from "../paths.ts";
import { liveTime } from "../liveTime.ts";
import { fullTime } from "../shortTime.ts";
import { el, iconButton } from "./dom.ts";
import { iconSvg } from "./icons.ts";

/** A message that only Vet has, in a thread the platform has or will have: posted with the review. */
export const isDraft = (thread: Thread, message: ThreadMessage) => !!thread.platform && message.state.type !== "posted";

/** A card inside its slot. */
export function slot(card: HTMLElement): HTMLElement {
  const outer = el("div", "vet-thread");
  outer.append(card);
  return outer;
}

/** The author's picture, or a disc with their initials. */
export function avatar(author: string, url: string | null): HTMLElement {
  if (url) {
    const img = el("img", "vet-avatar");
    img.src = url;
    img.alt = "";
    return img;
  }
  const initials = el("span", "vet-avatar vet-avatar-initials", author.slice(0, 2).toUpperCase());
  initials.style.background = `var(--avatar-${avatarColor(author)})`;
  return initials;
}

/** The text of a comment, as markdown; a link in it opens in the browser, never in the webview. */
export function markdownBody(text: string, actions: { openLink(url: string): void }): HTMLElement {
  const body = el("div", "vet-msg-body vet-md selectable");
  body.innerHTML = renderComment(text);
  body.addEventListener("click", (e) => {
    const link = e.target instanceof Element ? e.target.closest("a[href]") : null;
    if (!(link instanceof HTMLAnchorElement)) return;
    e.preventDefault();
    if (/^(https?|mailto):/i.test(link.href)) actions.openLink(link.href);
  });
  return body;
}

/** A message's meta line: who wrote it and when, whether it was edited, and whether it waits to be posted. */
export function messageMeta(thread: Thread, message: ThreadMessage): HTMLElement {
  const meta = el("div", "vet-msg-meta");
  const when = el("span", "vet-msg-time");
  liveTime(when, message.at, "detailed");
  when.title = fullTime(message.at);
  meta.append(el("span", "vet-msg-author", message.author), when);
  if (message.editedAt !== null) {
    const edited = el("span", "vet-msg-edited", "edited");
    edited.title = fullTime(message.editedAt);
    meta.append(edited);
  }
  if (isDraft(thread, message)) meta.append(el("span", "vet-msg-draft-label", "Draft"));
  return meta;
}

/** What a resolved thread's one-row card says. */
export function foldedSummary(thread: Thread) {
  const [first] = thread.messages;
  const participants = new Map<string, string | null>();
  for (const m of thread.messages) if (!participants.has(m.author)) participants.set(m.author, m.avatar);
  const by = thread.platform?.resolvedBy;
  return {
    author: first.author,
    text: commentPreview(first.body),
    count: thread.messages.length > 1 ? `${thread.messages.length} comments` : null,
    participants: [...participants].map(([author, picture]) => ({ author, picture })),
    resolved: by ? `Resolved by ${by}` : "Resolved",
  };
}

/**
 * A resolved thread, folded to the row that says who started it and what they said. A narrow row (`place`:
 * where the thread is) leaves out who took part and who resolved it, and ends with the place.
 */
export function foldedCard(thread: Thread, toggle: () => void, place?: HTMLElement): HTMLElement {
  const summary = foldedSummary(thread);
  const card = el("div", "vet-thread-card vet-thread-folded");
  card.addEventListener("click", toggle);
  const chevron = iconButton("Show the thread", "closed", () => undefined, "vet-thread-btn", 10, 2);
  const ok = el("span", "vet-thread-ok");
  ok.innerHTML = iconSvg("check", 13, 2);
  const preview = el("span", "vet-thread-preview");
  preview.append(el("span", "vet-msg-author", summary.author), `: ${summary.text}`);
  card.append(chevron, ok, preview);
  if (place) {
    if (summary.count) card.append(el("span", "vet-thread-count", String(thread.messages.length)));
    card.append(place);
    return slot(card);
  }
  if (summary.count) card.append(el("span", "vet-thread-count", summary.count));
  const stack = el("span", "vet-avatar-stack");
  stack.append(...summary.participants.slice(0, 3).map((p) => avatar(p.author, p.picture)));
  card.append(stack, el("span", "vet-thread-by", summary.resolved));
  return slot(card);
}

/** How many lines of a message the brief card shows before "Show all". */
export const BRIEF_LINES = 4;

/** A message this long is cut by the lines of a narrow card, whatever its own line breaks. */
const LONG_MESSAGE = 240;

/** Whether a message is cut short in a brief card, so it offers "Show all". */
export function needsShowAll(text: string): boolean {
  return text.length > LONG_MESSAGE || text.split("\n").length > BRIEF_LINES;
}

/** The first message and the last, and how many lie between them; a thread of two or fewer shows all. */
export function briefMessages<T>(messages: readonly T[]): { shown: T[]; between: number } {
  if (messages.length <= 2) return { shown: [...messages], between: 0 };
  return { shown: [messages[0], messages[messages.length - 1]], between: messages.length - 2 };
}

/** What a brief card needs of its owner. */
export interface BriefOptions {
  /** The file and the line suffix (`:12`) the thread is on; none for the changeset's own. */
  place?: { path: string; suffix: string };
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
  const fresh = message.new;
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
  const fresh = thread.messages.filter((m) => m.new).length;
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

/** A comment thread as a card for reading, in a panel outside the editors: where it is, the first
 * message and the last with a line for those between, and no way to answer or change it. A resolved
 * thread is the folded row of the diff's card, or its open card with the folded row's chevron. */
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
