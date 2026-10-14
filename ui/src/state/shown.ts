// What the UI shows of the changeset: the manifest revision and the view it is of. A view of files
// captures it when it is built, and every request that marks or comments from it carries it, so the
// request goes out under what the reviewer was looking at, not under what is shown by the time it is sent.

import type { View } from "../generated/View";
import { sameView } from "./view.ts";

export interface Shown {
  readonly revision: number;
  readonly view: View;
}

/** Whether a request built for `shown` still is about what `current` says is on view. */
export function isCurrent(shown: Shown, current: Shown | undefined): boolean {
  if (!current) return false;
  return shown.revision === current.revision && sameView(shown.view, current.view);
}
