// Ties the backend and the views of the open tabs together, and registers the commands that act
// on the active view. Each tab's pane hands its scroll root to `attach`; the controller builds
// the tab's view in it.

import { tick, untrack } from "svelte";
import type { CheckLog } from "../../generated/CheckLog";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { CheckRun } from "../../generated/CheckRun";
import type { CommitUnit } from "../../generated/CommitUnit";
import type { SearchScope } from "../../generated/SearchScope";
import type { Side } from "../../generated/Side";
import type { View } from "../../generated/View";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { TextSpan } from "../../generated/TextSpan";
import { isAbort } from "../../lib/abort";
import { Debounced } from "../../lib/debounced";
import type { LocationRow } from "../../generated/LocationRow";
import { nextPaint } from "../../lib/nextPaint";
import { platformName } from "../../lib/platformName";
import { preferences } from "../../lib/preferences.svelte";
import type { Backend } from "../../state/backend";
import { browse } from "../../state/browse";
import type { Changeset } from "../../state/changeset.svelte";
import { contextMenus } from "../../state/contextMenu.svelte";
import { findSets } from "../../state/findSets.svelte";
import { notices } from "../../state/notices.svelte";
import { platform } from "../../state/platform.svelte";
import { locationPicker } from "../../state/locationPicker.svelte";
import { LocationSearch } from "../../state/locationSearch";
import type { FileLayout, Frame, ViewLayout } from "../../state/reviewModel";
import { reviewShown } from "../../state/reviewShown.svelte";
import { stepInFile } from "../../state/problemRows";
import { problems } from "../../state/problems.svelte";
import { semantic } from "../../state/semantic.svelte";
import { servedBy } from "../../state/servedBy.svelte";
import { serverList } from "../../state/serverList.svelte";
import { withoutFeature } from "../../state/serverStatus";
import { session } from "../../state/session.svelte";
import { type Shown, isCurrent } from "../../state/shown";
import { sameView } from "../../state/view";
import { type StructureTarget, structure } from "../../state/structure.svelte";
import { symbols } from "../../state/symbols.svelte";
import { threads } from "../../state/threads.svelte";
import { rangeKey } from "../../state/timelineRange";
import { type ScopeSpec, type ViewScope, type ViewSource, newScope, scopeKey } from "../../state/viewScope.svelte";
import type { Thread } from "../../generated/Thread";
import { SUBMIT_KEY } from "../../lib/composer";
import { STALE_NOTICE } from "../../lib/notice";
import { RequestError } from "../../lib/requestError";
import { ChangesetView, type Listeners, type PanePosition, type ViewHooks } from "./editor/ChangesetView";
import { fileSource } from "./editor/fileSource";
import { type FindOptions, type PathPos, fromPathPos, orderOf, toPathPos } from "./editor/findMatches";
import type { OpenedResult } from "../search";
import { routeFound } from "./foundRoute";
import { type Finder, NO_SUMMARY, type FindSummary } from "./editor/findView";
import { FindState } from "./FindState.svelte";
import type { Rendered } from "./editor/rendered";
import { LogView } from "./editor/LogView";
import { type ThreadActions, buildCard } from "./editor/threadCard";
import type { ThreadListeners } from "./editor/threadLayer";
import { bindRevision, focusedEditor } from "./editor/semantic";
import type { SoftWrap } from "../../lib/softWrap";
import type { Mode, ViewMode } from "../../lib/viewMode";
import { type CommandContext, type TabId, commands } from "../../state/commands";
import { type TextActions, TextView } from "./editor/TextView";
import { type FilePositions, type SavedView, ViewMemory, restoredTabs } from "./changesetViews";
import { keyId } from "../../state/changesetKey";
import { threadMenu } from "../comments";
import { type HunkPlace, stepFlagged } from "./flaggedHunks";
import { problemTab } from "./problemTab";
import { provideActions } from "./reviewActions";
import type { ViewActions } from "./reviewCommands";
import {
  CHANGES,
  ON_SCREEN,
  type Origin,
  commitOf,
  fileTab,
  hasDiff,
  kindOf,
  logTab,
  modesOf,
  pathOf,
  settleMode,
} from "./tabModel";
import { Tabs } from "./Tabs.svelte";

/** Space types in a field and presses a focused control; it marks the current item only when none of those has the focus. */
const takesSpace = (el: Element | null) =>
  !!el?.closest("button, a, input, textarea, select, [contenteditable], [tabindex], [role=menuitem], [role=tab]");

/** What the controller reads of the open changeset and asks of it. */
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
  | "refresh"
  | "reportFailure"
>;

const NO_HOVER = { text: "", code: [] };

/** Where the views of the changesets of the repository on view are kept. */
const projectViews = {
  read: () => preferences.read("project", "tabs"),
  write: (value: string | undefined) => preferences.write("project", "tabs", value),
};

const FIND_TYPING_MS = 120;

/** The tabs and positions are saved this long after the last change. */
const SAVE_AFTER_MS = 400;

/** Said when a mark or a comment is asked for in a range that does not end at the latest version. */
export const READ_ONLY_NOTE = "This range does not end at the latest version: marks and comments are off";

export class ReviewController {
  readonly tabs = new Tabs("unified");
  /** The find bar: the window's query, the tabs it is open in, and what the active tab's view found. */
  readonly find = new FindState();
  /** Where the current match is in each tab, which a rebuilt view starts from; by file, since the files before it come and go. */
  private findAt = new Map<TabId, PathPos>();
  private wraps = 0;
  /** The tab whose current match is to be shown, as long as its view may still be replaced. */
  private landing?: TabId;
  /** The diff view of each attached tab; a file tab has none while its file is not in the changeset. */
  private views = new Map<TabId, ChangesetView>();
  /** The text view of each file tab that showed its text; built when the tab first shows it. */
  private texts = new Map<TabId, TextView>();
  /** The logs fetched for the log tabs, which are built from them whenever their pane attaches. */
  private logs = new Map<TabId, CheckLog>();
  /** The view of each attached log tab. */
  private logViews = new Map<TabId, LogView>();
  /** The scroll root of each attached tab's diff (or log) view. */
  private roots = new Map<TabId, HTMLElement>();
  /** The scroll root of each attached file tab's text view. */
  private textRoots = new Map<TabId, HTMLElement>();
  /** The diff mode chosen last, which a diff tab opens in. */
  private diffMode: Mode = "unified";
  /** Which files the views wrap the long lines of. */
  softWrap: SoftWrap = "prose";
  /** The views that file tabs read while they are not the one on screen, by `scopeKey`. */
  private scopes = new Map<string, { scope: ViewScope; off: () => void }>();
  /** Another changeset is on its way: the open tabs are the old one's, and no scope may read for them. */
  private frozen = false;
  /** Show Usages and Go to Implementation: goes to the one place found, or lists the places in the popup. */
  private search = new LocationSearch({
    picker: locationPicker,
    notice: (text) => notices.show(text),
    isAbort,
    fail: (error, revision) => this.changeset.reportFailure(error, revision),
    restartServer: (language) => serverList.restart(language),
  });
  /** The changeset on view, by `keyId`; unknown before the first manifest. */
  private key?: string;
  /** The range the views show, if they show one. */
  private shown?: { from: number; to: number; revision: number };
  /** The files the Changes tab was limited to when it was built, if it was. */
  private filtered?: ReadonlySet<string>;
  /** The commit the views show, if they show one. */
  private shownCommit?: number;
  /** What each changeset of the repository had open when it was last on view, for when it comes back. */
  private memory?: ViewMemory;
  /** Scroll positions the next builds start at, instead of carrying the old views' over. */
  private restore?: { scroll: Map<TabId, number> };
  /** Where file tabs' panes are put when they are built: the positions of the changeset just brought back. */
  private pending = new Map<TabId, FilePositions>();
  private saving = new Debounced();
  /** The Changes tab of the views built last going to its first hunk. */
  private arriving: Promise<void> = Promise.resolve();

  constructor(
    readonly backend: Backend,
    private changeset: ChangesetSource,
    /** Context menu on a file's header in a view. */
    private fileMenu: (path: string, x: number, y: number) => void,
  ) {
    semantic.reset(); // the controller belongs to one repository's screen
    const offReview = changeset.subscribe((paths) => this.refreshReview(paths));
    const offThreads = threads.onChange(() => this.showThreads());
    const offCommands = provideActions(this.viewActions());
    const offContext = commands.provide(() => this.facts());
    const offResync = session.onResync(() => this.reloadScopes());
    const offOrigins = semantic.onOrigins((path) => this.refreshDiagnostics(path));
    const offShown = reviewShown.onShown(() => this.refreshDiagnostics());
    // The open tabs are the old changeset's until the new one's manifest has come and brought its tabs back.
    const offOpening = changeset.onOpening((opening) => {
      if (opening.type === "requested") {
        this.dropScopes();
        this.frozen = true;
      } else if (opening.type === "unannounced") {
        this.frozen = false;
        this.reloadScopes();
      }
    });
    this.unsubscribe = () => {
      offReview();
      offThreads();
      offCommands();
      offContext();
      offResync();
      offOrigins();
      offShown();
      offOpening();
    };
    this.unsubscribeEvents = backend.onEvent((event) => {
      if (event.project !== backend.project) return;
      // A server installed or reconfigured elsewhere may answer differently now.
      if (event.type === "serversChanged") {
        semantic.invalidate();
        this.refreshDiagnostics();
      } else if (event.type === "reviewChanged") {
        for (const { scope } of this.scopes.values()) scope.heard(event.update, event.revision);
      } else if (event.type === "snapshotSettled" || event.type === "targetChanged") {
        this.reloadScopes();
      }
    });
  }

  private unsubscribe: () => void;
  private unsubscribeEvents: () => void;

  /** The mounted editors ask for their diagnostics again (those of `path` only, if given). */
  private refreshDiagnostics(path?: string) {
    for (const view of this.views.values()) view.refreshDiagnostics(path);
    for (const text of this.texts.values()) text.refreshDiagnostics(path);
  }

  /**
   * The semantic layer of the tab `id`, which reads the changeset's snapshots: not in a commit, and
   * in a range only the new side of the latest version is one.
   */
  private hooksFor(id: TabId): ViewHooks {
    return {
      hover: (revision, side, path, line, col, signal) =>
        this.analysable(id, side) ? semantic.hover(revision, side, path, line, col, signal) : Promise.resolve(NO_HOVER),
      definition: (revision, side, path, line, col) => {
        if (this.analysable(id, side)) void this.goToDefinition(id, revision, side, path, line, col);
      },
      hasDefinition: (revision, side, path, line, col, signal) =>
        this.analysable(id, side)
          ? semantic.hasDefinition(revision, side, path, line, col, signal)
          : Promise.resolve(false),
      locations: (revision, kind, side, path, at, subject) => {
        if (!this.analysable(id, side)) return;
        const backend = this.backend;
        void this.search.start({
          kind,
          symbol: subject.name,
          anchor: subject.anchor,
          side,
          revision,
          open: (row) => void this.openRow(id, row, side),
          keep:
            kind === "usages"
              ? (answer) => findSets.keepUsages({ type: "usages", symbol: subject.name, side, path, ...at }, answer)
              : undefined,
          ask: (signal) =>
            kind === "usages"
              ? backend.usages(revision, side, path, at.line, at.col, signal)
              : backend.implementations(revision, side, path, at.line, at.col, signal),
        });
      },
      findUsages: (_revision, side, path, at, subject) => {
        if (this.analysable(id, side)) findSets.findUsages({ type: "usages", symbol: subject.name, side, path, ...at });
      },
      // Asked again for the mounted editors when the review screen comes up.
      diagnostics: (revision, path) =>
        reviewShown.on && this.analysable(id, "target") ? semantic.diagnostics(revision, path) : Promise.resolve([]),
      cursor: (revision, side, path, line, col) => {
        if (this.analysable(id, side)) symbols.follow(revision, side, path, line, col);
      },
    };
  }

  /** What the scope of a tab is, if it is not the view on screen: a tab at a commit stays there, and a file tab shows the changeset while a commit is on screen. */
  private specOf(id: TabId): ScopeSpec | undefined {
    const commit = commitOf(id);
    if (commit !== undefined) return { type: "commit", id: commit };
    return kindOf(id) === "file" && this.changeset.shownCommit !== undefined ? { type: "changeset" } : undefined;
  }

  /** What the tab reads its file and review state from. */
  private sourceOf(id: TabId): ViewSource {
    const spec = this.specOf(id);
    return spec ? this.scope(spec) : this.changeset;
  }

  private scope(spec: ScopeSpec): ViewScope {
    const key = scopeKey(spec);
    const held = this.scopes.get(key);
    if (held) return held.scope;
    // What a mark in a tab got is the view on screen's own answer too, if it shows the same commit.
    const scope = newScope(this.backend, spec, (shown, update) => this.changeset.adopt(shown, update));
    const off = scope.subscribe((paths) => this.scopeWritten(scope, paths));
    this.scopes.set(key, { scope, off });
    if (!this.frozen) void scope.load();
    return scope;
  }

  /** The scope of a tab at a commit; none for the tabs that read the view on screen. */
  private commitScope(id: TabId): ViewScope | undefined {
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

  /** Whether the tab has a diff pane: the Changes tab, and a file's tab while its view changes the file or is not read yet, so the pane keeps its place across a switch of views. */
  hasDiffPane(id: TabId): boolean {
    const path = pathOf(id);
    return path === undefined || this.stillChanged(id, path);
  }

  /** Whether the view the tab reads still changes the file, or does not say yet. */
  private stillChanged(id: TabId, path: string): boolean {
    const source = this.sourceOf(id);
    return source.currentLayout() === undefined || source.layoutOf(path) !== undefined;
  }

  /** Drops the scopes no open tab reads (a tab closed). */
  pruneScopes() {
    const used = new Set(this.tabs.open.flatMap((id) => this.specOf(id) ?? []).map(scopeKey));
    for (const [key, { off }] of this.scopes) {
      if (used.has(key)) continue;
      off();
      this.scopes.delete(key);
    }
  }

  /** Reads the scopes the open tabs need again, as the changeset moved on. */
  private reloadScopes() {
    this.pruneScopes();
    if (this.frozen) return;
    for (const { scope } of this.scopes.values()) void scope.load();
  }

  private dropScopes() {
    for (const { off } of this.scopes.values()) off();
    this.scopes.clear();
  }

  /** Starts reading the scopes the open tabs need. */
  private ensureScopes() {
    for (const id of this.tabs.open) this.sourceOf(id);
  }

  /**
   * The file the Structure tool shows and the view it reads it in: the active file tab's, the file
   * at the top of the Changes tab, none for a log. Reading it subscribes to the view's layout.
   */
  structureTarget(): StructureTarget | undefined {
    const id = this.tabs.active;
    const path = pathOf(id) ?? (id === CHANGES ? this.tabs.changesFile : undefined);
    const shown = path === undefined ? undefined : this.sourceOf(id).currentShown();
    if (path === undefined || !shown) return undefined;
    // The Symbol panel works on the changeset's snapshots, which no commit on screen has.
    const panel =
      shown.view.type === "changeset" && this.changeset.shownCommit === undefined && this.analysable(id, "target");
    const caret = () =>
      this.tabs.modes.get(id) === "file" ? this.texts.get(id)?.caret() : this.views.get(id)?.caretIn(path);
    return { tab: id, path, commit: commitOf(id), shown, panel, caret };
  }

  /** Gives the keyboard focus back to the editor of the active tab, where the Structure window or popup took it from. */
  focusEditor() {
    const id = this.tabs.active;
    const path = pathOf(id) ?? (id === CHANGES ? this.tabs.changesFile : undefined);
    if (this.tabs.modes.get(id) === "file") this.texts.get(id)?.focusEditor();
    else if (path !== undefined) this.views.get(id)?.focusEditor(path);
  }

  /** Shows a symbol of the Structure list where it is declared, in the file's tab (the base side's text for a removed one). */
  async jumpToOutlineSymbol(symbol: SymbolInfo, removed: boolean) {
    await this.openLocation(this.tabs.active, symbol.id.path, symbol.at.line, removed ? "base" : "target");
  }

  /** The tabs that read `scope`. */
  private tabsOf(scope: ViewScope): TabId[] {
    return this.tabs.open.filter((id) => this.specOf(id) !== undefined && this.sourceOf(id) === scope);
  }

  /** A scope read its manifest or took review state: the views of its tabs follow. */
  private scopeWritten(scope: ViewScope, paths: ReadonlySet<string> | "all") {
    if (scope.gone) {
      // The commit is not among the changeset's any more (a rewritten branch): its tabs have nothing to show.
      for (const id of this.tabsOf(scope)) this.tabs.close(id);
      return;
    }
    const layout = scope.currentLayout();
    if (!layout) return;
    this.tabs.dropStale((id, path) => this.stillChanged(id, path));
    for (const id of this.tabsOf(scope)) {
      const view = this.views.get(id);
      if (view && isCurrent(view.shown, layout.shown)) view.refreshReview(paths);
      else if (this.roots.has(id)) this.build(id, view);
      const text = this.texts.get(id);
      const root = this.textRoots.get(id);
      if (text?.isFor(layout.shown)) text.refreshReview(paths);
      else if (root && (text || this.tabs.modes.get(id) === "file")) this.buildText(id, root);
    }
  }

  /** Detaches the controller from the window and the store, and tears down the views. */
  destroy() {
    this.search.cancel();
    this.saving.cancel();
    this.findTyping.cancel();
    this.saveView();
    this.unsubscribe();
    this.unsubscribeEvents();
    this.dropScopes();
    for (const view of this.views.values()) view.destroy();
    this.views.clear();
    for (const text of this.texts.values()) text.destroy();
    this.texts.clear();
    for (const log of this.logViews.values()) log.destroy();
    this.logViews.clear();
  }

  /**
   * The view a request that starts from `origin` reads: the tab's own (see `sourceOf`), or for
   * `ON_SCREEN` the one on screen. Whatever a tab asks for goes through here, never to `changeset`
   * directly: a tab may read another view than the one on screen.
   */
  private viewOf(origin: Origin): ViewSource {
    return origin === ON_SCREEN ? this.changeset : this.sourceOf(origin);
  }

  /** The full id of the commit the view `origin` reads is of; none if it is of the changeset. */
  private async commitAt(origin: Origin): Promise<string | undefined> {
    if (this.viewOf(origin) === this.changeset) return this.commitOnScreen();
    return origin === ON_SCREEN ? undefined : commitOf(origin);
  }

  /** The view on screen shows a range that does not end at the latest version: nothing in it can be marked or commented. */
  private get screenReadOnly(): boolean {
    return this.changeset.currentLayout()?.interdiff?.markable === false;
  }

  /** The view the tab shows is of a commit, not of the changeset as a whole. */
  private showsCommit(id: TabId): boolean {
    const spec = this.specOf(id);
    return spec ? spec.type === "commit" : this.changeset.shownCommit !== undefined;
  }

  /** Whether the semantic layer has `side` of what the tab shows: not in a commit, whose snapshots are not the changeset's. */
  private analysable(id: TabId, side: Side): boolean {
    if (this.showsCommit(id)) return false;
    if (this.specOf(id)) return true;
    const range = this.changeset.currentLayout()?.interdiff;
    return !range || (range.markable && side === "target");
  }

  /** The threads the tab shows: those of the changeset, and none in a commit, where they would sit in the wrong place. */
  private shownThreads(id: TabId): Thread[] {
    return this.showsCommit(id) ? [] : threads.items;
  }

  /** Runs a request that marks or comments in `source`, unless the view on screen is read-only: then it says so and `refuse` answers. */
  private async editing<T>(source: ViewSource, request: () => Promise<T>, refuse: () => T): Promise<T> {
    if (source !== this.changeset || !this.screenReadOnly) return request();
    notices.show(READ_ONLY_NOTE);
    return refuse();
  }

  /** The Changes tab's scroll root. */
  get changesRoot(): HTMLElement {
    const root = this.roots.get(CHANGES);
    if (!root) throw new Error("the Changes tab is not mounted");
    return root;
  }

  /** The Changes tab's view; throws before the first `open`. */
  get changes(): ChangesetView {
    const view = this.views.get(CHANGES);
    if (!view) throw new Error("no changeset view is mounted");
    return view;
  }

  /** The diff view of the focused tab, unless the tab shows its file's text. */
  private get active(): ChangesetView | undefined {
    return this.activeMode === "file" ? undefined : this.views.get(this.tabs.active);
  }

  /** The text of the focused tab, if it shows it and the file is a changed one with hunks to step through. */
  private get activeText(): TextView | undefined {
    const text = this.activeMode === "file" ? this.texts.get(this.tabs.active) : undefined;
    return text?.hasHunks ? text : undefined;
  }

  /** What the active tab shows; none for a log. */
  get activeMode(): ViewMode | undefined {
    return this.tabs.modes.get(this.tabs.active);
  }

  /**
   * The full id of the commit on screen, if one is. The commits are asked for rather than taken from
   * the store, whose list may not be of this revision yet; a reply of another revision, or a commit
   * that changed meanwhile, is an error.
   */
  private async commitOnScreen(): Promise<string | undefined> {
    const index = this.changeset.shownCommit;
    const shown = this.changeset.currentShown();
    if (index === undefined || !shown) return undefined;
    const { revision, commits } = await this.backend.commits();
    const id =
      revision === shown.revision && this.changeset.shownCommit === index ? commits[index]?.commit.id : undefined;
    if (id === undefined) throw new RequestError("outdated", STALE_NOTICE);
    return id;
  }

  /** The facts about the review screen that commands read. */
  private facts(): Partial<CommandContext> {
    const id = this.tabs.active;
    const path = pathOf(id);
    const view = this.active;
    const text = this.activeText;
    const editor = focusedEditor(document.activeElement);
    const file = editor?.path ?? path;
    return {
      tab: id,
      tabCount: this.tabs.open.length,
      viewMode: this.activeMode,
      viewModes: modesOf(kindOf(id), this.fileChanged(id)),
      diffView: view !== undefined || text !== undefined,
      noComments: commitOf(id) !== undefined,
      byCommit: this.changeset.shownCommit !== undefined,
      hunk: view?.current ?? text?.current ?? -1,
      findOpen: this.find.open.has(id),
      editor,
      noImplementations:
        editor && withoutFeature(servedBy.language(editor.path), serverList.servers, "implementations"),
      noUsages: this.showsCommit(id),
      serverLanguage: file === undefined ? undefined : servedBy.language(file),
    };
  }

  private viewActions(): ViewActions {
    return {
      step: (delta) => this.stepHunk(delta),
      toggleSeen: () => {
        const text = this.activeText;
        if (text) return text.toggleCurrent();
        const view = this.active;
        return !!view && this.toggleBySpace(view);
      },
      toggleFlag: () => {
        const text = this.activeText;
        if (text) return text.toggleFlagCurrent();
        return this.active?.toggleFlagCurrent() !== undefined;
      },
      stepFlag: (delta) => this.stepFlagged(delta > 0 ? 1 : -1),
      firstFlagged: () => this.goToFlagged(this.flaggedFrom(undefined, 1)),
      reconfirm: () => {
        const text = this.activeText;
        if (text) return text.reconfirmCurrent();
        const done = this.active?.reconfirmCurrent();
        return done !== undefined;
      },
      comment: () => this.activeText?.commentHere() ?? this.active?.commentHere() ?? false,
      showCard: () => this.activeText?.showCaretCard() ?? false,
      jumpToSource: () => {
        const caret = this.active?.caret();
        if (!caret) return false;
        void this.openSource(this.tabs.active, caret.path, caret.line);
        return true;
      },
      stepCommit: (delta) => this.changeset.stepCommit(delta),
      toggleStructure: () => {
        if (!structure.target) return false;
        structure.popup = !structure.popup;
        return true;
      },
      setViewMode: (mode) => void this.setViewMode(mode),
      openFind: () => this.openFind(),
      stepFind: (delta) => void this.stepFind(delta),
      closeFind: () => this.closeFind(),
      toggleFindOption: (option) => this.setFindOptions({ [option]: !this.find.options[option] }),
    };
  }

  /** The tab's pane that shows its file: the text, or the diff. */
  private findPane(id: TabId): "text" | "diff" {
    return this.tabs.modes.get(id) === "file" ? "text" : "diff";
  }

  private finder(id: TabId, pane: "text" | "diff"): Finder | undefined {
    return pane === "text" ? this.texts.get(id) : this.views.get(id);
  }

  /** Runs the find in the pane the tab shows, and ends it in the other. */
  private applyFind(id: TabId) {
    const shown = this.findPane(id);
    this.finder(id, shown === "text" ? "diff" : "text")?.find(undefined);
    const finder = this.finder(id, shown);
    if (!finder || !this.find.open.has(id)) return;
    finder.onFind((summary) => this.found(id, shown, summary));
    const at = this.findAt.get(id);
    finder.find(this.find.options, at && fromPathPos(this.findPaths(id, shown), at));
    if (this.landing === id) void this.land(id, shown, finder);
  }

  /** The files of the sections a find in the tab's pane runs over, in order. */
  private findPaths(id: TabId, pane: "text" | "diff"): readonly string[] {
    const view = pane === "diff" ? this.views.get(id) : undefined;
    if (view) return view.paths();
    const path = pathOf(id);
    return path === undefined ? [] : [path];
  }

  /** Scrolls the tab's current match into view; a view rebuilt meanwhile starts the find again and does it again. */
  private async land(id: TabId, pane: "text" | "diff", finder: Finder) {
    await finder.showFound();
    if (this.finder(id, pane) === finder && this.landing === id) this.landing = undefined;
  }

  /** A pane found something: the bar shows it if it is the active tab's shown pane. */
  private found(id: TabId, pane: "text" | "diff", summary: FindSummary) {
    if (pane !== this.findPane(id) || !this.find.open.has(id)) return;
    const at = summary.at && toPathPos(this.findPaths(id, pane), summary.at);
    if (at) this.findAt.set(id, at);
    if (id === this.tabs.active) this.find.summary = summary;
  }

  /** The tabs, the active one or a view mode changed: the find follows to the pane the active tab shows. */
  syncFind() {
    for (const id of [...this.find.open]) {
      if (this.tabs.open.includes(id)) continue;
      this.find.open.delete(id);
      this.findAt.delete(id);
    }
    const id = this.tabs.active;
    if (this.find.open.has(id)) this.applyFind(id);
    else this.find.summary = NO_SUMMARY;
  }

  /**
   * Opens the find bar in the active tab and puts the cursor in its field; a selection of one line in
   * the focused editor is what it looks for. Answers whether the tab can be searched.
   */
  openFind(): boolean {
    const id = this.tabs.active;
    if (kindOf(id) === "log") return false;
    const selected = focusedEditor(document.activeElement)?.selectedText();
    if (selected && !/[\r\n]/.test(selected)) this.find.options.text = selected;
    this.find.open.add(id);
    this.find.focusRequests++;
    this.applyFind(id);
    return true;
  }

  /** The view the window shows now. */
  private viewOnShow(): View {
    const range = this.changeset.currentLayout()?.interdiff;
    if (range) return { type: "interdiff", from: range.from, to: range.to };
    const index = this.changeset.shownCommit;
    return index === undefined ? { type: "changeset" } : { type: "commit", index };
  }

  /**
   * Shows a result of Find in Files in a view of the place it was found in (see `routeFound`), which
   * is brought on screen if it takes a view the window does not show: in the Changes tab when that
   * tab is on show, else in the file's own tab. The find bar opens there with the search's query and
   * the result as its current match.
   */
  async openFound(result: OpenedResult) {
    const { path, kind, line, col, width } = result;
    const side: Side = kind === "removed" ? "base" : "target";
    const id = await this.showFound(result.scope, result.ownTab, result.commit, { path, side, line, col, width });
    if (id === undefined) return;
    const mode = this.findPane(id) === "text" ? "file" : "diff";
    Object.assign(this.find.options, result.options);
    this.findAt.set(id, { path, order: orderOf(result.hunks, kind, line, mode), from: col });
    this.find.open.add(id);
    this.landing = id;
    this.applyFind(id);
  }

  /**
   * Shows a place a list found in `scope`, in the view the route says (`routeFound`). Answers the tab
   * it landed in, or nothing where the file has no tab to show it in.
   */
  private async showFound(
    scope: SearchScope,
    ownTab: boolean,
    commit: string | undefined,
    at: { path: string; side: Side; line: number; col: number; width: number },
  ): Promise<TabId | undefined> {
    const { path, side, line, col, width } = at;
    const span: TextSpan = { start: { line, col }, end: { line, col: col + width } };
    const route = routeFound(scope, this.viewOnShow(), ownTab);
    if (route.show) {
      if (route.show.type === "interdiff") await this.changeset.setRange({ from: route.show.from, to: route.show.to });
      else await this.changeset.showWhole();
      await tick();
    }
    const section = this.tabs.active === CHANGES && route.inChanges ? this.changes.indexOf(path) : -1;
    if (section >= 0) {
      await this.changes.goToSpan(section, side, span);
      return CHANGES;
    }
    const pinned = route.pinned && commit !== undefined ? commit : undefined;
    await this.openLocation(fileTab(path, pinned), path, line, side, span);
    return pathOf(this.tabs.active) === path ? this.tabs.active : undefined;
  }

  /**
   * Shows a place of the Find tool's list of usages, which are of the changeset: in the Changes tab
   * when that is on show, else in the file's own tab, on the side the place is on.
   */
  async openUsage(
    at: { path: string; side: Side; line: number; col: number; width: number; external: boolean },
    ownTab: boolean,
  ) {
    if (at.external) {
      notices.show(`Outside the repository: ${at.path}`);
      return;
    }
    await this.showFound({ type: "view", view: { type: "changeset" } }, ownTab, undefined, at);
  }

  /** Closes the bar: the current match stays selected in the editor, which gets the focus. */
  closeFind() {
    const id = this.tabs.active;
    this.landing = undefined;
    this.findTyping.cancel();
    this.finder(id, this.findPane(id))?.selectFound();
    this.find.open.delete(id);
    this.finder(id, "diff")?.find(undefined);
    this.finder(id, "text")?.find(undefined);
    this.find.summary = NO_SUMMARY;
  }

  /** Goes to the next or previous match of the active tab, and says when that wrapped round the end. */
  async stepFind(delta: 1 | -1) {
    const id = this.tabs.active;
    this.landing = undefined;
    const stepped = await this.finder(id, this.findPane(id))?.findStep(delta);
    if (stepped?.wrapped)
      this.find.wrapped = { text: delta > 0 ? "Wrapped to the start" : "Wrapped to the end", n: ++this.wraps };
  }

  /** The query is searched this long after the last key, so typing does not search a long changeset for every letter. */
  private findTyping = new Debounced();

  setFindOptions(patch: Partial<FindOptions>) {
    Object.assign(this.find.options, patch);
    this.findTyping.cancel();
    if (Object.keys(patch).length === 1 && "text" in patch) {
      this.findTyping.schedule(FIND_TYPING_MS, () => this.applyFind(this.tabs.active));
      return;
    }
    this.applyFind(this.tabs.active);
  }

  /**
   * Space in a diff view: with an editor focused it flips the hunk at the caret; otherwise the
   * current stop, unless a field or control has the focus and takes the key. Answers whether it acted.
   */
  private toggleBySpace(view: ChangesetView): boolean {
    if (view.toggleAtCaret()) return true;
    if (view.current < 0 || takesSpace(document.activeElement)) return false;
    void view.toggleCurrent();
    return true;
  }

  /** The hunk the focused tab is at: the current one of its view, or of its text. */
  private hunkPlace(): HunkPlace | undefined {
    const path = pathOf(this.tabs.active);
    const text = this.activeText;
    if (text && path !== undefined) return { path, hunk: text.current };
    return this.active?.currentHunk();
  }

  /** The hunk flagged after or before `from` over the files of the view, as they are flagged now. */
  private flaggedFrom(from: HunkPlace | undefined, delta: 1 | -1): HunkPlace | undefined {
    const source = this.sourceOf(this.tabs.active);
    const paths = source.currentLayout()?.paths ?? [];
    return stepFlagged(paths, (path) => source.state(path)?.review.flagged, from, delta);
  }

  /** Goes to the flagged hunk after or before the current one, across files; says so when there is none. */
  private stepFlagged(delta: 1 | -1): boolean {
    if (this.goToFlagged(this.flaggedFrom(this.hunkPlace(), delta))) return true;
    notices.show(delta > 0 ? "No further flagged hunk" : "No earlier flagged hunk");
    return false;
  }

  /** Shows a hunk in the focused tab, or in the tab of its file when that is not the tab's; answers whether there was one. */
  private goToFlagged(to: HunkPlace | undefined): boolean {
    if (!to) return false;
    const id = this.tabs.active;
    void (async () => {
      // New views go to their first hunk first; a later jump must not be undone by it.
      await this.arriving.catch(() => undefined);
      if (id === CHANGES) {
        await this.views.get(CHANGES)?.goToFileHunk(to.path, to.hunk);
      } else {
        // The file's tab at the commit of the tab the step started from, in the mode that tab is in.
        const opened = await this.showFile(id, to.path, this.tabs.modes.get(id));
        if (!opened) return;
        if (opened.mode === "file") this.texts.get(opened.id)?.goToHunk(to.hunk);
        else await this.views.get(opened.id)?.goToFileHunk(to.path, to.hunk);
      }
    })();
    return true;
  }

  /**
   * The file of the focused tab whose text the semantic layer has, which the footer words servers and
   * diagnostics for; none for the Changes tab and for a tab at a commit, whose text is not the changeset's.
   */
  get semanticPath(): string | undefined {
    const id = this.tabs.active;
    return this.analysable(id, "target") ? pathOf(id) : undefined;
  }

  /** The files that have a tab open, each once. */
  get openPaths(): string[] {
    return [...new Set(this.tabs.open.flatMap((id) => pathOf(id) ?? []))];
  }

  /** Shows a place the popup listed, which the tab `from` asked for: in its file's tab, or says where it is when it is outside the repository. */
  private async openRow(from: TabId, row: LocationRow, side: Side) {
    if (row.external) {
      notices.show(`Outside the repository: ${row.path}`);
      return;
    }
    const line = row.line - 1;
    const col = row.col - 1;
    const width = row.matchEnd - row.matchStart;
    await this.openLocation(from, row.path, line, side, { start: { line, col }, end: { line, col: col + width } });
  }

  /** The definition of the symbol at a position of `side`'s text in the tab `from`, shown by `openLocation`. */
  private async goToDefinition(from: TabId, revision: number, side: Side, path: string, line: number, col: number) {
    const found = await semantic.definition(revision, side, path, line, col);
    const location = found?.[0];
    if (!location) return;
    if (location.external) {
      notices.show(`Defined outside the repository: ${location.path}`);
      return;
    }
    await this.openLocation(from, location.path, location.span.start.line, side, location.span);
  }

  /**
   * Shows a 0-based `line` of `side`'s text of the file at `path`, marking `span` if given, in the
   * file's tab of the view `origin` reads. A changed file's tab keeps its mode (a new one opens in the diff mode last chosen);
   * an unchanged file (the same text on both sides) has its text. A base-side location lands in
   * the base content: the base editor of a side-by-side view, or the target line of the unified
   * view where the deleted lines are drawn. Without a span a base line has no place in the diff
   * views yet, so the diff opens at its start. The text has no base side: a tab showing it
   * changes to the diff for such a location.
   */
  async openLocation(origin: Origin, path: string, line: number | undefined, side: Side, span?: TextSpan) {
    // A base path is the file's old path if it was renamed.
    const files = this.viewOf(origin).currentLayout()?.files ?? [];
    const file = files.find((f) => (side === "base" ? (f.oldPath ?? f.path) : f.path) === path);
    const target = file?.path ?? path;
    const opened = await this.showFile(origin, target, (current) =>
      side === "base" && current === "file" ? this.diffMode : undefined,
    );
    if (!opened) return;
    if (opened.mode === "file") {
      const text = this.texts.get(opened.id);
      if (span) text?.goToSpan(span);
      else if (line !== undefined) text?.goToLine(line);
    } else {
      const view = this.views.get(opened.id);
      if (span) await view?.goToSpan(0, side, span);
      else if (side === "target" && line !== undefined) await view?.goToLine(0, line);
    }
  }

  /**
   * Focuses the file's tab, opening it if needed, showing `want` (a mode, or what to want given the tab's
   * mode) if the file has that mode (else the tab's mode, or for a new tab the diff mode last chosen),
   * with the view built. The tab is the one in the view `origin` reads: at the commit of a tab at a
   * commit, else at the commit on screen if the origin reads the view on screen. Answers the tab and
   * its mode, or nothing after saying why it could not be opened.
   */
  private async showFile(
    origin: Origin,
    path: string,
    want?: ViewMode | ((current?: ViewMode) => ViewMode | undefined),
  ): Promise<{ id: TabId; mode: ViewMode } | undefined> {
    let commit: string | undefined;
    try {
      commit = await this.commitAt(origin);
    } catch (error) {
      notices.report(error);
      return undefined;
    }
    const id = fileTab(path, commit);
    const source = this.sourceOf(id);
    const spec = this.specOf(id);
    const scope = spec && this.scope(spec);
    await scope?.whenLoaded();
    if (scope?.gone) {
      notices.show("That commit is not in the changeset any more");
      return undefined;
    }
    const current = this.tabs.modes.get(id);
    const mode = settleMode({
      changed: source.layoutOf(path) !== undefined,
      want: typeof want === "function" ? want(current) : want,
      current,
      last: this.diffMode,
    });
    this.tabs.showFile(path, mode, commit);
    this.ensureScopes();
    await tick(); // a new tab's pane attaches and builds its view
    if (mode === "file") this.ensureText(id);
    else this.syncDiffMode(id);
    return { id, mode };
  }

  /** Draws the tab's diff view in the tab's diff mode. */
  private syncDiffMode(id: TabId) {
    const mode = this.tabs.modes.get(id);
    const view = this.views.get(id);
    if (view && mode !== undefined && mode !== "file" && view.mode !== mode) view.setMode(mode);
  }

  /** Builds the tab's text view if the tab has its root and no view yet. */
  private ensureText(id: TabId) {
    const root = this.textRoots.get(id);
    if (root && !this.texts.has(id)) this.buildText(id, root);
  }

  /**
   * Shows a changed symbol where it is reviewed: its hunk in the Changes tab, made the current one;
   * else the file's tab at the symbol (its text tab when the file is unchanged).
   */
  async revealSymbol({ symbol, change }: ChangedSymbol) {
    const side: Side = change === "removed" ? "base" : "target";
    this.tabs.active = CHANGES;
    if (await this.views.get(CHANGES)?.goToSymbol(symbol.id.path, side, symbol.span)) return;
    await this.openLocation(ON_SCREEN, symbol.id.path, symbol.span.start.line, side, symbol.span);
  }

  /** Jump to Source for a changed symbol: its file's text at the symbol. A removed symbol has no text, so it is shown where it is reviewed. */
  async jumpToSymbol(changed: ChangedSymbol) {
    if (changed.change === "removed") await this.revealSymbol(changed);
    else await this.openSource(ON_SCREEN, changed.symbol.id.path, changed.symbol.span.start.line);
  }

  /** Scrolls the Changes tab to a changed file; answers whether `path` is one. */
  async revealInChanges(path: string): Promise<boolean> {
    // The Changes tab may show only some of the changed files.
    const index = this.changes.indexOf(path);
    if (index < 0) return false;
    this.tabs.active = CHANGES;
    await this.changes.goToFile(index);
    return true;
  }

  private listenersFor(id: TabId, source: ViewSource, built: Shown): Listeners {
    return {
      pinned: id === CHANGES ? (path) => (this.tabs.changesFile = path) : undefined,
      toggleHunk: (shown, path, hunk) =>
        this.editing(
          source,
          () => source.toggleHunk(shown, path, hunk),
          () => undefined,
        ),
      toggleFlag: (shown, path, hunk) =>
        this.editing(
          source,
          () => source.toggleFlag(shown, path, hunk),
          () => undefined,
        ),
      toggleItem: (shown, path, kind) =>
        this.editing(
          source,
          () => source.toggleItem(shown, path, kind),
          () => undefined,
        ),
      toggleFileSeen: (shown, path) =>
        this.editing(
          source,
          () => source.toggleFileSeen(shown, path),
          () => undefined,
        ),
      reconfirmFile: (shown, path) =>
        this.editing(
          source,
          () => source.reconfirmFile(shown, path),
          () => undefined,
        ),
      // Only the changeset has a base diff to show, not a range of versions.
      baseDiff: built.view.type === "changeset" ? (shown, path, hunk) => source.baseDiff(shown, path, hunk) : undefined,
      fileMenu: this.fileMenu,
      openSource: (path, line) => void this.openSource(id, path, line),
      loadFailed: (error) => this.changeset.reportFailure(error, built.revision),
      threads: this.threadListeners(source),
      commentable: commitOf(id) === undefined,
    };
  }

  /** What the cards ask of the owner, for threads of the view `source` shows; they go through that view's own admission. */
  private threadListeners(source: ViewSource = this.changeset): ThreadListeners {
    return {
      create: (shown, target, body) =>
        this.editing(
          source,
          () => threads.create(shown, target, body, (sent) => source.admit(sent)),
          () => false,
        ),
      commentTarget: (shown, path, at) =>
        this.editing(
          source,
          () => source.commentTarget(shown, path, at),
          () => {
            throw new Error(READ_ONLY_NOTE);
          },
        ),
      reply: (thread, body) => threads.reply(thread, body),
      setStatus: (thread, status) => threads.setStatus(thread, status),
      remove: (thread) => threads.remove(thread),
      editMessage: (message, body) => threads.editMessage(message, body),
      removeMessage: (message) => threads.removeMessage(message),
      keepEdit: (message, keep) => threads.keepEdit(message, keep),
      platformName: () => (platform.status ? platformName(platform.status.kind) : "the platform"),
      confirm: (thread) => threads.confirm(thread),
      menu: (thread, x, y) => {
        const subject = threads.items.find((t) => t.id === thread);
        if (subject) void contextMenus.open(threadMenu, x, y, { thread: subject });
      },
      openLink: browse,
      submitKey: commands.keyLabel(SUBMIT_KEY),
      draftScope: () => `${this.backend.project}/${this.key}`,
    };
  }

  /**
   * A thread as the card the editors show, for a place outside them (the Comments tool window), with
   * the same actions; `toggle` opens or folds a resolved one.
   */
  threadCard(thread: Thread, open: boolean, toggle: () => void): HTMLElement {
    const actions: ThreadActions = {
      ...this.threadListeners(),
      create: () => Promise.resolve(false),
      cancelDraft: () => undefined,
      toggle,
    };
    return buildCard({ thread, open }, actions);
  }

  /** The threads were read for the revision and the range `layout` is of. */
  private threadsFit(layout: ViewLayout): boolean {
    return threads.revision === layout.shown.revision && threads.key === rangeKey(layout.interdiff);
  }

  /**
   * Review state was written for `paths`: the views of what is on screen look it up again. The views of
   * another revision or view are about to be replaced and are left alone.
   */
  private refreshReview(paths: ReadonlySet<string> | "all") {
    const shown = this.changeset.currentShown();
    if (!shown) return;
    for (const [id, view] of this.views) {
      if (this.sourceOf(id) === this.changeset && isCurrent(view.shown, shown)) view.refreshReview(paths);
    }
    for (const [id, text] of this.texts) {
      if (this.sourceOf(id) === this.changeset && text.isFor(shown)) text.refreshReview(paths);
    }
  }

  /** What every view shows of the files' review state, read from what is drawn. */
  rendered(): Rendered {
    return {
      views: Object.fromEntries([...this.views].map(([id, view]) => [id, view.rendered()])),
      texts: Object.fromEntries([...this.texts].map(([id, text]) => [id, text.rendered()])),
    };
  }

  /** Gives every view the threads, if they are of the view's revision and range. */
  private showThreads() {
    const revision = threads.revision;
    if (revision === undefined) return;
    const fits = (id: TabId) => {
      const layout = this.sourceOf(id).currentLayout();
      return layout !== undefined && this.threadsFit(layout);
    };
    for (const [id, view] of this.views) if (fits(id)) view.setThreads(this.shownThreads(id), revision);
    for (const [id, text] of this.texts) if (fits(id)) text.setThreads(this.shownThreads(id), revision);
  }

  /**
   * Shows a thread on code: its card in the file's tab, in the mode that tab is in, when the file
   * has a tab open (a tab at a commit shows no threads, so only the file's other tab counts); else in
   * the Changes tab, where it is its card or its file. The changeset's own threads are in no diff.
   */
  async goToThread(thread: Thread): Promise<boolean> {
    if (thread.kind === "changeset") return false;
    const id = thread.path === null ? undefined : fileTab(thread.path);
    if (id !== undefined && this.tabs.open.includes(id)) {
      const file = this.tabs.modes.get(id) === "file";
      this.tabs.active = id;
      if (file) this.ensureText(id);
      if (file ? this.texts.get(id)?.goToThread(thread) : await this.views.get(id)?.goToThread(thread)) return true;
    }
    this.tabs.active = CHANGES;
    return this.views.get(CHANGES)?.goToThread(thread) ?? false;
  }

  /** The diff mode the tab's diff view draws in: the tab's own, or while it shows its text, the one chosen last. */
  private diffModeOf(id: TabId): Mode {
    const mode = this.tabs.modes.get(id);
    return mode === undefined || mode === "file" ? this.diffMode : mode;
  }

  /** Builds a file tab's text view in its root, replacing the one it had. */
  private buildText(id: TabId, root: HTMLElement, force = false) {
    const path = pathOf(id);
    if (path === undefined) return;
    const source = this.sourceOf(id);
    const layout = source.currentLayout();
    const previous = this.texts.get(id);
    // A scope's views are rebuilt when its own manifest comes.
    if (!force && previous && source !== this.changeset && layout && previous.isFor(layout.shown)) return;
    if (!layout) this.keepPlace(id, "text", previous?.lines());
    const place = layout && previous?.isOfView(layout.shown.view) === false ? previous.lines() : undefined;
    previous?.destroy();
    this.texts.delete(id);
    if (!layout) return;
    const shown = layout.shown;
    const file = layout.files.find((f) => f.path === path);
    const text = new TextView(
      root,
      path,
      fileSource(this.backend, shown.revision, shown.view),
      bindRevision(this.hooksFor(id), shown.revision),
      this.softWrap,
      { shown, listeners: this.threadListeners(source), commentable: commitOf(id) === undefined },
      file && { file, review: source, actions: this.textActions(source, file, shown) },
    );
    this.texts.set(id, text);
    if (this.find.open.has(id)) this.applyFind(id);
    const at = this.takePending(id, "text") ?? place;
    if (at) text.showLines(at);
    if (this.threadsFit(layout)) text.setThreads(this.shownThreads(id), shown.revision);
  }

  /**
   * Builds the tab's diff view in its root, replacing `previous`. The scroll position and the
   * current hunk (the nearest one of the same file) carry over.
   */
  private build(id: TabId, previous?: ChangesetView, force = false) {
    const root = this.roots.get(id);
    if (!root || kindOf(id) === "log") return;
    const source = this.sourceOf(id);
    const layout = source.currentLayout();
    // A scope's views are rebuilt when its own manifest comes.
    if (!force && previous && source !== this.changeset && layout && isCurrent(previous.shown, layout.shown)) return;
    const path = pathOf(id);
    const only = this.changeset.baseChanged;
    const files =
      path === undefined
        ? (layout?.files ?? []).filter((f) => !only || only.has(f.path))
        : (layout?.files ?? []).filter((f) => f.path === path);
    const restoring = this.restore?.scroll.has(id) ? this.restore : undefined;
    const anchor = restoring ? undefined : previous?.currentAnchor();
    const top = restoring ? (restoring.scroll.get(id) ?? 0) : root.scrollTop;
    if (restoring) {
      restoring.scroll.delete(id);
      if (restoring.scroll.size === 0) this.restore = undefined;
    }
    // The file left the view, or it is not read yet: its place waits for the view that has it again.
    const absent = !layout || (path !== undefined && !files.length);
    if (absent) this.keepPlace(id, "diff", previous?.lines());
    // A file's tab that is put on another view (a commit, a push, All changes) keeps its lines.
    const place =
      path !== undefined && !restoring && layout && previous && !sameView(previous.shown.view, layout.shown.view)
        ? previous.lines()
        : undefined;
    previous?.destroy();
    this.views.delete(id);
    if (absent) return;
    const shown = layout.shown;
    const view = new ChangesetView(
      root,
      shown,
      files,
      source,
      fileSource(this.backend, shown.revision, shown.view),
      this.diffModeOf(id),
      this.softWrap,
      this.hooksFor(id),
      this.listenersFor(id, source, shown),
      id === CHANGES, // only the Changes tab collapses unchanged regions
    );
    this.views.set(id, view);
    if (this.threadsFit(layout)) view.setThreads(this.shownThreads(id), shown.revision);
    view.scrollTo(top);
    view.keepCurrent(anchor);
    view.sync();
    if (this.find.open.has(id)) this.applyFind(id);
    // A file tab shows its one file at once; the Changes tab loads sections as they come near.
    // A stale revision is replaced by the next build.
    if (path === undefined) return;
    const at = this.takePending(id, "diff") ?? place;
    (at ? view.showLines(at) : view.load(0)).catch(() => undefined);
  }

  /** Remembers where a pane was, for the view that replaces it once its source has read. */
  private keepPlace(id: TabId, pane: keyof FilePositions, at: PanePosition | undefined) {
    if (at) this.pending.set(id, { ...this.pending.get(id), [pane]: at });
  }

  /** The position a pane is put at when it is built, once. */
  private takePending(id: TabId, pane: keyof FilePositions): PanePosition | undefined {
    const positions = this.pending.get(id);
    const at = positions?.[pane];
    if (positions) {
      positions[pane] = undefined;
      if (!positions.diff && !positions.text) this.pending.delete(id);
    }
    return at;
  }

  /** The views of the tabs now, as `memory` keeps them. Where a pane has no view to ask (its tab's diff pane while the next changeset loads), the position stored for it stays. */
  private capture(key: string): SavedView {
    const before = this.memory?.get(key);
    // A range or a commit shows other files than the changeset's own: the positions of what it shows are not kept.
    const plain = !this.shown && this.shownCommit === undefined;
    const panes = new Map<TabId, FilePositions>();
    for (const id of this.tabs.open) {
      if (pathOf(id) === undefined) continue;
      const old = before?.panes.get(id);
      const live = (plain || this.specOf(id) !== undefined) && {
        diff: this.views.get(id)?.lines() ?? this.pending.get(id)?.diff,
        text: this.texts.get(id)?.lines() ?? this.pending.get(id)?.text,
      };
      const kept = live ? { diff: live.diff ?? old?.diff, text: live.text ?? old?.text } : old;
      if (kept && (kept.diff || kept.text)) panes.set(id, kept);
    }
    const changes = this.views.get(CHANGES);
    return {
      open: [...this.tabs.open],
      active: this.tabs.active,
      modes: new Map(this.tabs.modes),
      panes,
      changes: plain ? (changes?.position() ?? before?.changes) : before?.changes,
      anchor: plain ? (changes?.currentAnchor() ?? before?.anchor) : before?.anchor,
    };
  }

  /** Remembers the tabs and where the panes are for the changeset on view. */
  private saveView() {
    const repo = session.repo;
    if (repo === undefined || this.key === undefined) return;
    this.memory ??= new ViewMemory(projectViews);
    this.memory.set(this.key, this.capture(this.key));
  }

  /** The tabs or a pane changed: the views are saved once the changes pause. */
  scheduleSave = () => {
    this.saving.schedule(SAVE_AFTER_MS, () => this.saveView());
  };

  /** A file tab's text pane has its scroll root; returns the function that detaches it. The text is built once the tab shows it. */
  attachText(id: TabId, root: HTMLElement): () => void {
    this.textRoots.set(id, root);
    // The text view's own scroller does not bubble its scrolls.
    root.addEventListener("scroll", this.scheduleSave, { passive: true, capture: true });
    if (this.tabs.modes.get(id) === "file") this.ensureText(id);
    return () => {
      root.removeEventListener("scroll", this.scheduleSave, { capture: true });
      this.texts.get(id)?.destroy();
      this.texts.delete(id);
      this.textRoots.delete(id);
    };
  }

  /** A tab's pane has its scroll root (for a file tab, that of its diff); returns the function that detaches it. */
  attach(id: TabId, root: HTMLElement): () => void {
    this.roots.set(id, root);
    // Registered before the view is, so it hears the scroll that the view's own listener stops.
    root.addEventListener("scroll", this.scheduleSave, { passive: true });
    // The Changes tab is built by `open`, which fetches the manifest first.
    if (kindOf(id) === "log") this.buildLog(id, root);
    // Untracked: the pane is not torn down when the state the view reads changes, `show` rebuilds it.
    else if (id !== CHANGES) untrack(() => this.build(id));
    return () => {
      root.removeEventListener("scroll", this.scheduleSave);
      this.views.get(id)?.destroy();
      this.views.delete(id);
      this.logViews.get(id)?.destroy();
      this.logViews.delete(id);
      this.roots.delete(id);
    };
  }

  private buildLog(id: TabId, root: HTMLElement) {
    const log = this.logs.get(id);
    if (log) this.logViews.set(id, new LogView(root, log.text, log.failure?.line));
  }

  /**
   * Shows the log of a finished check run in a tab of its own (`shown`: the tab's key and title, which
   * differ per attempt), scrolled to the line where the job failed, else at the top. The log is
   * fetched the first time; a log that cannot be had is reported and opens no tab.
   */
  async openLog(run: CheckRun, shown: { key: string; title: string }) {
    const id = logTab(shown.key);
    let log = this.logs.get(id);
    if (!log) {
      try {
        log = await this.backend.checkLog(run.id);
      } catch (error) {
        notices.failed(`Could not load the log of ${run.name}`, error);
        return;
      }
      this.logs.set(id, log);
    }
    const step = log.failure?.step;
    this.tabs.showLog(shown.key, {
      title: shown.title,
      hint: step ? `${run.name}: the step "${step}" failed` : undefined,
    });
    await tick(); // a new tab's pane attaches and builds its view
    this.logViews.get(id)?.goToFailure();
  }

  /**
   * Shows the active tab in the view mode: the Changes tab or a changed file's diff drawn that way,
   * or a file's text. The tab stays the same one; a file's other view keeps its own caret and
   * scroll, and the caret's line carries over to the view that comes up. Does nothing in a tab
   * that does not offer the mode.
   */
  async setViewMode(mode: ViewMode) {
    const id = this.tabs.active;
    const path = pathOf(id);
    if (!modesOf(kindOf(id), this.fileChanged(id)).includes(mode)) return;
    const from = this.tabs.modes.get(id);
    if (mode !== "file") this.diffMode = mode;
    if (from === mode) return;
    if (path === undefined) {
      this.tabs.modes.set(id, mode);
      this.syncDiffMode(id);
      return;
    }
    const line = from === "file" ? this.texts.get(id)?.caretLine() : this.views.get(id)?.caretLine();
    this.tabs.modes.set(id, mode);
    if (mode === "file") {
      this.ensureText(id);
      if (line !== undefined) this.texts.get(id)?.goToLine(line);
    } else {
      this.syncDiffMode(id);
      if (from === "file" && line !== undefined) await this.views.get(id)?.goToLine(0, line);
    }
  }

  /**
   * Jump to Source: the file's tab shows its text, at a 0-based target `line` if given. The tab
   * is opened in that mode if the file has none, in the view `origin` reads.
   */
  async openSource(origin: Origin, path: string, line?: number) {
    const opened = await this.showFile(origin, path, "file");
    if (opened && line !== undefined) this.texts.get(opened.id)?.goToLine(line);
  }

  /**
   * Shows a problem, which sits at a 0-based `line` of the latest code. With `jump` it is Jump to
   * Source: the file's tab in File mode, the editor focused. Otherwise it is shown without taking the
   * focus, in the tab on screen when that shows the file (the Changes tab of the whole changeset, or
   * the file's own tab), else in the file's open tab, else in the Changes tab, else in a tab the file
   * gets. A tab at a commit is never used: it shows the commit's code, not the latest.
   */
  async showProblem(path: string, line: number, jump: boolean) {
    const id = fileTab(path);
    if (jump) {
      await this.openSource(id, path, line);
      await tick(); // the tab's editor is attached before it takes the focus
      this.focusEditor();
      return;
    }
    const wholeChanges = this.changeset.shownCommit === undefined && !this.changeset.currentLayout()?.interdiff;
    const changes = () => this.views.get(CHANGES);
    const changesIndex = wholeChanges ? (changes()?.indexOf(path) ?? -1) : -1;
    const place = problemTab({
      active: this.tabs.active,
      changes: CHANGES,
      file: id,
      open: this.tabs.open,
      inChanges: changesIndex >= 0,
    });
    if (place === "changes") {
      await changes()?.goToLine(changesIndex, line, true);
    } else if (place === "file") {
      this.tabs.active = id;
      await tick(); // the tab's pane attaches and builds its view
      await this.lineInTab(id, line);
    } else if (place === "changesToFront") {
      this.tabs.active = CHANGES;
      await tick();
      await changes()?.goToLine(changesIndex, line, true);
    } else {
      await this.openLocation(id, path, line, "target");
    }
  }

  /**
   * The file whose problems Next and Previous Problem step through: the active tab's file (the file
   * at the top of the Changes tab), none for a tab at a commit, whose text is not the latest code.
   */
  get problemFile(): string | undefined {
    const target = this.structureTarget();
    return target && target.commit === undefined ? target.path : undefined;
  }

  /** Shows the problem after (before) the caret in `problemFile`, wrapping around within the file; the editor keeps its focus. */
  async stepProblem(delta: 1 | -1) {
    const target = this.structureTarget();
    if (!target || target.commit !== undefined) return;
    const next = stepInFile(problems.problemsOf([target.path]), target.caret()?.line, problems.selected, delta);
    if (!next) return;
    problems.selected = next.id;
    await this.showProblem(next.path, next.line, false);
  }

  /** A file's header in a list was chosen: the Changes tab, when it is the tab on screen, shows the file. */
  async revealFileInChanges(path: string) {
    if (this.tabs.active === CHANGES) await this.revealInChanges(path);
  }

  /** Puts the caret on a target line of the file tab `id`, in the mode it is in, without moving the focus. */
  private async lineInTab(id: TabId, line: number) {
    if (this.tabs.modes.get(id) === "file") {
      this.ensureText(id);
      this.texts.get(id)?.goToLine(line);
    } else {
      await this.views.get(id)?.goToLine(0, line, true);
    }
  }

  /** What the file's text tab offers for the file's hunks. */
  private textActions(source: ViewSource, file: FileLayout, shown: Shown): TextActions {
    return {
      toggleSeen: (hunk) =>
        void this.editing(
          source,
          () => source.toggleHunk(shown, file.path, hunk),
          () => undefined,
        ),
      toggleFlag: (hunk) =>
        void this.editing(
          source,
          () => source.toggleFlag(shown, file.path, hunk),
          () => undefined,
        ),
    };
  }

  /** Renders `layout` in every tab, keeping scroll positions and current hunks (a new revision replaces the views). */
  show(force = false) {
    this.ensureScopes();
    for (const id of this.roots.keys()) this.build(id, this.views.get(id), force);
    // A text view is built where the tab shows it or showed it.
    for (const [id, root] of this.textRoots) {
      if (this.texts.has(id) || this.tabs.modes.get(id) === "file") this.buildText(id, root, force);
    }
  }

  /**
   * A refetched manifest may be of a newer revision than the views; rebuild them then. A manifest of
   * another changeset first remembers the tabs and positions of the one that was on view, and
   * brings back those of the new one, if it was on view before.
   */
  update({ layout, facts }: Frame) {
    const key = keyId(facts.target.changeset);
    const switching = this.key !== undefined && key !== this.key;
    if (this.frozen) {
      this.frozen = false;
      if (!switching) this.reloadScopes(); // the scopes made meanwhile have read nothing
    }
    const brought = switching ? this.switchTo(key) : undefined;
    if (!switching) this.tabs.dropStale((id, path) => this.stillChanged(id, path));
    this.key = key;
    const commitChanged = this.commitChanged();
    const rangeChanged = this.rangeChanged(layout) || commitChanged;
    const filterChanged = this.filtered !== this.changeset.baseChanged;
    this.filtered = this.changeset.baseChanged;
    const view = this.views.get(CHANGES);
    if (!view || (!switching && view.revision === layout.shown.revision && !rangeChanged && !filterChanged)) return;
    // Another range or commit is another set of files: the Changes tab starts at the top, and so does
    // the filtered one. A file's tab keeps its lines (`build`).
    if ((rangeChanged || filterChanged) && !switching) {
      this.restore = { scroll: new Map([[CHANGES, 0]]) };
    }
    this.show();
    const restored = switching && this.restoreChanges(brought, layout);
    if (!restored && (switching || rangeChanged || filterChanged)) this.arriving = this.changes.goToHunk(0);
    // The new views have loaded nothing; a newer revision replaces this load.
    this.changes.loadAll().catch(() => undefined);
  }

  /**
   * Whether `layout` shows another range than the last one did: no range and a range differ, and
   * so do ranges that start at another version. A range whose end moved with a new revision is the
   * same range.
   */
  private rangeChanged(layout: ViewLayout): boolean {
    const now = layout.interdiff;
    const was = this.shown;
    this.shown = now && { from: now.from, to: now.to, revision: layout.shown.revision };
    if (!now || !was) return !now !== !was;
    return now.from !== was.from || (now.to !== was.to && layout.shown.revision === was.revision);
  }

  /** Whether the views show another commit than they did, or one where there was none or none where there was one. */
  private commitChanged(): boolean {
    const now = this.changeset.shownCommit;
    const was = this.shownCommit;
    this.shownCommit = now;
    return now !== was;
  }

  /** Remembers the changeset on view and brings back the tabs of `next`; answers what was remembered of it. */
  private switchTo(next: string): SavedView | undefined {
    this.saving.cancel();
    this.saveView();
    return this.bringBack(next);
  }

  /** Sets the tabs the changeset `key` had, and what its panes are put at when they are built. Each tab is judged by the view it reads. */
  private bringBack(key: string): SavedView | undefined {
    const repo = session.repo;
    if (repo !== undefined) this.memory ??= new ViewMemory(projectViews);
    const saved = this.memory?.get(key);
    this.dropScopes();
    this.frozen = false;
    const { open, active, modes } = restoredTabs(
      saved,
      (tab, path) => this.stillChanged(tab, path),
      this.diffMode,
      (tab) => this.tabs.logs.has(tab),
    );
    this.tabs.modes.clear();
    for (const [id, mode] of modes) this.tabs.modes.set(id, mode);
    this.tabs.open = open;
    this.tabs.active = active;
    this.pending.clear();
    for (const id of open) {
      const at = saved?.panes.get(id);
      if (at) this.pending.set(id, { ...at });
    }
    // Every diff view starts at the top; a remembered position is put on it once it is built.
    this.restore = { scroll: new Map(open.filter(hasDiff).map((tab) => [tab, 0])) };
    return saved;
  }

  /** Puts the Changes tab where `saved` left it and makes its hunk the current one; answers whether the position was found. */
  private restoreChanges(saved: SavedView | undefined, layout: ViewLayout): boolean {
    const view = this.views.get(CHANGES);
    if (!view || !saved?.changes || layout.interdiff || this.changeset.shownCommit !== undefined) return false;
    if (!view.restore(saved.changes)) return false;
    view.keepCurrent(saved.anchor);
    return true;
  }

  /** Fetches the manifest and renders it, answering whether it did (not when the project was closed); with `prefetch`, then loads the rest of the Changes tab once its first hunk is painted. */
  async open(prefetch: boolean): Promise<boolean> {
    this.views.get(CHANGES)?.destroy();
    this.views.delete(CHANGES);
    const frame = await this.changeset.refresh();
    if (!frame) return false; // the project was closed meanwhile
    const { layout, facts } = frame;
    this.key = keyId(facts.target.changeset);
    const saved = this.bringBack(this.key);
    this.rangeChanged(layout);
    this.filtered = this.changeset.baseChanged;
    this.commitChanged();
    this.show();
    const view = this.changes;
    if (!this.restoreChanges(saved, layout)) await view.goToHunk(0);
    await nextPaint();
    if (prefetch) await view.loadAll();
    return true;
  }

  /** Builds the views anew with the setting, keeping their scroll positions and current hunks. */
  setSoftWrap(wrap: SoftWrap) {
    if (wrap === this.softWrap) return;
    this.softWrap = wrap;
    this.show(true);
  }

  /** Goes to the hunk `delta` away from the current one in the focused tab. */
  stepHunk(delta: number) {
    const text = this.activeText;
    if (text) {
      text.step(delta);
      return;
    }
    const view = this.active;
    if (!view) return;
    // By commit, the next hunk after a commit's last one is the next commit's first.
    const last = this.tabs.active === CHANGES && delta > 0 && view.current >= view.stopCount - 1;
    if (last && this.changeset.stepCommit(1)) return;
    void view.goToHunk(view.current + delta);
  }
}
