// A comment thread as a card: its messages, a reply field and Resolve, or the form that opens
// a thread. Plain DOM, like `changeCard.ts`, so editor widgets and section blocks share it.
// Its height depends on the text, so the view measures it (`ChangesetView`); the estimate of
// `threadPlacement.ts` only holds the place until then.

import type { EditChoice } from "../../../generated/EditChoice";
import type { EditConflict } from "../../../generated/EditConflict";
import type { Thread } from "../../../generated/Thread";
import type { ThreadMessage } from "../../../generated/ThreadMessage";
import type { ThreadStatus } from "../../../generated/ThreadStatus";
import type { ThreadTarget } from "../../../generated/ThreadTarget";
import { avatarColor } from "../../../lib/avatar.ts";
import { canSubmit, isSubmitKey } from "../../../lib/composer.ts";
import { lineRange } from "../../../lib/lineRange.ts";
import { movedTitle } from "../../../lib/movedTitle.ts";
import { el, iconButton } from "./dom.ts";
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

/**
 * What the viewer has typed in cards and not sent, by form (`reply:3`, `edit:7`, `new:…`). A card is
 * drawn again when the threads change, which must not cost the text; a form that is open has an entry,
 * possibly empty.
 */
const unsent = new Map<string, string>();

/**
 * A text field and its submit button, which stays disabled while the text is blank or a request runs.
 * With a `key` its text outlives the card.
 */
function composer(
  rows: number,
  placeholder: string,
  label: string,
  submit: (body: string) => Promise<boolean>,
  key?: string,
) {
  const field = el("textarea", "field vet-thread-field");
  field.rows = rows;
  field.placeholder = placeholder;
  field.spellcheck = true;
  const send = button(label, "ctl primary vet-thread-send", () => void run());
  if (key !== undefined) field.value = unsent.get(key) ?? "";
  send.disabled = !canSubmit(field.value, false);
  let busy = false;
  async function run() {
    if (!canSubmit(field.value, busy)) return;
    busy = true;
    send.disabled = true;
    const sent = await submit(field.value.trim());
    busy = false;
    if (sent) {
      field.value = "";
      if (key !== undefined) unsent.delete(key);
    }
    send.disabled = !canSubmit(field.value, busy);
  }
  field.addEventListener("input", () => {
    send.disabled = !canSubmit(field.value, busy);
    if (key !== undefined) unsent.set(key, field.value);
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
  const card = el("div", "vet-thread vet-thread-draft");
  const head = el("div", "vet-thread-head");
  head.append(el("span", "vet-thread-title", draftTitle(target)));
  const key = `new:${JSON.stringify(target)}`;
  const { field, send } = composer(3, "Comment…", "Comment", (body) => actions.create(target, body), key);
  const dismiss = () => {
    unsent.delete(key);
    actions.cancelDraft();
  };
  const cancel = button("Cancel", "ctl", dismiss);
  field.addEventListener("keydown", (e) => {
    if (e.key === "Escape") dismiss();
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

/** A comment on the pull request in the platform's flat conversation: it has no status and takes no replies. */
export const isFlat = (thread: Thread) => thread.kind === "changeset" && thread.platform?.resolvable === false;

/** A message that only Vet has, in a thread the platform has or will have: posted with the review. */
const isDraft = (thread: Thread, message: ThreadMessage) => !!thread.platform && !message.posted;

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

/** The two texts of a comment that was edited in two places, side by side with Keep under each. */
function conflictCard(message: ThreadMessage, conflict: EditConflict, actions: ThreadActions): HTMLElement {
  const box = el("div", "vet-conflict");
  box.append(el("div", "vet-conflict-title", "Edited in two places"));
  const columns = el("div", "vet-conflict-columns");
  const column = (title: string, text: string, other: string, keep: EditChoice) => {
    const side = el("div", "vet-conflict-side");
    const body = el("div", "vet-msg-body selectable");
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
    unsent.delete(key);
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

/** A message with who wrote it and when; the viewer's own can be edited and removed. */
function messageParts(thread: Thread, message: ThreadMessage, actions: ThreadActions): HTMLElement[] {
  const meta = el("div", "vet-msg-meta");
  meta.append(avatar(message.author, message.avatar), el("span", "vet-msg-author", message.author));
  meta.append(el("span", "vet-msg-time", time(message.at)));
  if (message.editedAt !== null) meta.append(el("span", "vet-msg-edited", "edited"));
  if (isDraft(thread, message)) meta.append(el("span", "badge vet-msg-draft", "Draft"));
  const body = el(
    "div",
    message.conflict || isDraft(thread, message) ? "vet-msg-body selectable vet-draft" : "vet-msg-body selectable",
  );
  body.textContent = message.body;
  const shown = message.conflict ? conflictCard(message, message.conflict, actions) : body;
  const parts = [meta, shown];
  const editKey = `edit:${message.id}`;
  const makeForm = () => {
    if (!unsent.has(editKey)) unsent.set(editKey, message.body);
    const form: HTMLElement = editForm(message, actions, () => form.replaceWith(shown));
    return form;
  };
  const showForm = () => shown.replaceWith(makeForm());
  // A form left open by a card that was drawn again is open again, with its text.
  if (message.mine && !message.conflict && unsent.has(editKey)) parts[1] = makeForm();
  if (message.mine) {
    const edit = iconButton("Edit the comment", "edit", showForm, "vet-thread-btn", 11, 2);
    const remove = iconButton(
      thread.messages[0] === message ? "Delete the thread" : "Remove the comment",
      "close",
      () => void actions.removeMessage(message.id),
      "vet-thread-btn",
      11,
      2,
    );
    // A thread of no platform has the head's delete button for its opening message.
    const opening = thread.messages[0] === message && !thread.platform;
    meta.append(el("span", "vet-thread-grow"), ...(message.conflict ? [] : [edit]), ...(opening ? [] : [remove]));
  }
  return parts;
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
    head.append(toggle, el("span", "badge badge-ok", "Resolved"));
  }
  if (thread.anchor?.type === "inferred") {
    const moved = el("span", "badge badge-moved", "moved");
    moved.title = movedTitle(thread);
    head.append(
      moved,
      button("Confirm", "ctl vet-thread-small", () => void actions.confirm(thread.id)),
    );
  }
  if (thread.anchor?.type === "exact" && thread.anchor.whitespaceChanged) {
    const note = el("span", "badge", "whitespace changed");
    note.title = "Only whitespace differs from where the comment was made";
    head.append(note);
  }
  if (folded) {
    const [first] = thread.messages;
    head.append(el("span", "vet-thread-preview", `${first.author}: ${first.body.split("\n")[0]}`));
  }
  head.append(el("span", "vet-thread-grow"));
  if (!folded) {
    if (resolved && thread.platform?.resolvedBy) {
      head.append(el("span", "vet-thread-by", `by ${thread.platform.resolvedBy}`));
    }
    // A comment on the pull request as a whole cannot be resolved.
    if (thread.platform?.resolvable !== false) {
      head.append(
        button(
          resolved ? "Reopen" : "Resolve",
          "ctl vet-thread-small",
          () => void actions.setStatus(thread.id, resolved ? "open" : "resolved"),
        ),
      );
    }
    if (!thread.platform) {
      head.append(
        iconButton("Delete the thread", "close", () => void actions.remove(thread.id), "vet-thread-btn", 11, 2),
      );
    }
  }
  card.append(head);
  if (folded) return card;

  for (const message of thread.messages) card.append(...messageParts(thread, message, actions));
  if (isFlat(thread)) return card;
  const reply = el("div", "vet-thread-reply");
  const { field, send } = composer(
    1,
    "Reply…",
    "Reply",
    (body) => actions.reply(thread.id, body),
    `reply:${thread.id}`,
  );
  reply.append(field, send);
  card.append(reply);
  return card;
}

/** The card for a thread or a form. */
export function buildCard(data: CardData, actions: ThreadActions): HTMLElement {
  if (data.thread) return threadCard(data.thread, !!data.open, actions);
  return draftCard(data.draft ?? { type: "changeset" }, actions);
}
