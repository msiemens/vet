// What the review controller and its modules read of the open changeset and ask of it. The changeset's
// session implements it; naming the part keeps the coupling visible and the modules testable.

import type { ChangesetSession } from "../../state/changesetSession.svelte";

export type ChangesetSource = Pick<
  ChangesetSession,
  | "baseChanged"
  | "currentShown"
  | "currentLayout"
  | "layoutOf"
  | "admit"
  | "adopt"
  | "onViewChange"
  | "shownCommit"
  | "whole"
  | "commitList"
  | "drafts"
  | "findSets"
  | "stepCommit"
  | "setRange"
  | "showWhole"
  | "subscribe"
  | "state"
  | "version"
  | "toggleHunk"
  | "toggleFlag"
  | "toggleItem"
  | "toggleFileSeen"
  | "reconfirmFile"
  | "baseDiff"
  | "commentTarget"
  | "reportFailure"
>;
