// What Find in Files holds between openings: the query, the options, the filters and the file mask stay as
// they were for as long as the window lives. The place searched and the chosen result belong to the
// changeset on view (its session). The dialog itself belongs to the open project: closing one puts it away.

import type { SearchSide } from "../../generated/SearchSide.ts";
import { session } from "../../state/session.svelte";

/** What a request to open Find in Files can bring along. */
export interface FindInFilesRequest {
  query?: string;
  matchCase?: boolean;
  wholeWord?: boolean;
  regex?: boolean;
}

class FindInFiles {
  open = $state(false);
  text = $state("");
  matchCase = $state(false);
  wholeWord = $state(false);
  regex = $state(false);
  side = $state<SearchSide>("both");
  onlyChanges = $state(false);
  notSeen = $state(false);
  mask = $state("");
  /** Each request puts the cursor in the query field, also while the dialog is open. */
  focusRequests = $state(0);

  constructor() {
    session.onClose(() => this.close());
  }

  show({ query, matchCase, wholeWord, regex }: FindInFilesRequest = {}) {
    if (query !== undefined) this.text = query;
    if (matchCase !== undefined) this.matchCase = matchCase;
    if (wholeWord !== undefined) this.wholeWord = wholeWord;
    if (regex !== undefined) this.regex = regex;
    this.open = true;
    this.focusRequests++;
  }

  close() {
    this.open = false;
  }
}

export const findInFiles = new FindInFiles();

/** Opens Find in Files, with `query` if there is one, else with the last one. */
export const openFindInFiles = (request: FindInFilesRequest = {}) => findInFiles.show(request);
