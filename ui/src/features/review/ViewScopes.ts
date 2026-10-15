// What each tab reads: the changeset store for the tabs that show the view on screen, and a `ViewScope`
// of its own for a tab that stays at a commit (or on the whole changeset while a commit is shown). One
// registry makes each view's scope once and drops it with the last tab that reads it.

import type { CommitUnit } from "../../generated/CommitUnit";
import type { Side } from "../../generated/Side";
import type { TabId } from "../../state/commands";
import type { Answer } from "../../state/reviewMarks";
import { type ScopeSpec, scopeKey } from "../../state/scopeSpec.ts";
import type { Shown } from "../../state/shown";
import type { ViewScope, ViewSource } from "../../state/viewScope.svelte";
import type { Tabs } from "./Tabs.svelte";
import { commitOf, kindOf, pathOf } from "./tabModel.ts";

/** What the registry reads of the open changeset; the store itself is the view on screen's source. */
export type ScopedChangeset = ViewSource & {
  shownCommit: number | undefined;
  adopt(shown: Shown, update: Answer): void;
};

/** Makes the scope of a view; the real one is `newScope`, plugged in so this module reaches no runes. */
export type ScopeFactory = (spec: ScopeSpec, marked: (shown: Shown, update: Answer) => void) => ViewScope;

/** Anything a scope read; the views of the tabs that read it follow. */
export type ScopeListener = (scope: ViewScope, paths: ReadonlySet<string> | "all") => void;

export class ViewScopes {
  /** The scopes held, by `scopeKey`. */
  private scopes = new Map<string, { scope: ViewScope; off: () => void }>();
  /** Another changeset is on its way: the open tabs are the old one's, and no scope may read for them. */
  private frozen = false;

  private changeset: ScopedChangeset;
  private tabs: Tabs;
  private written: ScopeListener;
  private makeScope: ScopeFactory;

  constructor(changeset: ScopedChangeset, tabs: Tabs, written: ScopeListener, makeScope: ScopeFactory) {
    this.changeset = changeset;
    this.tabs = tabs;
    this.written = written;
    this.makeScope = makeScope;
  }

  get isFrozen(): boolean {
    return this.frozen;
  }

  /** Holds (`true`) or lets go of (`false`) the scopes while another changeset is on its way. */
  setFrozen(frozen: boolean) {
    this.frozen = frozen;
  }

  /**
   * What the scope of a tab is, if it is not the view on screen: a tab at a commit stays there, and a
   * file tab shows the changeset while a commit is on screen.
   */
  specOf(id: TabId): ScopeSpec | undefined {
    const commit = commitOf(id);
    if (commit !== undefined) return { type: "commit", id: commit };
    return kindOf(id) === "file" && this.changeset.shownCommit !== undefined ? { type: "changeset" } : undefined;
  }

  /** What the tab reads its file and review state from. */
  sourceOf(id: TabId): ViewSource {
    const spec = this.specOf(id);
    return spec ? this.scope(spec) : this.changeset;
  }

  /** The scope of `spec`, made once and kept until the last tab that reads it goes. */
  scope(spec: ScopeSpec): ViewScope {
    const key = scopeKey(spec);
    const held = this.scopes.get(key);
    if (held) return held.scope;
    // What a mark in a tab got is the view on screen's own answer too, if it shows the same commit.
    const scope = this.makeScope(spec, (shown, update) => this.changeset.adopt(shown, update));
    const off = scope.subscribe((paths) => this.written(scope, paths));
    this.scopes.set(key, { scope, off });
    if (!this.frozen) void scope.load();
    return scope;
  }

  /** The scope of a tab at a commit; none for the tabs that read the view on screen. */
  commitScope(id: TabId): ViewScope | undefined {
    const commit = commitOf(id);
    return commit === undefined ? undefined : this.scope({ type: "commit", id: commit });
  }

  /** The commit a file tab stays at, once it is read. */
  commitUnit(id: TabId): CommitUnit | undefined {
    return this.commitScope(id)?.commit;
  }

  /** Whether the tab's file is changed in the view the tab reads. */
  fileChanged(id: TabId): boolean {
    const path = pathOf(id);
    return path !== undefined && this.sourceOf(id).layoutOf(path) !== undefined;
  }

  /** Whether the view the tab reads still changes the file, or does not say yet. */
  stillChanged(id: TabId, path: string): boolean {
    const source = this.sourceOf(id);
    return source.currentLayout() === undefined || source.layoutOf(path) !== undefined;
  }

  /** Whether the tab has a diff pane: the Changes tab, and a file's tab while its view changes the file or is not read yet, so the pane keeps its place across a switch of views. */
  hasDiffPane(id: TabId): boolean {
    const path = pathOf(id);
    return path === undefined || this.stillChanged(id, path);
  }

  /** The view the tab shows is of a commit, not of the changeset as a whole. */
  showsCommit(id: TabId): boolean {
    const spec = this.specOf(id);
    return spec ? spec.type === "commit" : this.changeset.shownCommit !== undefined;
  }

  /** Whether the semantic layer has `side` of what the tab shows: not in a commit, whose snapshots are not the changeset's. */
  analysable(id: TabId, side: Side): boolean {
    if (this.showsCommit(id)) return false;
    if (this.specOf(id)) return true;
    const range = this.changeset.currentLayout()?.interdiff;
    return !range || (range.markable && side === "target");
  }

  /** The tabs that read `scope`. */
  tabsOf(scope: ViewScope): TabId[] {
    return this.tabs.open.filter((id) => this.specOf(id) !== undefined && this.sourceOf(id) === scope);
  }

  /** The backend announced a mark of a changeset's file: every scope takes it (a commit's reads again). */
  heard(update: Answer, revision: number) {
    for (const { scope } of this.scopes.values()) scope.heard(update, revision);
  }

  /** Drops the scopes no open tab reads (a tab closed). */
  prune() {
    const used = new Set(this.tabs.open.flatMap((id) => this.specOf(id) ?? []).map(scopeKey));
    for (const [key, { off }] of this.scopes) {
      if (used.has(key)) continue;
      off();
      this.scopes.delete(key);
    }
  }

  /** Reads the scopes the open tabs need again, as the changeset moved on. */
  reload() {
    this.prune();
    if (this.frozen) return;
    for (const { scope } of this.scopes.values()) void scope.load();
  }

  /** Drops every scope, as another changeset is brought on view. */
  drop() {
    for (const { off } of this.scopes.values()) off();
    this.scopes.clear();
  }

  /** Starts reading the scopes the open tabs need. */
  ensure() {
    for (const id of this.tabs.open) this.sourceOf(id);
  }
}
