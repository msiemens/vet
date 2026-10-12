// What the editor views show of a file's review state, read from what they drew (header texts, card
// classes, the editors' own review fields, the scroll bar's marks) and not from a model, so a test
// can compare it with what the backend says.

import type { HunkState } from "../../../generated/HunkState";

/** What a section of a diff view shows of its file. */
export interface RenderedFile {
  path: string;
  /** The header's progress text. */
  count: string;
  /** The width of the header's meter, in percent. */
  seenPercent: number;
  allSeen: boolean;
  /** Whether the header offers to re-confirm hunks seen against an old base. */
  reconfirm: boolean;
  /** Whether each change card shows its item as acknowledged. */
  cards: boolean[];
  /** The review state the editor draws, if the section has an editor mounted. */
  hunks?: HunkState[];
  flagged?: boolean[];
}

export interface RenderedView {
  files: RenderedFile[];
  /** The kinds of the marks in the scroll bar, top down. */
  marks: string[];
}

/** What a file's text shows of its hunks, once its editor exists. */
export interface RenderedText {
  hunks?: HunkState[];
  flagged?: boolean[];
}

/** What every view of the review screen shows, by tab. */
export interface Rendered {
  views: Record<string, RenderedView>;
  texts: Record<string, RenderedText>;
}
