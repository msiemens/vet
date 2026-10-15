import assert from "node:assert/strict";
import { test } from "node:test";
import type { FindOptions } from "./editor/findMatches.ts";
import { type FindSummary, NO_SUMMARY } from "./editor/findView.ts";
import { FindControl, type FindPane } from "./FindControl.ts";
import type { FindState } from "./FindState.svelte";
import type { Tabs } from "./Tabs.svelte.ts";

/** A finder that records what it was asked and can be made to wrap or to answer. */
class FakeFinder {
  options: (FindOptions | undefined)[] = [];
  steps: (1 | -1)[] = [];
  shown = 0;
  selected = 0;
  wrapped = false;
  private listener?: (summary: FindSummary) => void;

  find(options: FindOptions | undefined) {
    this.options.push(options);
  }
  onFind(listener: (summary: FindSummary) => void) {
    this.listener = listener;
  }
  findStep(delta: 1 | -1) {
    this.steps.push(delta);
    return { wrapped: this.wrapped };
  }
  showFound() {
    this.shown++;
  }
  selectFound() {
    this.selected++;
    return true;
  }
  found(summary: FindSummary) {
    this.listener?.(summary);
  }
}

const options = (): FindOptions => ({
  text: "",
  matchCase: false,
  wholeWord: false,
  regex: false,
  side: "both",
  onlyChanges: false,
});

const summary = (n: number): FindSummary => ({ total: n, index: 0, unchanged: 0, pending: 0, skipped: 0 });

/** The find bar's own state, as plain objects where the real `FindState` uses runes. */
interface FakeState {
  options: FindOptions;
  open: Set<string>;
  summary: FindSummary;
  focusRequests: number;
  wrapped?: { text: string; n: number };
}

function setup(modes: Record<string, string>, barOpen: string[], active = "changes") {
  const state: FakeState = { options: options(), open: new Set(barOpen), summary: NO_SUMMARY, focusRequests: 0 };
  const tabs = { modes: new Map(Object.entries(modes)), open: Object.keys(modes), active };
  const finders = new Map<string, FakeFinder>();
  const key = (id: string, pane: FindPane) => `${id}:${pane}`;
  const finder = (id: string, pane: FindPane) => {
    const made = new FakeFinder();
    finders.set(key(id, pane), made);
    return made;
  };
  const control = new FindControl(state as unknown as FindState, tabs as unknown as Tabs, {
    finder: (id, pane) => finders.get(key(id, pane)),
    paths: () => ["a.rs"],
  });
  return { control, state, tabs, finder };
}

void test("the pane a tab is searched in is its text while it shows its file, its diff otherwise", () => {
  const { control } = setup({ changes: "unified", "file:a.rs": "file" }, ["changes"]);
  assert.equal(control.pane("changes"), "diff");
  assert.equal(control.pane("file:a.rs"), "text");
});

void test("a find runs in the shown pane and ends in the other; a closed bar ends both", () => {
  const { control, state, finder } = setup({ changes: "unified" }, ["changes"]);
  const diff = finder("changes", "diff");
  const text = finder("changes", "text");
  state.open.add("changes");
  control.apply("changes");
  assert.deepEqual(diff.options, [state.options], "the shown pane is searched");
  assert.deepEqual(text.options, [undefined], "the other pane's find ends");

  state.open.delete("changes");
  control.apply("changes");
  assert.equal(diff.options.length, 1, "a closed bar searches nothing");
});

void test("opening the bar takes a selection of one line as the query, and counts the focus request", () => {
  const { control, state } = setup({ "file:a.rs": "file" }, ["file:a.rs"]);
  control.open("file:a.rs", "needle");
  assert.equal(state.options.text, "needle");
  assert.equal(state.open.has("file:a.rs"), true);
  assert.equal(state.focusRequests, 1);

  control.open("file:a.rs", "two\nlines");
  assert.equal(state.options.text, "needle", "a multi-line selection is not the query");
});

void test("closing the bar leaves the current match selected in the editor and ends the find", () => {
  const { control, state, finder } = setup({ changes: "unified" }, ["changes"]);
  const diff = finder("changes", "diff");
  const text = finder("changes", "text");
  state.open.add("changes");
  state.summary = summary(3);
  control.close("changes");
  assert.equal(diff.selected, 1);
  assert.equal(state.open.has("changes"), false);
  assert.deepEqual(diff.options, [undefined]);
  assert.deepEqual(text.options, [undefined]);
  assert.deepEqual(state.summary, NO_SUMMARY);
});

void test("a step that wraps says so, a step that does not leaves the notice", async () => {
  const { control, state, finder } = setup({ changes: "unified" }, ["changes"]);
  const diff = finder("changes", "diff");
  state.open.add("changes");
  diff.wrapped = true;
  await control.step("changes", 1);
  assert.deepEqual(state.wrapped, { text: "Wrapped to the start", n: 1 });
  await control.step("changes", -1);
  assert.deepEqual(state.wrapped, { text: "Wrapped to the end", n: 2 });

  diff.wrapped = false;
  await control.step("changes", 1);
  assert.deepEqual(state.wrapped, { text: "Wrapped to the end", n: 2 }, "a step that did not wrap leaves the notice");
});

void test("the bar shows what the active, open tab's shown pane found", () => {
  const { control, state, tabs, finder } = setup({ changes: "unified", "file:a.rs": "unified" }, [
    "changes",
    "file:a.rs",
  ]);
  const diff = finder("changes", "diff");
  state.open.add("changes");
  control.apply("changes");
  diff.found(summary(3));
  assert.deepEqual(state.summary, summary(3));

  tabs.active = "file:a.rs"; // the find belongs to another tab
  state.summary = NO_SUMMARY;
  diff.found(summary(9));
  assert.deepEqual(state.summary, NO_SUMMARY);
});

void test("the find follows the tabs: a closed tab's bar goes and the bar is reset when none is open", () => {
  const { control, state, tabs } = setup({ changes: "unified", "file:a.rs": "unified" }, ["changes"], "changes");
  state.open.add("changes");
  state.open.add("file:a.rs");
  tabs.open = ["changes"]; // the file tab was closed
  state.summary = summary(4);
  control.sync();
  assert.equal(state.open.has("file:a.rs"), false, "the closed tab's bar is dropped");
  assert.deepEqual(state.summary, summary(4), "the active tab's bar is still shown");

  state.open.clear();
  control.sync();
  assert.deepEqual(state.summary, NO_SUMMARY, "with no bar open nothing is shown");
});

void test("a result of Find in Files opens the bar in the tab it landed in with the result as its match", () => {
  const { control, state, finder } = setup({ "file:a.rs": "file" }, ["file:a.rs"]);
  const text = finder("file:a.rs", "text");
  control.landOn("file:a.rs", { ...options(), text: "hit" }, { path: "a.rs", order: 2, from: 3 });
  assert.equal(state.options.text, "hit");
  assert.equal(state.open.has("file:a.rs"), true);
  assert.equal(text.options.length, 1, "the find started");
});

void test("a change of query waits while any other change searches at once; a cancel drops the wait", async () => {
  const { control, state, finder } = setup({ changes: "unified" }, ["changes"]);
  const diff = finder("changes", "diff");
  state.open.add("changes");
  control.setOptions({ matchCase: true });
  assert.equal(state.options.matchCase, true);
  assert.equal(diff.options.length, 1, "a non-text change searches at once");

  control.setOptions({ text: "a" });
  assert.equal(diff.options.length, 1, "typing waits");
  control.cancel();
  await new Promise((resolve) => setTimeout(resolve, 200));
  assert.equal(diff.options.length, 1, "a cancelled wait does not search");

  control.setOptions({ text: "ab" });
  await new Promise((resolve) => setTimeout(resolve, 200));
  assert.equal(diff.options.length, 2, "a wait that is not cancelled searches");
});
