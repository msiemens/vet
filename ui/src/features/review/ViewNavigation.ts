// Opening a place in a tab: a line, a span or a whole file, in the view the tab that asked reads. This
// also carries the semantic hooks a view asks its owner for (definition, usages, hover, diagnostics),
// since those open places too.

import { tick } from "svelte";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { LocationRow } from "../../generated/LocationRow";
import type { SearchScope } from "../../generated/SearchScope";
import type { Side } from "../../generated/Side";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { TextSpan } from "../../generated/TextSpan";
import type { View } from "../../generated/View";
import { isAbort } from "../../lib/abort";
import { STALE_NOTICE } from "../../lib/notice";
import { RequestError } from "../../lib/requestError";
import type { ViewMode } from "../../lib/viewMode";
import type { Backend } from "../../state/backend";
import type { TabId } from "../../state/commands";
import { findSets } from "../../state/findSets.svelte";
import { locationPicker } from "../../state/locationPicker.svelte";
import { LocationSearch } from "../../state/locationSearch";
import { notices } from "../../state/notices.svelte";
import { stepInFile } from "../../state/problemRows";
import { problems } from "../../state/problems.svelte";
import { reviewShown } from "../../state/reviewShown.svelte";
import { semantic } from "../../state/semantic.svelte";
import { serverList } from "../../state/serverList.svelte";
import type { StructureTarget } from "../../state/structure.svelte";
import { symbols } from "../../state/symbols.svelte";
import type { ViewSource } from "../../state/viewScope.svelte";
import type { OpenedResult } from "../search";
import type { ViewHooks } from "./editor/ChangesetView";
import { orderOf } from "./editor/findMatches";
import type { FindControl } from "./FindControl";
import { routeFound } from "./foundRoute";
import { problemTab } from "./problemTab";
import type { ReviewViews } from "./ReviewViews";
import type { ChangesetSource } from "./changesetSource";
import { CHANGES, ON_SCREEN, type Origin, commitOf, fileTab, pathOf, settleMode } from "./tabModel";
import type { Tabs } from "./Tabs.svelte";
import type { ViewScopes } from "./ViewScopes";

/** The view a request that starts from `origin` reads: the tab's own (see `ViewScopes`), or for `ON_SCREEN` the one on screen. */
export class ViewNavigation {
  private backend: Backend;
  /** The changeset on screen as the navigation reads it: its source, plus the commit it shows, where failures are reported and how the view is switched. */
  private changeset: ViewSource & Pick<ChangesetSource, "shownCommit" | "reportFailure" | "setRange" | "showWhole">;
  private tabs: Tabs;
  private scopes: ViewScopes;
  private views: ReviewViews;
  private find: FindControl;
  /** Show Usages and Go to Implementation: goes to the one place found, or lists the places in the popup. */
  private search: LocationSearch;

  constructor(
    backend: Backend,
    changeset: ViewSource & Pick<ChangesetSource, "shownCommit" | "reportFailure" | "setRange" | "showWhole">,
    tabs: Tabs,
    scopes: ViewScopes,
    views: ReviewViews,
    find: FindControl,
  ) {
    this.backend = backend;
    this.changeset = changeset;
    this.tabs = tabs;
    this.scopes = scopes;
    this.views = views;
    this.find = find;
    this.search = new LocationSearch({
      picker: locationPicker,
      notice: (text) => notices.show(text),
      isAbort,
      fail: (error, revision) => this.changeset.reportFailure(error, revision),
      restartServer: (language) => serverList.restart(language),
    });
  }

  /** Stops the location search of a torn-down screen. */
  cancel() {
    this.search.cancel();
  }

  /**
   * The semantic layer of the tab `id`, which reads the changeset's snapshots: not in a commit, and
   * in a range only the new side of the latest version is one.
   */
  hooks(id: TabId): ViewHooks {
    return {
      hover: (revision, side, path, line, col, signal) =>
        this.scopes.analysable(id, side)
          ? semantic.hover(revision, side, path, line, col, signal)
          : Promise.resolve({ text: "", code: [] }),
      definition: (revision, side, path, line, col) => {
        if (this.scopes.analysable(id, side)) void this.goToDefinition(id, revision, side, path, line, col);
      },
      hasDefinition: (revision, side, path, line, col, signal) =>
        this.scopes.analysable(id, side)
          ? semantic.hasDefinition(revision, side, path, line, col, signal)
          : Promise.resolve(false),
      locations: (revision, kind, side, path, at, subject) => {
        if (!this.scopes.analysable(id, side)) return;
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
        if (this.scopes.analysable(id, side))
          findSets.findUsages({ type: "usages", symbol: subject.name, side, path, ...at });
      },
      // Asked again for the mounted editors when the review screen comes up.
      diagnostics: (revision, path) =>
        reviewShown.on && this.scopes.analysable(id, "target")
          ? semantic.diagnostics(revision, path)
          : Promise.resolve([]),
      cursor: (revision, side, path, line, col) => {
        if (this.scopes.analysable(id, side)) symbols.follow(revision, side, path, line, col);
      },
    };
  }

  /**
   * The file the Structure tool shows and the view it reads it in: the active file tab's, the file
   * at the top of the Changes tab, none for a log. Reading it subscribes to the view's layout.
   */
  structureTarget(): StructureTarget | undefined {
    const id = this.tabs.active;
    const path = pathOf(id) ?? (id === CHANGES ? this.tabs.changesFile : undefined);
    const shown = path === undefined ? undefined : this.scopes.sourceOf(id).currentShown();
    if (path === undefined || !shown) return undefined;
    // The Symbol panel works on the changeset's snapshots, which no commit on screen has.
    const panel =
      shown.view.type === "changeset" &&
      this.changeset.shownCommit === undefined &&
      this.scopes.analysable(id, "target");
    const caret = () =>
      this.tabs.modes.get(id) === "file" ? this.views.text(id)?.caret() : this.views.diff(id)?.caretIn(path);
    return { tab: id, path, commit: commitOf(id), shown, panel, caret };
  }

  /** Gives the keyboard focus back to the editor of the active tab, where the Structure window or popup took it from. */
  focusEditor() {
    const id = this.tabs.active;
    const path = pathOf(id) ?? (id === CHANGES ? this.tabs.changesFile : undefined);
    if (this.tabs.modes.get(id) === "file") this.views.text(id)?.focusEditor();
    else if (path !== undefined) this.views.diff(id)?.focusEditor(path);
  }

  /** The file whose problems Next and Previous Problem step through: the active tab's file (the file at the top of the Changes tab), none for a tab at a commit, whose text is not the latest code. */
  get problemFile(): string | undefined {
    const target = this.structureTarget();
    return target && target.commit === undefined ? target.path : undefined;
  }

  /** Shows a symbol of the Structure list where it is declared, in the file's tab (the base side's text for a removed one). */
  async jumpToOutlineSymbol(symbol: SymbolInfo, removed: boolean) {
    await this.openLocation(this.tabs.active, symbol.id.path, symbol.at.line, removed ? "base" : "target");
  }

  /** The view a request that starts from `origin` reads: the tab's own (see `sourceOf`), or for `ON_SCREEN` the one on screen. */
  private viewOf(origin: Origin): ViewSource {
    return origin === ON_SCREEN ? this.changeset : this.scopes.sourceOf(origin);
  }

  /** The full id of the commit the view `origin` reads is of; none if it is of the changeset. */
  private async commitAt(origin: Origin): Promise<string | undefined> {
    if (this.viewOf(origin) === this.changeset) return this.commitOnScreen();
    return origin === ON_SCREEN ? undefined : commitOf(origin);
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
      side === "base" && current === "file" ? this.views.diffMode : undefined,
    );
    if (!opened) return;
    if (opened.mode === "file") {
      const text = this.views.text(opened.id);
      if (span) text?.goToSpan(span);
      else if (line !== undefined) text?.goToLine(line);
    } else {
      const view = this.views.diff(opened.id);
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
  async showFile(
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
    const source = this.scopes.sourceOf(id);
    const spec = this.scopes.specOf(id);
    const scope = spec && this.scopes.scope(spec);
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
      last: this.views.diffMode,
    });
    this.tabs.showFile(path, mode, commit);
    this.scopes.ensure();
    await tick(); // a new tab's pane attaches and builds its view
    if (mode === "file") this.views.ensureText(id);
    else this.views.syncDiffMode(id);
    return { id, mode };
  }

  /**
   * Shows a changed symbol where it is reviewed: its hunk in the Changes tab, made the current one;
   * else the file's tab at the symbol (its text tab when the file is unchanged).
   */
  async revealSymbol({ symbol, change }: ChangedSymbol) {
    const side: Side = change === "removed" ? "base" : "target";
    this.tabs.active = CHANGES;
    if (await this.views.diff(CHANGES)?.goToSymbol(symbol.id.path, side, symbol.span)) return;
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
    const index = this.views.changes.indexOf(path);
    if (index < 0) return false;
    this.tabs.active = CHANGES;
    await this.views.changes.goToFile(index);
    return true;
  }

  /** Jump to Source: the file's tab shows its text, at a 0-based target `line` if given. The tab is opened in that mode if the file has none, in the view `origin` reads. */
  async openSource(origin: Origin, path: string, line?: number) {
    const opened = await this.showFile(origin, path, "file");
    if (opened && line !== undefined) this.views.text(opened.id)?.goToLine(line);
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
    const changes = () => this.views.diff(CHANGES);
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
      this.views.ensureText(id);
      this.views.text(id)?.goToLine(line);
    } else {
      await this.views.diff(id)?.goToLine(0, line, true);
    }
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
    const mode = this.find.pane(id) === "text" ? "file" : "diff";
    this.find.landOn(id, result.options, { path, order: orderOf(result.hunks, kind, line, mode), from: col });
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
    const section = this.tabs.active === CHANGES && route.inChanges ? this.views.changes.indexOf(path) : -1;
    if (section >= 0) {
      await this.views.changes.goToSpan(section, side, span);
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
}
