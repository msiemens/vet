// What the find bar shows. The query and its options are the window's, one for every tab; which tabs
// have the bar open, and what the active tab's view found, are per tab. It lives with the window, not
// the screen: rebuilding the review screen over the same tabs brings the bar back as it was.

import { SvelteSet } from "svelte/reactivity";
import type { TabId } from "../../state/commands";
import type { FindOptions } from "./editor/findMatches";
import { type FindSummary, NO_SUMMARY } from "./editor/findView";

export class FindState {
  options = $state<FindOptions>({
    text: "",
    matchCase: false,
    wholeWord: false,
    regex: false,
    side: "both",
    onlyChanges: false,
  });
  /** The tabs whose bar is open. */
  open = new SvelteSet<TabId>();
  /** What the active tab's view found. */
  summary = $state<FindSummary>(NO_SUMMARY);
  /** Counts the requests to put the cursor in the query field. */
  focusRequests = $state(0);
  /** The last step that went round the end, as the notice says it; `n` tells one from the next. */
  wrapped = $state<{ text: string; n: number }>();
}

/** The find bar's state, shared by the review controllers the window builds. */
export const reviewFind = new FindState();
