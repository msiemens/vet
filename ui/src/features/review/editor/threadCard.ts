// A comment thread as a card: its messages stacked as rows, a reply field and the thread's actions
// in its top right corner, or the form that opens a thread. Plain DOM, like `changeCard.ts`, so
// editor widgets and section blocks share it. The card sits in a slot (`.vet-thread`) that insets it
// from the code column's edge. Its height depends on the text, so the view measures the slot
// (`ChangesetView`); the estimate of `threadPlacement.ts` only holds the place until then. The pieces
// a card shares with the queue's brief card (message body, meta, avatar, folded row) are in
// `lib/ui/commentCard.ts`.

import type { EditChoice } from "../../../generated/EditChoice";
import type { EditConflict } from "../../../generated/EditConflict";
import type { Thread } from "../../../generated/Thread";
import type { ThreadMessage } from "../../../generated/ThreadMessage";
import type { ThreadStatus } from "../../../generated/ThreadStatus";
import type { ThreadTarget } from "../../../generated/ThreadTarget";
import { canSubmit, isSubmitKey, replyShape } from "../../../lib/composer.ts";
import type { CommentDrafts } from "../../../state/commentDrafts.ts";
import { isFlat } from "../../../lib/flatThread.ts";
import { lineRange } from "../../../lib/lineRange.ts";
import { movedLabel, movedTitle } from "../../../lib/movedTitle.ts";
import { avatar, foldedCard, isDraft, markdownBody, messageMeta, slot } from "../../../lib/ui/commentCard.ts";
import { el, iconButton } from "../../../lib/ui/dom.ts";
import type { CardData } from "./threadPlacement.ts";
import { markDifferences } from "./wordDiff.ts";

/** What a card asks of its owner; each resolves whether the request went through. */
export interface ThreadActions {
  create(target: ThreadTarget, body: string): Promise<boolean>;
  reply(thread: number, body: string): Promise<boolean>;
  setStatus(thread: number, status: ThreadStatus): Promise<boolean>;
  remove(thread: number): Promise<boolean>;
  /** Edits one of the viewer's own comments. */
  editMessage(message: number, body: string): Promise<boolean>;
  /** Removes one of the viewer's own comments; a thread's first takes the thread with it. */
  removeMessage(message: number): Promise<boolean>;
  /** Settles a comment that was edited here and on the platform. */
  keepEdit(message: number, keep: EditChoice): Promise<boolean>;
  /** What the platform of the pull request on view is called. */
  platformName(): string;
  confirm(thread: number): Promise<boolean>;
  /** The form was dismissed. */
  cancelDraft(): void;
  /** Opens a resolved thread, or folds it again; the owner redraws it. */
  toggle(thread: number): void;
  /** Opens the menu of a thread's less common actions at a viewport position. */
  menu(thread: number, x: number, y: number): void;
  /** Shows a link of a comment in the browser. */
  openLink(url: string): void;
  /** The key that posts a form, as the viewer reads keys on this platform. */
  submitKey: string;
  /** What was typed into the cards' forms and not sent: the changeset the cards are drawn for keeps it. */
  drafts: CommentDrafts;
}

/** The key under which the unsent text of the form for a new comment on `target` is kept. */
export function newCommentKey(target: ThreadTarget): string {
  return `new:${JSON.stringify(target)}`;
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

/**
 * A text field and its submit button, which stays disabled while the text is blank or a request runs.
 * With a `key` its text outlives the card, in `drafts`. A `lazy` one shows as a reply field does (`replyShape`).
 */
function composer(
  rows: number,
  placeholder: string,
  label: string,
  submit: (body: string) => Promise<boolean>,
  drafts: CommentDrafts,
  key?: string,
  lazy = false,
) {
  const field = el("textarea", "field vet-thread-field");
  field.rows = rows;
  field.placeholder = placeholder;
  field.spellcheck = true;
  const send = button(label, "ctl primary vet-thread-send", () => void run());
  if (key !== undefined) field.value = drafts.get(key) ?? "";
  let busy = false;
  let focused = false;
  const refresh = () => {
    send.disabled = !canSubmit(field.value, busy);
    if (!lazy) return;
    const shape = replyShape(field.value, focused);
    field.rows = shape.rows;
    send.hidden = !shape.send && !busy;
  };
  refresh();
  async function run() {
    if (!canSubmit(field.value, busy)) return;
    busy = true;
    refresh();
    const sent = await submit(field.value.trim());
    busy = false;
    if (sent) {
      field.value = "";
      if (key !== undefined) drafts.delete(key);
    }
    refresh();
  }
  field.addEventListener("input", () => {
    refresh();
    if (key !== undefined) drafts.set(key, field.value);
  });
  field.addEventListener("focus", () => {
    focused = true;
    refresh();
  });
  field.addEventListener("blur", () => {
    focused = false;
    refresh();
  });
  field.addEventListener("keydown", (e) => {
    if (isSubmitKey(e)) {
      e.preventDefault();
      void run();
    }
  });
  return { field, send };
}

function draftCard(target: ThreadTarget, actions: ThreadActions): HTMLElement {
  const card = el("div", "vet-thread-card");
  const form = el("div", "vet-thread-form");
  const key = newCommentKey(target);
  const { field, send } = composer(
    3,
    "Comment…",
    "Comment",
    (body) => actions.create(target, body),
    actions.drafts,
    key,
  );
  const dismiss = () => {
    actions.drafts.delete(key);
    actions.cancelDraft();
  };
  const cancel = button("Cancel", "ctl", dismiss);
  field.addEventListener("keydown", (e) => {
    if (e.key === "Escape") dismiss();
  });
  const foot = el("div", "vet-thread-foot");
  foot.append(el("span", "vet-thread-hint", `${actions.submitKey} to post`), cancel, send);
  form.append(el("div", "vet-thread-title", draftTitle(target)), field, foot);
  card.append(form);
  // The card is in the document once the editor has laid it out, which may take a few frames if it is scrolled to.
  const focus = (tries: number) => {
    if (field.isConnected) field.focus();
    else if (tries > 0) requestAnimationFrame(() => focus(tries - 1));
  };
  requestAnimationFrame(() => focus(30));
  return slot(card);
}

/** The two texts of a comment that was edited in two places, side by side with Keep under each. */
function conflictCard(message: ThreadMessage, conflict: EditConflict, actions: ThreadActions): HTMLElement {
  const box = el("div", "vet-conflict");
  box.append(el("div", "vet-conflict-title", "Edited in two places"));
  const columns = el("div", "vet-conflict-columns");
  const column = (title: string, text: string, other: string, keep: EditChoice) => {
    const side = el("div", "vet-conflict-side");
    const body = el("div", "vet-conflict-text selectable");
    for (const piece of markDifferences(text, other)) {
      if (piece.changed) body.append(el("mark", "vet-diff", piece.text));
      else body.append(document.createTextNode(piece.text));
    }
    side.append(
      el("div", "vet-conflict-label", title),
      body,
      button(`Keep ${title}`, "ctl vet-thread-small", () => void actions.keepEdit(message.id, keep)),
    );
    return side;
  };
  columns.append(
    column("Vet", message.body, conflict.remoteBody, "vet"),
    column(actions.platformName(), conflict.remoteBody, message.body, "remote"),
  );
  box.append(columns);
  return box;
}

/** The form that replaces a message's text while it is edited. */
function editForm(message: ThreadMessage, actions: ThreadActions, close: () => void): HTMLElement {
  const key = `edit:${message.id}`;
  const leave = () => {
    actions.drafts.delete(key);
    close();
  };
  const form = el("div", "vet-msg-edit");
  const { field, send } = composer(
    2,
    "Comment…",
    "Save",
    async (body) => {
      const saved = await actions.editMessage(message.id, body);
      if (saved) leave();
      return saved;
    },
    actions.drafts,
    key,
  );
  field.addEventListener("keydown", (e) => {
    if (e.key === "Escape") leave();
  });
  const foot = el("div", "vet-thread-foot");
  foot.append(button("Cancel", "ctl vet-thread-small", leave), send);
  form.append(field, foot);
  requestAnimationFrame(() => field.focus());
  return form;
}

/** A message: its author's picture, who wrote it and when, and its text; the viewer's own can be edited and removed. */
function messageRow(thread: Thread, message: ThreadMessage, actions: ThreadActions): HTMLElement {
  const draft = isDraft(thread, message);
  const row = el("div", draft ? "vet-msg vet-msg-draft" : "vet-msg");
  const meta = messageMeta(thread, message);
  let shown: HTMLElement = message.conflict
    ? conflictCard(message, message.conflict, actions)
    : markdownBody(message.body, actions);
  const body = shown;
  const editKey = `edit:${message.id}`;
  const makeForm = () => {
    if (!actions.drafts.has(editKey)) actions.drafts.begin(editKey, message.body);
    const form: HTMLElement = editForm(message, actions, () => {
      form.replaceWith(body);
      shown = body;
    });
    return form;
  };
  const showForm = () => {
    const form = makeForm();
    shown.replaceWith(form);
    shown = form;
  };
  // A form left open by a card that was drawn again is open again, with its text.
  if (message.mine && !message.conflict && actions.drafts.has(editKey)) shown = makeForm();
  if (message.mine) {
    const first = thread.messages[0] === message;
    const edit = iconButton("Edit the comment", "edit", showForm, "vet-thread-btn", 12, 2);
    const remove = iconButton(
      draft ? "Discard the draft" : first ? "Delete the thread" : "Delete the comment",
      "close",
      () => void actions.removeMessage(message.id),
      "vet-thread-btn",
      12,
      2,
    );
    // A thread of no platform has the card's menu for deleting it, with its opening message.
    const opening = first && !thread.platform;
    const tools = el("span", "vet-msg-actions");
    tools.append(...(message.conflict ? [] : [edit]), ...(opening ? [] : [remove]));
    meta.append(tools);
  }
  row.append(avatar(message.author, message.avatar), meta, shown);
  return row;
}

/** What is known about a thread's anchor and state, above its messages; none when there is nothing to say. */
function notes(thread: Thread, actions: ThreadActions): HTMLElement | undefined {
  const row = el("div", "vet-thread-notes");
  if (thread.status === "resolved") {
    // Where the folded card has its chevron, so folding and opening are the same spot.
    row.append(
      iconButton("Hide the thread", "open", () => actions.toggle(thread.id), "vet-thread-btn", 10, 2),
      el("span", "badge badge-ok", "Resolved"),
    );
    const by = thread.platform?.resolvedBy;
    if (by) row.append(el("span", "vet-thread-by", `by ${by}`));
  }
  if (thread.anchor?.type === "inferred") {
    const moved = el("span", "badge badge-moved", movedLabel(thread));
    moved.title = movedTitle(thread);
    row.append(
      moved,
      button("Confirm", "ctl vet-thread-small", () => void actions.confirm(thread.id)),
    );
  }
  if (thread.anchor?.type === "exact" && thread.anchor.whitespaceChanged) {
    const note = el("span", "badge", "whitespace changed");
    note.title = "Only whitespace differs from where the comment was made";
    row.append(note);
  }
  return row.childElementCount ? row : undefined;
}

/** The icon buttons in the card's top right corner: Resolve or Reopen, and the menu. */
function threadTools(thread: Thread, actions: ThreadActions): HTMLElement | undefined {
  const resolved = thread.status === "resolved";
  const tools = el("div", "vet-thread-actions");
  // A comment on the pull request as a whole cannot be resolved.
  if (thread.platform?.resolvable !== false) {
    tools.append(
      iconButton(
        resolved ? "Reopen the thread" : "Resolve the thread",
        resolved ? "reconfirm" : "check",
        () => void actions.setStatus(thread.id, resolved ? "open" : "resolved"),
        "vet-icon-btn",
        13,
        2,
      ),
    );
  }
  // A thread the platform has is deleted through its opening message.
  if (!thread.platform) {
    const more = iconButton(
      "More actions",
      "more",
      () => {
        const at = more.getBoundingClientRect();
        actions.menu(thread.id, at.left, at.bottom);
      },
      "vet-icon-btn",
      13,
      2,
    );
    tools.append(more);
  }
  return tools.childElementCount ? tools : undefined;
}

function threadCard(thread: Thread, open: boolean, actions: ThreadActions): HTMLElement {
  const resolved = thread.status === "resolved";
  if (resolved && !open) return foldedCard(thread, () => actions.toggle(thread.id));
  const card = el("div", `vet-thread-card${resolved ? " vet-thread-resolved" : ""}`);
  card.dataset.thread = String(thread.id);
  const note = notes(thread, actions);
  if (note) card.append(note);
  for (const message of thread.messages) card.append(messageRow(thread, message, actions));
  if (!isFlat(thread)) {
    const reply = el("div", "vet-thread-reply");
    const { field, send } = composer(
      1,
      "Reply…",
      "Reply",
      (body) => actions.reply(thread.id, body),
      actions.drafts,
      `reply:${thread.id}`,
      true,
    );
    reply.append(field, send);
    card.append(reply);
  }
  const tools = threadTools(thread, actions);
  // The corner belongs to the first row, which keeps clear of it.
  if (tools && card.firstElementChild) {
    card.firstElementChild.classList.add("vet-thread-top");
    card.style.setProperty("--vet-actions", `${tools.childElementCount * 24}px`);
    card.append(tools);
  }
  return slot(card);
}

/** The card for a thread or a form. */
export function buildCard(data: CardData, actions: ThreadActions): HTMLElement {
  if (data.thread) return threadCard(data.thread, !!data.open, actions);
  return draftCard(data.draft ?? { type: "changeset" }, actions);
}
