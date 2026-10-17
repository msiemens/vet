// Ties the backend and the views of the open tabs together, and registers the commands that act
// on the active view. Each tab's pane hands its scroll root to `attach`; the controller builds
// the tab's view in it. The work itself lives in the view host, the navigation, the commands, the
// scopes, the find and the persistence; this owns the screen's lifecycle and coordinates them.
// A controller is made for one changeset's session and destroyed with the screen it belongs to.

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
import type { ChangesetSession } from "../../state/changesetSession.svelte";
import { commands } from "../../state/commands";
import type { TabId } from "../../state/commands";
import { type Frame, type ViewLayout } from "../../state/reviewModel";
import type { ViewChange } from "../../state/viewChange";
import { semantic } from "../../state/semantic.svelte";
import { session } from "../../state/session.svelte";
import { isCurrent } from "../../state/shown";
import { type ViewSource, newScope } from "../../state/viewScope.svelte";
import type { OpenedResult } from "../search";
import { type FilePositions, type SavedView, restoredTabs } from "./changesetViews";
import { type FindOptions } from "./editor/findMatches";
import { FindControl } from "./FindControl";
import { reviewFind } from "./FindState.svelte";
import { provideActions } from "./reviewActions";
import { reviewMemory } from "./reviewMemory";
import { CHANGES, type Origin, pathOf } from "./tabModel";
import { Tabs } from "./Tabs.svelte";
import { ViewCommands } from "./ViewCommands";
import { ViewNavigation } from "./ViewNavigation";
import { ViewPersistence } from "./ViewPersistence";
import { ReviewViews } from "./ReviewViews";
import { ViewScopes } from "./ViewScopes";

export type { ChangesetSource } from "./changesetSource";

/** What the screens of the sessions of the repository on view remember of its changesets. */
const memoryOf = reviewMemory({
  read: () => preferences.read("project", "tabs"),
  write: (value: string | undefined) => preferences.write("project", "tabs", value),
});

export class ReviewController {
  readonly tabs = new Tabs("unified");
  /** The find bar: the window's query, the tabs it is open in, and what the active tab's view found. */
  readonly find = reviewFind;
  /** Which files the views wrap the long lines of. */
  softWrap: SoftWrap = "prose";

  private readonly scopes: ViewScopes;
  private readonly persistence: ViewPersistence;
  private readonly findControl: FindControl;
  private readonly views: ReviewViews;
  private readonly nav: ViewNavigation;
  private readonly commands: ViewCommands;

  /** The changeset the controller is of, by `keyId`. */
  private readonly key: string;
  /** The tabs are those of the changeset, brought back, and can be remembered. */
  private brought = false;
  private destroyed = false;
  /** The views built last show the changeset's own files, not those of a range or a commit. */
  private plain = true;
  /** The Changes tab of the views built last going to its first hunk. */
  private arriving: Promise<void> = Promise.resolve();
  private unsubscribe: () => void;
  private unsubscribeEvents: () => void;

  constructor(
    readonly backend: Backend,
    private changeset: ChangesetSession,
    private fileMenu: (path: string, x: number, y: number) => void,
  ) {
    semantic.reset(); // the controller belongs to one repository's screen
    this.key = keyId(changeset.key);
    this.persistence = new ViewPersistence(memoryOf(session.repo));
    this.scopes = new ViewScopes(
      changeset,
      changeset.whole,
      this.tabs,
      (scope, paths) => this.scopeWritten(scope, paths),
      (spec, marked) => newScope(backend, spec, marked, changeset.commitList, () => changeset.currentShown()?.revision),
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
    this.views = new ReviewViews(
      backend,
      changeset,
      changeset.threads,
      this.tabs,
      this.scopes,
      this.findControl,
      this.persistence,
      {
        softWrap: () => this.softWrap,
        hooks: (id) => this.nav.hooks(id),
        openSource: (id, path, line) => void this.nav.openSource(id, path, line),
        fileMenu: this.fileMenu,
        scheduleSave: this.scheduleSave,
      },
    );
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
    const offThreads = changeset.threads.onChange(() => this.views.showThreads());
    const offCommands = provideActions(this.commands.viewActions());
    const offContext = commands.provide(() => this.commands.facts());
    const offResync = session.onResync(() => this.scopes.reload());
    const offOrigins = semantic.onOrigins((path) => this.views.refreshDiagnostics(path));
    const offViewChange = changeset.onViewChange((frame, change) => this.viewChanged(frame, change));
    const offScopes = changeset.onScopesReload(() => this.scopes.reload());
    this.unsubscribe = () => {
      offScopes();
      offViewChange();
      offReview();
      offThreads();
      offCommands();
      offContext();
      offResync();
      offOrigins();
    };
    // The session clears itself as it ends; the tabs are remembered before that.
    changeset.lifetime.add(() => {
      if (!this.destroyed) this.saveView();
    });
    this.unsubscribeEvents = backend.onEvent((event) => {
      // A server installed or reconfigured elsewhere may answer differently now.
      if (event.type === "serversChanged") {
        semantic.invalidate();
        this.views.refreshDiagnostics();
      } else if (event.type === "reviewChanged") {
        this.scopes.heard(event.update, event.revision);
      }
    });
  }

  /** A scope read its manifest or took review state: the views of its tabs follow. */
  private scopeWritten(scope: ViewSource, paths: ReadonlySet<string> | "all") {
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
      else if (this.views.root(id)) this.views.build(id, view);
      const text = this.views.text(id);
      const root = this.views.textRoot(id);
      if (text?.isFor(layout.shown)) text.refreshReview(paths);
      else if (root && (text || this.tabs.modes.get(id) === "file")) this.views.buildText(id, root);
    }
  }

  /** Detaches the controller from the window and the store, and tears down the views. */
  destroy() {
    this.nav.cancel();
    this.persistence.cancelScheduled();
    this.findControl.cancel();
    if (this.changeset.lifetime.alive) this.saveView();
    this.destroyed = true;
    this.unsubscribe();
    this.unsubscribeEvents();
    this.scopes.drop();
    this.scopes.dispose();
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

  /** The views of the tabs now, as the persistence keeps them. Where a pane has no view to ask (it is not built yet), the position stored for it stays. */
  private capture(key: string): SavedView {
    const before = this.persistence.stored(key);
    // A range or a commit shows other files than the changeset's own: the positions of what it shows are not kept.
    const plain = this.plain;
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

  /** Remembers the tabs and where the panes are for the changeset. */
  private saveView() {
    if (session.repo === undefined || !this.brought) return;
    this.persistence.remember(this.key, this.capture(this.key));
  }

  /** The tabs or a pane changed: the views are saved once the changes pause. */
  scheduleSave = () => {
    this.persistence.schedule(() => this.saveView());
  };

  /**
   * The session says the views changed: a newer revision, or another range, commit or filter, which
   * start the Changes tab at the top. Before `open` has brought the tabs back, it draws what the
   * session holds.
   */
  private viewChanged({ layout }: Frame, change: ViewChange) {
    if (!this.brought) return;
    this.tabs.dropStale((id, path) => this.scopes.stillChanged(id, path));
    this.plain = !layout.interdiff && this.changeset.shownCommit === undefined;
    if (!this.views.diff(CHANGES)) return;
    this.drawUpdate(change === "start");
  }

  /** Rebuilds the views; where `top`, the Changes tab starts at the top. */
  private drawUpdate(top: boolean): void {
    // Another range, commit or filter is another set of files. A file's tab keeps its lines (`build`).
    if (top) this.persistence.beginRestore([CHANGES]);
    this.views.show();
    if (top) this.arriving = this.views.changes.goToHunk(0);
    // The new views load what is in view and the margin; the rest trickles while the reviewer is idle.
    this.views.startTrickle();
  }

  /** Sets the tabs the changeset had when it was last on view, and what its panes are put at when they are built. Each tab is judged by the view it reads. */
  private bringBack(): SavedView | undefined {
    const saved = session.repo === undefined ? undefined : this.persistence.stored(this.key);
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
    this.brought = true;
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

  /** Takes the manifest the session holds and renders it, answering whether it did (not when the session ended meanwhile). */
  async open(): Promise<boolean> {
    this.views.destroyDiff(CHANGES);
    const frame = await this.changeset.frame();
    if (!frame || !this.changeset.lifetime.alive) return false;
    const saved = this.bringBack();
    this.plain = !frame.layout.interdiff && this.changeset.shownCommit === undefined;
    await this.drawOpen(saved);
    return true;
  }

  /** Builds the Changes tab and starts loading its files. The panes that attached already built themselves. */
  private async drawOpen(saved: SavedView | undefined): Promise<void> {
    this.views.show();
    const view = this.views.changes;
    const layout = this.changeset.currentLayout();
    if (layout && !this.restoreChanges(saved, layout)) await view.goToHunk(0);
    await nextPaint();
    this.views.startTrickle();
  }

  /** Builds the views anew with the setting, keeping their scroll positions and current hunks. */
  setSoftWrap(wrap: SoftWrap) {
    if (wrap === this.softWrap) return;
    this.softWrap = wrap;
    this.views.show(true);
  }
}
