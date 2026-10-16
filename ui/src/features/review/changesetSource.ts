// What the review controller and its modules read of the open changeset and ask of it. The whole
// changeset store implements it; naming the part keeps the coupling visible and the modules testable.

import type { Changeset } from "../../state/changeset.svelte";

export type ChangesetSource = Pick<
  Changeset,
  | "baseChanged"
  | "currentShown"
  | "currentLayout"
  | "layoutOf"
  | "admit"
  | "adopt"
  | "onOpening"
  | "shownCommit"
  | "whole"
  | "commitList"
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
  | "frame"
  | "refresh"
  | "reportFailure"
>;
