// A comment thread as a card: its messages, a reply field and Resolve, or the form that opens
// a thread. Plain DOM, like `changeCard.ts`, so editor widgets and section blocks share it.
// Its height depends on the text, so the view measures it (`ChangesetView`); the estimate of
// `threadPlacement.ts` only holds the place until then.

import type { Thread } from "../../../generated/Thread";
import type { ThreadStatus } from "../../../generated/ThreadStatus";
import type { ThreadTarget } from "../../../generated/ThreadTarget";
import { lineRange } from "../../../lib/lineRange.ts";
import { el, iconButton } from "./dom.ts";
import type { CardData } from "./threadPlacement.ts";

/** What a card asks of its owner; each resolves whether the request went through. */
export interface ThreadActions {
  create(target: ThreadTarget, body: string): Promise<boolean>;
  reply(thread: number, body: string): Promise<boolean>;
  setStatus(thread: number, status: ThreadStatus): Promise<boolean>;
  remove(thread: number): Promise<boolean>;
  confirm(thread: number): Promise<boolean>;
  /** The form was dismissed. */
  cancelDraft(): void;
  /** Opens a resolved thread, or folds it again; the owner redraws it. */
  toggle(thread: number): void;
}

/** The tooltip of a "moved" badge: how the anchor was found and where it was recorded. */
export function movedTitle(thread: Thread): string {
  const anchor = thread.anchor;
  if (anchor?.type !== "inferred") return "";
  const how = anchor.strategy === "crossFileMove" ? "found in another file" : "found by its surrounding lines";
  const was = thread.original
    ? `; was ${thread.original.path}:${lineRange(thread.original.start, thread.original.len)}`
    : "";
  return `Moved: ${how}, ${anchor.confidence} confidence${was}`;
}

/** What a new comment is attached to, for the form's title. */
export function draftTitle(target: ThreadTarget): string {
  if (target.type === "changeset") return "Comment on the changeset";
  if (target.type === "file") return "Comment on this file";
  const lines = lineRange(target.start, target.len);
  return target.side === "base" ? `Comment on base ${lines}` : `Comment on ${lines}`;
}

function button(label: string, className: string, onclick: () => void) {
  const b = el("button", className, label);
  b.type = "button";
  b.addEventListener("click", onclick);
  return b;
}

const time = (at: number) => new Date(at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** A text field and its submit button, which stays disabled while the text is blank or a request runs. */
function composer(rows: number, placeholder: string, label: string, submit: (body: string) => Promise<boolean>) {
  const field = el("textarea", "vet-thread-field");
  field.rows = rows;
  field.placeholder = placeholder;
  field.spellcheck = true;
  const send = button(label, "ctl primary vet-thread-send", () => void run());
  send.disabled = true;
  let busy = false;
  async function run() {
    if (busy || !field.value.trim()) return;
    busy = true;
    send.disabled = true;
    const sent = await submit(field.value.trim());
    busy = false;
    if (sent) field.value = "";
    send.disabled = !field.value.trim();
  }
  field.addEventListener("input", () => (send.disabled = busy || !field.value.trim()));
  field.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void run();
    }
  });
  return { field, send };
}

function draftCard(target: ThreadTarget, actions: ThreadActions): HTMLElement {
  const card = el("div", "vet-thread vet-thread-draft");
  const head = el("div", "vet-thread-head");
  head.append(el("span", "vet-thread-title", draftTitle(target)));
  const { field, send } = composer(3, "Comment…", "Comment", (body) => actions.create(target, body));
  const cancel = button("Cancel", "ctl", () => actions.cancelDraft());
  field.addEventListener("keydown", (e) => {
    if (e.key === "Escape") actions.cancelDraft();
  });
  const foot = el("div", "vet-thread-foot");
  foot.append(el("span", "vet-thread-hint", "Ctrl+Enter to post"), cancel, send);
  card.append(head, field, foot);
  // The card is in the document once the editor has laid it out, which may take a few frames if it is scrolled to.
  const focus = (tries: number) => {
    if (field.isConnected) field.focus();
    else if (tries > 0) requestAnimationFrame(() => focus(tries - 1));
  };
  requestAnimationFrame(() => focus(30));
  return card;
}

function threadCard(thread: Thread, open: boolean, actions: ThreadActions): HTMLElement {
  const resolved = thread.status === "resolved";
  const folded = resolved && !open;
  const card = el("div", `vet-thread${resolved ? " vet-thread-resolved" : ""}`);
  card.dataset.thread = String(thread.id);

  const head = el("div", "vet-thread-head");
  if (resolved) {
    const toggle = iconButton(
      folded ? "Show the thread" : "Hide the thread",
      folded ? "closed" : "open",
      () => actions.toggle(thread.id),
      "vet-thread-btn",
      11,
      2,
    );
    head.append(toggle, el("span", "vet-badge vet-badge-ok", "Resolved"));
  }
  if (thread.anchor?.type === "inferred") {
    const moved = el("span", "vet-badge vet-badge-moved", "moved");
    moved.title = movedTitle(thread);
    head.append(
      moved,
      button("Confirm", "ctl vet-thread-small", () => void actions.confirm(thread.id)),
    );
  }
  if (thread.anchor?.type === "exact" && thread.anchor.whitespaceChanged) {
    const note = el("span", "vet-badge", "whitespace changed");
    note.title = "Only whitespace differs from where the comment was made";
    head.append(note);
  }
  if (folded) {
    const [first] = thread.messages;
    head.append(el("span", "vet-thread-preview", `${first.author}: ${first.body.split("\n")[0]}`));
  }
  head.append(el("span", "vet-thread-grow"));
  if (!folded) {
    head.append(
      button(
        resolved ? "Reopen" : "Resolve",
        "ctl vet-thread-small",
        () => void actions.setStatus(thread.id, resolved ? "open" : "resolved"),
      ),
      iconButton("Delete the thread", "close", () => void actions.remove(thread.id), "vet-thread-btn", 11, 2),
    );
  }
  card.append(head);
  if (folded) return card;

  for (const message of thread.messages) {
    const meta = el("div", "vet-msg-meta");
    meta.append(el("span", "vet-msg-author", message.author), el("span", "vet-msg-time", time(message.at)));
    card.append(meta, el("div", "vet-msg-body selectable", message.body));
  }
  const reply = el("div", "vet-thread-reply");
  const { field, send } = composer(1, "Reply…", "Reply", (body) => actions.reply(thread.id, body));
  reply.append(field, send);
  card.append(reply);
  return card;
}

/** The card for a thread or a form. */
export function buildCard(data: CardData, actions: ThreadActions): HTMLElement {
  if (data.thread) return threadCard(data.thread, !!data.open, actions);
  return draftCard(data.draft ?? { type: "changeset" }, actions);
}
