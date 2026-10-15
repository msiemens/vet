// Ties the backend and the views of the open tabs together, and registers the commands that act
// on the active view. Each tab's pane hands its scroll root to `attach`; the controller builds
// the tab's view in it. The work itself lives in the view host, the navigation, the commands, the
// scopes, the find and the persistence; this owns the screen's lifecycle and coordinates them.

import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { CheckRun } from "../../generated/CheckRun";
import type { CommitUnit } from "../../generated/CommitUnit";
import type { Side } from "../../generated/Side";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { TextSpan } from "../../generated/TextSpan";
import type { Thread } from "../../generated/Thread";
import { nextPaint } from "../../lib/nextPaint";
import { preferences } from "../../lib/preferences.svelte";
import type { SoftWrap } from "../../lib/softWrap";
import type { Backend } from "../../state/backend";
import { keyId } from "../../state/changesetKey";
import { commands } from "../../state/commands";
import type { TabId } from "../../state/commands";
import { type Frame, type ViewLayout } from "../../state/reviewModel";
import { reviewShown } from "../../state/reviewShown.svelte";
import { semantic } from "../../state/semantic.svelte";
import { session } from "../../state/session.svelte";
import { isCurrent } from "../../state/shown";
import { threads } from "../../state/threads.svelte";
import { type ViewScope, newScope } from "../../state/viewScope.svelte";
import type { OpenedResult } from "../search";
import { type ChangesetSource } from "./changesetSource";
import { type FilePositions, type SavedView, restoredTabs } from "./changesetViews";
import { type FindOptions } from "./editor/findMatches";
import { FindControl } from "./FindControl";
import { FindState } from "./FindState.svelte";
import { provideActions } from "./reviewActions";
import { CHANGES, type Origin, pathOf } from "./tabModel";
import { Tabs } from "./Tabs.svelte";
import { ViewCommands } from "./ViewCommands";
import { ViewNavigation } from "./ViewNavigation";
import { ViewPersistence } from "./ViewPersistence";
import { ReviewViews } from "./ReviewViews";
import { ViewScopes } from "./ViewScopes";

export type { ChangesetSource } from "./changesetSource";

/** Where the views of the changesets of the repository on view are kept. */
const projectViews = {
  read: () => preferences.read("project", "tabs"),
  write: (value: string | undefined) => preferences.write("project", "tabs", value),
};

/** The draw the review screen's return has to make, kept while the screen was away. */
type Away =
  | {
      kind: "update";
      layout: ViewLayout;
      switching: boolean;
      rangeChanged: boolean;
      filterChanged: boolean;
      brought?: SavedView;
    }
  | { kind: "reload" };

export class ReviewController {
  readonly tabs = new Tabs("unified");
  /** The find bar: the window's query, the tabs it is open in, and what the active tab's view found. */
  readonly find = new FindState();
  /** Which files the views wrap the long lines of. */
  softWrap: SoftWrap = "prose";

  private readonly scopes: ViewScopes;
  private readonly persistence: ViewPersistence;
  private readonly findControl: FindControl;
  private readonly views: ReviewViews;
  private readonly nav: ViewNavigation;
  private readonly commands: ViewCommands;

  /** The changeset on view, by `keyId`; unknown before the first manifest. */
  private key?: string;
  /** The range the views show, if they show one. */
  private shown?: { from: number; to: number; revision: number };
  /** The files the Changes tab was limited to when it was built, if it was. */
  private filtered?: ReadonlySet<string>;
  /** The commit the views show, if they show one. */
  private shownCommit?: number;
  /** The Changes tab of the views built last going to its first hunk. */
  private arriving: Promise<void> = Promise.resolve();
  /** The saved view of the changeset on view, which the Changes tab's first draw goes back to. */
  private saved?: SavedView;
  /** `open` has read the manifest and restored the tabs. */
  private opened = false;
  /** The Changes tab has been drawn for the current changeset. */
  private drawn = false;
  /**
   * The review screen was away when the views would have been drawn: the manifest move is kept, and
   * the draw that only drawing needs waits for the screen (nothing loads for a view nobody sees).
   */
  private away?: Away;
  private unsubscribe: () => void;
  private unsubscribeEvents: () => void;

  constructor(
    readonly backend: Backend,
    private changeset: ChangesetSource,
    private fileMenu: (path: string, x: number, y: number) => void,
  ) {
    semantic.reset(); // the controller belongs to one repository's screen
    this.persistence = new ViewPersistence(projectViews);
    this.scopes = new ViewScopes(
      changeset,
      this.tabs,
      (scope, paths) => this.scopeWritten(scope, paths),
      (spec, marked) => newScope(backend, spec, marked),
    );
    this.findControl = new FindControl(this.find, this.tabs, {
      finder: (id, pane) => (pane === "text" ? this.views.text(id) : this.views.diff(id)),
      paths: (id, pane) => {
        const view = pane === "diff" ? this.views.diff(id) : undefined;
        if (view) return view.paths();
        const path = pathOf(id);
        return path === undefined ? [] : [path];
      },
    });
    this.views = new ReviewViews(backend, changeset, this.tabs, this.scopes, this.findControl, this.persistence, {
      softWrap: () => this.softWrap,
      key: () => this.key,
      hooks: (id) => this.nav.hooks(id),
      openSource: (id, path, line) => void this.nav.openSource(id, path, line),
      fileMenu: this.fileMenu,
      scheduleSave: this.scheduleSave,
    });
    this.nav = new ViewNavigation(backend, changeset, this.tabs, this.scopes, this.views, this.findControl);
    this.commands = new ViewCommands(
      this.tabs,
      this.scopes,
      changeset,
      this.views,
      this.nav,
      this.find,
      this.findControl,
      { arriving: () => this.arriving },
    );

    const offReview = changeset.subscribe((paths) => this.views.refreshReview(paths));
    const offThreads = threads.onChange(() => this.views.showThreads());
    const offCommands = provideActions(this.commands.viewActions());
    const offContext = commands.provide(() => this.commands.facts());
    const offResync = session.onResync(() => this.scopes.reload());
    const offOrigins = semantic.onOrigins((path) => this.views.refreshDiagnostics(path));
    const offShown = reviewShown.onShown(() => void this.screenShown());
    const offHidden = reviewShown.onHidden(() => this.views.stopTrickle());
    // The open tabs are the old changeset's until the new one's manifest has come and brought its tabs back.
    const offOpening = changeset.onOpening((opening) => {
      if (opening.type === "requested") {
        this.scopes.drop();
        this.scopes.setFrozen(true);
      } else if (opening.type === "unannounced") {
        this.scopes.setFrozen(false);
        this.scopes.reload();
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
      offHidden();
      offOpening();
    };
    this.unsubscribeEvents = backend.onEvent((event) => {
      if (event.project !== backend.project) return;
      // A server installed or reconfigured elsewhere may answer differently now.
      if (event.type === "serversChanged") {
        semantic.invalidate();
        this.views.refreshDiagnostics();
      } else if (event.type === "reviewChanged") {
        this.scopes.heard(event.update, event.revision);
      } else if (event.type === "snapshotSettled" || event.type === "targetChanged") {
        this.scopes.reload();
      }
    });
  }

  /** A scope read its manifest or took review state: the views of its tabs follow. */
  private scopeWritten(scope: ViewScope, paths: ReadonlySet<string> | "all") {
    if (scope.gone) {
      // The commit is not among the changeset's any more (a rewritten branch): its tabs have nothing to show.
      for (const id of this.scopes.tabsOf(scope)) this.tabs.close(id);
      return;
    }
    const layout = scope.currentLayout();
    if (!layout) return;
    this.tabs.dropStale((id, path) => this.scopes.stillChanged(id, path));
    for (const id of this.scopes.tabsOf(scope)) {
      const view = this.views.diff(id);
      if (view && isCurrent(view.shown, layout.shown)) view.refreshReview(paths);
      else if (reviewShown.on && this.views.root(id)) this.views.build(id, view);
      const text = this.views.text(id);
      const root = this.views.textRoot(id);
      if (text?.isFor(layout.shown)) text.refreshReview(paths);
      else if (reviewShown.on && root && (text || this.tabs.modes.get(id) === "file")) this.views.buildText(id, root);
    }
  }

  /** Detaches the controller from the window and the store, and tears down the views. */
  destroy() {
    this.nav.cancel();
    this.persistence.cancelScheduled();
    this.findControl.cancel();
    this.saveView();
    this.unsubscribe();
    this.unsubscribeEvents();
    this.scopes.drop();
    this.views.destroy();
  }

  /** The commit a file tab stays at, once it is read. */
  commitUnit(id: TabId): CommitUnit | undefined {
    return this.scopes.commitUnit(id);
  }

  /** Whether the tab's file is changed in the view the tab reads. */
  fileChanged(id: TabId): boolean {
    return this.scopes.fileChanged(id);
  }

  /** Whether the tab has a diff pane: the Changes tab, and a file's tab while its view changes the file or is not read yet, so the pane keeps its place across a switch of views. */
  hasDiffPane(id: TabId): boolean {
    return this.scopes.hasDiffPane(id);
  }

  /** Drops the scopes no open tab reads (a tab closed). */
  pruneScopes() {
    this.scopes.prune();
  }

  /** The file the Structure tool shows and the view it reads it in. */
  structureTarget() {
    return this.nav.structureTarget();
  }

  /** Gives the keyboard focus back to the editor of the active tab, where the Structure window or popup took it from. */
  focusEditor() {
    this.nav.focusEditor();
  }

  /** Shows a symbol of the Structure list where it is declared, in the file's tab (the base side's text for a removed one). */
  async jumpToOutlineSymbol(symbol: SymbolInfo, removed: boolean) {
    await this.nav.jumpToOutlineSymbol(symbol, removed);
  }

  /** The file of the focused tab whose text the semantic layer has, which the footer words servers and diagnostics for. */
  get semanticPath(): string | undefined {
    return this.commands.semanticPath;
  }

  /** The files that have a tab open, each once. */
  get openPaths(): string[] {
    return this.commands.openPaths;
  }

  /** The file whose problems Next and Previous Problem step through. */
  get problemFile(): string | undefined {
    return this.nav.problemFile;
  }

  /** What the active tab shows; none for a log. */
  get activeMode() {
    return this.views.activeMode;
  }

  /** A tab's pane has its scroll root (for a file tab, that of its diff); returns the function that detaches it. */
  attach(id: TabId, root: HTMLElement): () => void {
    return this.views.attach(id, root);
  }

  /** A file tab's text pane has its scroll root; returns the function that detaches it. */
  attachText(id: TabId, root: HTMLElement): () => void {
    return this.views.attachText(id, root);
  }

  /** What every view shows of the files' review state, read from what is drawn. */
  rendered() {
    return this.views.rendered();
  }

  /** A thread as the card the editors show, for a place outside them (the Comments tool window). */
  threadCard(thread: Thread, open: boolean, toggle: () => void): HTMLElement {
    return this.views.threadCard(thread, open, toggle);
  }

  /** Shows a thread on code, in the file's tab or in the Changes tab. */
  async goToThread(thread: Thread): Promise<boolean> {
    return this.views.goToThread(thread);
  }

  /** Shows the log of a finished check run in a tab of its own. */
  async openLog(run: CheckRun, shown: { key: string; title: string }) {
    await this.views.openLog(run, shown);
  }

  /** Shows a 0-based `line` of `side`'s text of the file at `path`, in the file's tab of the view `origin` reads. */
  async openLocation(origin: Origin, path: string, line: number | undefined, side: Side, span?: TextSpan) {
    await this.nav.openLocation(origin, path, line, side, span);
  }

  /** Jump to Source: the file's tab shows its text, at a 0-based target `line` if given. */
  async openSource(origin: Origin, path: string, line?: number) {
    await this.nav.openSource(origin, path, line);
  }

  /** Shows a problem, which sits at a 0-based `line` of the latest code. */
  async showProblem(path: string, line: number, jump: boolean) {
    await this.nav.showProblem(path, line, jump);
  }

  /** Shows the problem after (before) the caret in the active tab's file, wrapping around within the file. */
  async stepProblem(delta: 1 | -1) {
    await this.nav.stepProblem(delta);
  }

  /** Shows a changed symbol where it is reviewed. */
  async revealSymbol(changed: ChangedSymbol) {
    await this.nav.revealSymbol(changed);
  }

  /** Jump to Source for a changed symbol: its file's text at the symbol. */
  async jumpToSymbol(changed: ChangedSymbol) {
    await this.nav.jumpToSymbol(changed);
  }

  /** Scrolls the Changes tab to a changed file; answers whether `path` is one. */
  async revealInChanges(path: string): Promise<boolean> {
    return this.nav.revealInChanges(path);
  }

  /** A file's header in a list was chosen: the Changes tab shows the file when it is the tab on screen. */
  async revealFileInChanges(path: string) {
    await this.nav.revealFileInChanges(path);
  }

  /** Shows a result of Find in Files in a view of the place it was found in. */
  async openFound(result: OpenedResult) {
    await this.nav.openFound(result);
  }

  /** Shows a place of the Find tool's list of usages. */
  async openUsage(
    at: { path: string; side: Side; line: number; col: number; width: number; external: boolean },
    ownTab: boolean,
  ) {
    await this.nav.openUsage(at, ownTab);
  }

  /** The tabs, the active one or a view mode changed: the find follows to the pane the active tab shows. */
  syncFind() {
    this.findControl.sync();
  }

  /** Opens the find bar in the active tab; answers whether the tab can be searched. */
  openFind(): boolean {
    return this.commands.openFind();
  }

  /** Closes the bar: the current match stays selected in the editor, which gets the focus. */
  closeFind() {
    this.findControl.close(this.tabs.active);
  }

  /** Goes to the next or previous match of the active tab, and says when that wrapped round the end. */
  async stepFind(delta: 1 | -1) {
    await this.findControl.step(this.tabs.active, delta);
  }

  setFindOptions(patch: Partial<FindOptions>) {
    this.findControl.setOptions(patch);
  }

  /** The views of the tabs now, as the persistence keeps them. Where a pane has no view to ask (its tab's diff pane while the next changeset loads), the position stored for it stays. */
  private capture(key: string): SavedView {
    const before = this.persistence.stored(key);
    // A range or a commit shows other files than the changeset's own: the positions of what it shows are not kept.
    const plain = !this.shown && this.shownCommit === undefined;
    const panes = new Map<TabId, FilePositions>();
    for (const id of this.tabs.open) {
      if (pathOf(id) === undefined) continue;
      const old = before?.panes.get(id);
      const live = (plain || this.scopes.specOf(id) !== undefined) && {
        diff: this.views.diff(id)?.lines() ?? this.persistence.pendingOf(id, "diff"),
        text: this.views.text(id)?.lines() ?? this.persistence.pendingOf(id, "text"),
      };
      const kept = live ? { diff: live.diff ?? old?.diff, text: live.text ?? old?.text } : old;
      if (kept && (kept.diff || kept.text)) panes.set(id, kept);
    }
    const changes = this.views.diff(CHANGES);
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
    this.persistence.remember(this.key, this.capture(this.key));
  }

  /** The tabs or a pane changed: the views are saved once the changes pause. */
  scheduleSave = () => {
    this.persistence.schedule(() => this.saveView());
  };

  /**
   * A refetched manifest may be of a newer revision than the views; rebuild them then. A manifest of
   * another changeset first remembers the tabs and positions of the one that was on view, and
   * brings back those of the new one, if it was on view before.
   */
  update({ layout, facts }: Frame) {
    const key = keyId(facts.target.changeset);
    const switching = this.key !== undefined && key !== this.key;
    if (this.scopes.isFrozen) {
      this.scopes.setFrozen(false);
      if (!switching) this.scopes.reload(); // the scopes made meanwhile have read nothing
    }
    const brought = switching ? this.switchTo(key) : undefined;
    if (switching) this.saved = brought;
    if (!switching) this.tabs.dropStale((id, path) => this.scopes.stillChanged(id, path));
    this.key = key;
    const commitChanged = this.commitChanged();
    const rangeChanged = this.rangeChanged(layout) || commitChanged;
    const filterChanged = this.filtered !== this.changeset.baseChanged;
    this.filtered = this.changeset.baseChanged;
    const view = this.views.diff(CHANGES);
    if (!view || (!switching && view.revision === layout.shown.revision && !rangeChanged && !filterChanged)) return;
    if (!reviewShown.on) {
      // The review screen is away: the draw waits for it, so nothing loads for a view nobody sees.
      this.away = { kind: "update", layout, switching, rangeChanged, filterChanged, brought };
      return;
    }
    this.drawUpdate(false, layout, switching, rangeChanged, filterChanged, brought);
  }

  /**
   * Rebuilds the views on the manifest move `update` found. `force` also rebuilds the tabs whose own view
   * did not move, which the review screen was away for.
   */
  private drawUpdate(
    force: boolean,
    layout: ViewLayout,
    switching: boolean,
    rangeChanged: boolean,
    filterChanged: boolean,
    brought?: SavedView,
  ): void {
    // Another range or commit is another set of files: the Changes tab starts at the top, and so does
    // the filtered one. A file's tab keeps its lines (`build`).
    if ((rangeChanged || filterChanged) && !switching) {
      this.persistence.beginRestore([CHANGES]);
    }
    this.drawn = true;
    this.views.show(force);
    const restored = switching && this.restoreChanges(brought, layout);
    if (!restored && (switching || rangeChanged || filterChanged)) this.arriving = this.views.changes.goToHunk(0);
    // The new views load what is in view and the margin; the rest trickles while the reviewer is idle.
    this.views.startTrickle();
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
    this.persistence.cancelScheduled();
    this.saveView();
    return this.bringBack(next);
  }

  /** Sets the tabs the changeset `key` had, and what its panes are put at when they are built. Each tab is judged by the view it reads. */
  private bringBack(key: string): SavedView | undefined {
    const saved = session.repo === undefined ? undefined : this.persistence.stored(key);
    this.scopes.drop();
    this.scopes.setFrozen(false);
    const { open, active, modes } = restoredTabs(
      saved,
      (tab, path) => this.scopes.stillChanged(tab, path),
      this.views.diffMode,
      (tab) => this.tabs.logs.has(tab),
    );
    this.tabs.modes.clear();
    for (const [id, mode] of modes) this.tabs.modes.set(id, mode);
    this.tabs.open = open;
    this.tabs.active = active;
    this.persistence.setPending(saved?.panes);
    // Every diff view starts at the top; a remembered position is put on it once it is built.
    this.persistence.beginRestore(open);
    return saved;
  }

  /** Puts the Changes tab where `saved` left it and makes its hunk the current one; answers whether the position was found. */
  private restoreChanges(saved: SavedView | undefined, layout: ViewLayout): boolean {
    const view = this.views.diff(CHANGES);
    if (!view || !saved?.changes || layout.interdiff || this.changeset.shownCommit !== undefined) return false;
    if (!view.restore(saved.changes)) return false;
    view.keepCurrent(saved.anchor);
    return true;
  }

  /** Fetches the manifest and renders it, answering whether it did (not when the project was closed). */
  async open(): Promise<boolean> {
    this.views.destroyDiff(CHANGES);
    const frame = await this.changeset.refresh();
    if (!frame) return false; // the project was closed meanwhile
    const { layout, facts } = frame;
    this.key = keyId(facts.target.changeset);
    this.saved = this.bringBack(this.key);
    this.rangeChanged(layout);
    this.filtered = this.changeset.baseChanged;
    this.commitChanged();
    this.opened = true;
    // The review screen being away leaves the draw for its return, so nothing loads for a view nobody sees.
    if (reviewShown.on) await this.drawOpen(this.saved);
    return true;
  }

  /** Builds the Changes tab and starts loading its files, the draw the review screen's return waits for. */
  private async drawOpen(saved: SavedView | undefined): Promise<void> {
    this.drawn = true;
    this.views.show(true);
    const view = this.views.changes;
    const layout = this.changeset.currentLayout();
    if (layout && !this.restoreChanges(saved, layout)) await view.goToHunk(0);
    await nextPaint();
    this.views.startTrickle();
  }

  /** The review screen came up: the draw the views waited for runs, and the screen's own load starts. */
  private async screenShown(): Promise<void> {
    this.views.refreshDiagnostics();
    const away = this.away;
    this.away = undefined;
    if (!this.drawn && this.opened) await this.drawOpen(this.saved);
    else if (away?.kind === "update")
      this.drawUpdate(true, away.layout, away.switching, away.rangeChanged, away.filterChanged, away.brought);
    else if (away?.kind === "reload") this.reloadDrawn();
  }

  /** Loads what the views show again, after the setting they are drawn with changed while the screen was away. */
  private reloadDrawn(): void {
    this.views.show(true);
    this.views.startTrickle();
  }

  /** Builds the views anew with the setting, keeping their scroll positions and current hunks. */
  setSoftWrap(wrap: SoftWrap) {
    if (wrap === this.softWrap) return;
    this.softWrap = wrap;
    if (reviewShown.on) this.views.show(true);
    else this.away ??= { kind: "reload" };
  }
}
