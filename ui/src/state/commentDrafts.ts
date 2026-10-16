// The text the viewer has typed into comment boxes and not added as a comment, by box: the changeset
// comment's form, the form for a new comment on code, a reply field and the form that edits a message.
// A changeset keeps its own, so switching to another starts with empty boxes.

/** The comment on the changeset as a whole being written in the Comments tool window. */
export interface ChangesetComment {
  composing: boolean;
  body: string;
}

/** A comment on the changeset with nothing written. */
export const emptyChangesetComment = (): ChangesetComment => ({ composing: false, body: "" });

/** Opens the form for a comment on the changeset as a whole. */
export function composeChangesetComment(comment: ChangesetComment): void {
  comment.composing = true;
}

/** Closes the changeset comment form, keeping what was written. */
export function cancelChangesetComment(comment: ChangesetComment): void {
  comment.composing = false;
}

/** The changeset comment was posted: the form closes and starts empty. */
export function clearChangesetComment(comment: ChangesetComment): void {
  comment.composing = false;
  comment.body = "";
}

/**
 * What cards hold of their forms (`reply:3`, `edit:7`, `new:…`). A card is drawn again when the threads
 * change, which must not cost the text; a form that is open has an entry, possibly empty.
 */
export class CommentDrafts {
  private texts = new Map<string, string>();
  /** The message an edit form started from: its text is no draft until it differs. */
  private originals = new Map<string, string>();

  private changed: () => void;

  /** `changed` is called whenever what a form holds changes. */
  constructor(changed: () => void = () => undefined) {
    this.changed = changed;
  }

  get(key: string): string | undefined {
    return this.texts.get(key);
  }

  has(key: string): boolean {
    return this.texts.has(key);
  }

  set(key: string, text: string): void {
    this.texts.set(key, text);
    this.changed();
  }

  /** Opens the form `key` with the text of the message it edits. */
  begin(key: string, original: string): void {
    this.texts.set(key, original);
    this.originals.set(key, original);
    this.changed();
  }

  delete(key: string): void {
    this.texts.delete(key);
    this.originals.delete(key);
    this.changed();
  }

  /** How many forms hold text that was not added as a comment. */
  count(): number {
    let held = 0;
    for (const [key, text] of this.texts) if (text.trim() !== "" && text !== this.originals.get(key)) held++;
    return held;
  }
}

/** How many boxes hold text that would be lost: `cards` forms of the cards and the changeset comment's. */
export const unsentComments = (cards: number, changeset: ChangesetComment): number =>
  cards + (changeset.body.trim() === "" ? 0 : 1);
