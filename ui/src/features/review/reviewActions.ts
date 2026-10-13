// What the review commands do, supplied by whatever is mounted. The commands are registered once,
// so the menus list them whether or not a project is open; with no review screen they do nothing.

import type { FileActions, ViewActions } from "./reviewCommands.ts";

type ReviewActions = ViewActions & FileActions;

const sources = new Set<Partial<ReviewActions>>();

const live = () => Object.assign({}, ...sources) as Partial<ReviewActions>;

/** Makes `actions` what the commands do, until the returned function is called. */
export function provideActions(actions: Partial<ReviewActions>): () => void {
  sources.add(actions);
  return () => void sources.delete(actions);
}

export const reviewActions: ReviewActions = {
  step: (delta) => live().step?.(delta),
  toggleSeen: () => live().toggleSeen?.() ?? false,
  toggleFlag: () => live().toggleFlag?.() ?? false,
  stepFlag: (delta) => live().stepFlag?.(delta) ?? false,
  firstFlagged: () => live().firstFlagged?.() ?? false,
  reconfirm: () => live().reconfirm?.() ?? false,
  comment: () => live().comment?.() ?? false,
  showCard: () => live().showCard?.() ?? false,
  jumpToSource: () => live().jumpToSource?.() ?? false,
  stepCommit: (delta) => live().stepCommit?.(delta) ?? false,
  toggleStructure: () => live().toggleStructure?.() ?? false,
  setViewMode: (mode) => live().setViewMode?.(mode),
  copyText: (text) => live().copyText?.(text),
  excludePath: (path, dir) => live().excludePath?.(path, dir),
  reviewOnly: (path) => live().reviewOnly?.(path),
  setFolderSeen: (path, seen) => live().setFolderSeen?.(path, seen),
  closeTab: (tab) => live().closeTab?.(tab),
  closeOtherTabs: (tab) => live().closeOtherTabs?.(tab),
  stepTab: (delta) => live().stepTab?.(delta),
  revealFile: () => live().revealFile?.() ?? false,
};
