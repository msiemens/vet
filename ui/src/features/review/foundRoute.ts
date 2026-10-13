// Where a result of Find in Files is shown. Its line numbers are those of the place that was searched
// (the changeset, a commit, a range of versions, or the repository), so it is shown in a view of that
// place and never in whatever else is on screen.

import type { SearchScope } from "../../generated/SearchScope";
import type { View } from "../../generated/View";

export interface FoundRoute {
  /** The view to bring on screen first; none if what is on screen already serves. */
  show?: View;
  /** Show it in the Changes tab if that is the tab on show and the file is in it; else in the file's own tab. */
  inChanges: boolean;
  /** The file's tab is pinned to the commit searched. */
  pinned: boolean;
}

const sameView = (a: View, b: View): boolean => {
  if (a.type === "interdiff") return b.type === "interdiff" && a.from === b.from && a.to === b.to;
  if (a.type === "commit") return b.type === "commit" && a.index === b.index;
  return a.type === b.type;
};

/**
 * The route of a result found in `scope` while `onShow` is on screen. A file's tab reads the changeset
 * while a commit is on screen, so the changeset and the repository need no switch then; a range of
 * versions has no tab of its own to pin, so it is brought on screen.
 */
export function routeFound(scope: SearchScope, onShow: View, ownTab: boolean): FoundRoute {
  const whole: View = { type: "changeset" };
  if (scope.type === "repository") {
    return onShow.type === "interdiff"
      ? { show: whole, inChanges: false, pinned: false }
      : { inChanges: false, pinned: false };
  }
  const { view } = scope;
  if (view.type === "commit") return { inChanges: !ownTab && sameView(view, onShow), pinned: true };
  if (view.type === "interdiff")
    return sameView(view, onShow)
      ? { inChanges: !ownTab, pinned: false }
      : { show: view, inChanges: !ownTab, pinned: false };
  if (onShow.type === "interdiff") return { show: whole, inChanges: !ownTab, pinned: false };
  return { inChanges: !ownTab && onShow.type !== "commit", pinned: false };
}
