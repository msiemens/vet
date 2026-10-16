import assert from "node:assert/strict";
import { test } from "node:test";
import type { CommitUnit } from "../../generated/CommitUnit";
import type { ScopeSpec } from "../../state/scopeSpec";
import type { Answer } from "../../state/reviewMarks";
import type { WriteListener } from "../../state/reviewSource";
import type { ViewScope, ViewSource } from "../../state/viewScope.svelte";
import type { ChangesetSource } from "./changesetSource";
import type { Tabs } from "./Tabs.svelte.ts";
import { ViewScopes } from "./ViewScopes.ts";

/** A scope the registry can hold: only what the registry reads or calls. */
class FakeScope {
  readonly loads = { n: 0 };
  readonly answers = { n: 0 };
  readonly offs = { n: 0 };
  private listeners = new Set<WriteListener>();

  readonly spec: ScopeSpec;
  readonly commit?: CommitUnit;
  private listeners = new Set<WriteListener>();

  constructor(spec: ScopeSpec, commit?: CommitUnit) {
    this.spec = spec;
    this.commit = commit;
  }

  load() {
    this.loads.n++;
    return Promise.resolve();
  }
  subscribe(listener: WriteListener) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
      this.offs.n++;
    };
  }
  heard() {
    this.answers.n++;
  }
  /** Tells the scope's listeners what was written, as a manifest does. */
  written(paths: ReadonlySet<string> | "all" = "all") {
    for (const listener of this.listeners) listener(paths);
  }
}

/** A changeset stand-in: a commit `shownCommit`, the layout `currentLayout` returns and the files `layoutOf` knows. */
function changesetOf(init: { shownCommit?: number; layout?: { interdiff?: { markable: boolean } } } = {}) {
  return {
    shownCommit: init.shownCommit,
    currentLayout: (): unknown => init.layout,
    layoutOf: (path: string) => (path === "a.rs" ? {} : undefined),
    adopt: () => undefined,
  };
}

/** A whole-changeset view stand-in: only what the registry subscribes to. */
function wholeOf() {
  const listeners = new Set<WriteListener>();
  return {
    subscribe(listener: WriteListener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  } as unknown as ViewSource;
}

/** A registry over fake tabs, a fake changeset and a fake scope factory; answers the scopes it made. */
function registry(open: string[], changeset = changesetOf()) {
  const made: FakeScope[] = [];
  const told: { scope: ViewScope; paths: ReadonlySet<string> | "all" }[] = [];
  const tabs = { open };
  const whole = wholeOf();
  const scopes = new ViewScopes(
    changeset as unknown as ChangesetSource,
    whole,
    tabs as unknown as Tabs,
    (scope, paths) => told.push({ scope, paths }),
    (spec) => {
      const scope = new FakeScope(spec);
      made.push(scope);
      return scope as unknown as ViewScope;
    },
  );
  return { scopes, changeset, tabs, whole, made, told };
}

void test("a tab's scope is the view it reads; a tab on the view on screen reads the store", () => {
  const { scopes, changeset, whole, made } = registry(["changes", "file:a.rs", "pin:beef:file.rs"]);
  assert.equal(scopes.specOf("changes"), undefined);
  assert.equal(scopes.specOf("file:a.rs"), undefined);
  assert.deepEqual(scopes.specOf("pin:beef:file.rs"), { type: "commit", id: "beef" });
  assert.equal(scopes.sourceOf("file:a.rs"), changeset, "no scope reads the store itself");
  assert.equal(scopes.sourceOf("pin:beef:file.rs"), scopes.sourceOf("pin:beef:file.rs"));
  assert.equal(made.length, 1, "the same view's scope is made once");

  // A commit is on screen: a file tab then reads the changeset's whole view, not the commit's.
  changeset.shownCommit = 2;
  assert.deepEqual(scopes.specOf("file:a.rs"), { type: "changeset" });
  assert.equal(scopes.showsCommit("file:a.rs"), false, "it reads the changeset, not the commit on screen");
  assert.equal(scopes.showsCommit("pin:beef:file.rs"), true);
  assert.equal(scopes.showsCommit("changes"), true);
  assert.equal(scopes.sourceOf("file:a.rs"), whole, "a whole tab reads the store's whole view");
  assert.equal(made.length, 1, "no scope is made for the whole view");
});

void test("a scope reads when it is made, and again when the changeset moved on", () => {
  const { scopes, made } = registry(["changes", "pin:beef:file.rs"]);
  scopes.sourceOf("pin:beef:file.rs");
  assert.equal(made[0].loads.n, 1);

  scopes.reload();
  assert.equal(made[0].loads.n, 2);
});

void test("a scope no open tab reads is dropped when the tabs are pruned", () => {
  const { scopes, tabs, made } = registry(["changes", "pin:beef:file.rs"]);
  scopes.sourceOf("pin:beef:file.rs");
  tabs.open = ["changes"]; // the tab was closed
  scopes.prune();
  assert.equal(made[0].offs.n, 1, "the dropped scope's listener was taken off");
  assert.equal(made.length, 1);
});

void test("a scope's write reaches the registry, and the backend's answer reaches every scope", () => {
  const { scopes, made, told } = registry(["changes", "pin:beef:file.rs", "pin:cafe:file.rs"]);
  const one = scopes.sourceOf("pin:beef:file.rs") as unknown as FakeScope;
  scopes.sourceOf("pin:cafe:file.rs");
  one.written();
  assert.deepEqual(told, [{ scope: one as unknown as ViewScope, paths: "all" }]);

  scopes.heard({} as Answer, 7);
  assert.equal(one.answers.n, 1);
  assert.equal(made[1].answers.n, 1);
});

void test("dropping takes every listener off and lets the scopes be made again", () => {
  const { scopes, made } = registry(["changes", "pin:beef:file.rs"]);
  scopes.sourceOf("pin:beef:file.rs");
  scopes.drop();
  assert.equal(made[0].offs.n, 1, "dropping takes the listener off");
  assert.equal(made.length, 1);
  scopes.heard({} as Answer, 1);
  assert.equal(made[0].answers.n, 0, "a dropped scope hears nothing");
  const again = scopes.sourceOf("pin:beef:file.rs");
  assert.notEqual(again, made[0]);
  assert.equal(made.length, 2);
});

void test("the semantic layer has the target side of the changeset but not of a commit or an unmarkable range", () => {
  const { scopes, changeset } = registry(["changes", "pin:beef:file.rs", "file:a.rs"]);
  assert.equal(scopes.analysable("changes", "target"), true);
  assert.equal(scopes.analysable("pin:beef:file.rs", "target"), false, "a commit's snapshots are not the changeset's");

  // The view on screen shows a range that does not end at the latest version: only its new side is read.
  changeset.currentLayout = () => ({ interdiff: { markable: false } });
  assert.equal(scopes.analysable("changes", "target"), false);
  assert.equal(scopes.analysable("file:a.rs", "target"), false);
});
