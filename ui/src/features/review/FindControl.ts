// The find bar's runs: which pane of the active tab a query searches, where the current match is
// although the files before it come and go, and the step that wrapped. The shared `FindState` is the
// bar's; this drives the views' `Finder`s.

import { Debounced } from "../../lib/debounced.ts";
import type { TabId } from "../../state/commands";
import { type FindOptions, type PathPos, fromPathPos, toPathPos } from "./editor/findMatches.ts";
import { type Finder, type FindSummary, NO_SUMMARY } from "./editor/findView.ts";
import type { FindState } from "./FindState.svelte";
import type { Tabs } from "./Tabs.svelte";

/** The query is searched this long after the last key, so typing does not search a long changeset for every letter. */
const FIND_TYPING_MS = 120;

/** The pane of a file tab the find runs in: its text or its diff. */
export type FindPane = "text" | "diff";

/** What the runs read of the views. */
export interface FindViews {
  /** The finder that searches the pane of the tab, if the pane is built. */
  finder(id: TabId, pane: FindPane): Finder | undefined;
  /** The files of the sections a find in the pane runs over, in order. */
  paths(id: TabId, pane: FindPane): readonly string[];
}

export class FindControl {
  /** Where the current match is in each tab, which a rebuilt view starts from; by file, since the files before it come and go. */
  private at = new Map<TabId, PathPos>();
  private wraps = 0;
  /** The tab whose current match is to be shown, as long as its view may still be replaced. */
  private landing?: TabId;
  /** The query is searched this long after the last key. */
  private typing = new Debounced();
  private readonly state: FindState;
  private readonly tabs: Tabs;
  private readonly views: FindViews;

  constructor(state: FindState, tabs: Tabs, views: FindViews) {
    this.state = state;
    this.tabs = tabs;
    this.views = views;
  }

  /** The pane of the tab the find runs in. */
  pane(id: TabId): FindPane {
    return this.tabs.modes.get(id) === "file" ? "text" : "diff";
  }

  /** Runs the find in the pane the tab shows, and ends it in the other. */
  apply(id: TabId) {
    const shown = this.pane(id);
    this.views.finder(id, shown === "text" ? "diff" : "text")?.find(undefined);
    const finder = this.views.finder(id, shown);
    if (!finder || !this.state.open.has(id)) return;
    finder.onFind((summary) => this.found(id, shown, summary));
    const at = this.at.get(id);
    finder.find(this.state.options, at && fromPathPos(this.views.paths(id, shown), at));
    if (this.landing === id) void this.land(id, shown, finder);
  }

  /** A pane found something: the bar shows it if it is the active tab's shown pane. */
  private found(id: TabId, pane: FindPane, summary: FindSummary) {
    if (pane !== this.pane(id) || !this.state.open.has(id)) return;
    const at = summary.at && toPathPos(this.views.paths(id, pane), summary.at);
    if (at) this.at.set(id, at);
    if (id === this.tabs.active) this.state.summary = summary;
  }

  /** Scrolls the tab's current match into view; a view rebuilt meanwhile starts the find again and does it again. */
  private async land(id: TabId, pane: FindPane, finder: Finder) {
    await finder.showFound();
    if (this.views.finder(id, pane) === finder && this.landing === id) this.landing = undefined;
  }

  /** The tabs, the active one or a view mode changed: the find follows to the pane the active tab shows. */
  sync() {
    for (const id of [...this.state.open]) {
      if (this.tabs.open.includes(id)) continue;
      this.state.open.delete(id);
      this.at.delete(id);
    }
    const id = this.tabs.active;
    if (this.state.open.has(id)) this.apply(id);
    else this.state.summary = NO_SUMMARY;
  }

  /** Opens the bar in the tab and puts the cursor in its field; a one-line `selected` becomes the query. */
  open(id: TabId, selected?: string) {
    if (selected && !/[\r\n]/.test(selected)) this.state.options.text = selected;
    this.state.open.add(id);
    this.state.focusRequests++;
    this.apply(id);
  }

  /** Closes the bar in the tab: the current match stays selected in the editor, which gets the focus. */
  close(id: TabId) {
    this.landing = undefined;
    this.typing.cancel();
    this.views.finder(id, this.pane(id))?.selectFound();
    this.state.open.delete(id);
    this.views.finder(id, "diff")?.find(undefined);
    this.views.finder(id, "text")?.find(undefined);
    this.state.summary = NO_SUMMARY;
  }

  /** Goes to the next or previous match of the tab, and says when that wrapped round the end. */
  async step(id: TabId, delta: 1 | -1) {
    this.landing = undefined;
    const stepped = await this.views.finder(id, this.pane(id))?.findStep(delta);
    if (stepped?.wrapped)
      this.state.wrapped = { text: delta > 0 ? "Wrapped to the start" : "Wrapped to the end", n: ++this.wraps };
  }

  setOptions(patch: Partial<FindOptions>) {
    Object.assign(this.state.options, patch);
    this.typing.cancel();
    if (Object.keys(patch).length === 1 && "text" in patch) {
      this.typing.schedule(FIND_TYPING_MS, () => this.apply(this.tabs.active));
      return;
    }
    this.apply(this.tabs.active);
  }

  /** Lands on a result of Find in Files: the bar opens in the tab with the search's query and the result as its current match. */
  landOn(id: TabId, options: FindOptions, at: { path: string; order: number; from: number }) {
    Object.assign(this.state.options, options);
    this.at.set(id, at);
    this.state.open.add(id);
    this.landing = id;
    this.apply(id);
  }

  /** Drops the timer of a query that waits (a torn-down screen). */
  cancel() {
    this.typing.cancel();
  }
}
