// What the review's commands do and read: the facts a command asks about the active tab, the actions
// the registry runs, and the stepping through hunks and flagged hunks they drive.

import type { CommandContext } from "../../state/commands";
import { notices } from "../../state/notices.svelte";
import { servedBy } from "../../state/servedBy.svelte";
import { serverList } from "../../state/serverList.svelte";
import { withoutFeature } from "../../state/serverStatus";
import { structure } from "../../state/structure.svelte";
import type { ChangesetSource } from "./changesetSource";
import type { ChangesetView } from "./editor/ChangesetView";
import { focusedEditor } from "./editor/semantic";
import type { FindControl } from "./FindControl";
import type { FindState } from "./FindState.svelte";
import { type HunkPlace, stepFlagged } from "./flaggedHunks";
import type { ViewActions } from "./reviewCommands";
import { CHANGES, commitOf, kindOf, modesOf, pathOf } from "./tabModel";
import type { Tabs } from "./Tabs.svelte";
import type { ViewNavigation } from "./ViewNavigation";
import type { ReviewViews } from "./ReviewViews";
import type { ViewScopes } from "./ViewScopes";

/** Space types in a field and presses a focused control; it marks the current item only when none of those has the focus. */
const takesSpace = (el: Element | null) =>
  !!el?.closest("button, a, input, textarea, select, [contenteditable], [tabindex], [role=menuitem], [role=tab]");

/** What the commands ask of the controller that owns them. */
export interface CommandHost {
  /** The Changes tab of the views built last going to its first hunk, which a later jump must not undo. */
  arriving(): Promise<void>;
}

export class ViewCommands {
  private tabs: Tabs;
  private scopes: ViewScopes;
  private changeset: ChangesetSource;
  private views: ReviewViews;
  private nav: ViewNavigation;
  private find: FindState;
  private findControl: FindControl;
  private host: CommandHost;

  constructor(
    tabs: Tabs,
    scopes: ViewScopes,
    changeset: ChangesetSource,
    views: ReviewViews,
    nav: ViewNavigation,
    find: FindState,
    findControl: FindControl,
    host: CommandHost,
  ) {
    this.tabs = tabs;
    this.scopes = scopes;
    this.changeset = changeset;
    this.views = views;
    this.nav = nav;
    this.find = find;
    this.findControl = findControl;
    this.host = host;
  }

  /** The file of the focused tab whose text the semantic layer has, which the footer words servers and diagnostics for; none for the Changes tab and for a tab at a commit, whose text is not the changeset's. */
  get semanticPath(): string | undefined {
    const id = this.tabs.active;
    return this.scopes.analysable(id, "target") ? pathOf(id) : undefined;
  }

  /** The files that have a tab open, each once. */
  get openPaths(): string[] {
    return [...new Set(this.tabs.open.flatMap((id) => pathOf(id) ?? []))];
  }

  /** The facts about the review screen that commands read. */
  facts(): Partial<CommandContext> {
    const id = this.tabs.active;
    const path = pathOf(id);
    const view = this.views.activeDiff();
    const text = this.views.activeText();
    const editor = focusedEditor(document.activeElement);
    const file = editor?.path ?? path;
    return {
      tab: id,
      tabCount: this.tabs.open.length,
      viewMode: this.views.activeMode,
      viewModes: modesOf(kindOf(id), this.scopes.fileChanged(id)),
      diffView: view !== undefined || text !== undefined,
      noComments: commitOf(id) !== undefined,
      byCommit: this.changeset.shownCommit !== undefined,
      hunk: view?.current ?? text?.current ?? -1,
      findOpen: this.find.open.has(id),
      editor,
      noImplementations:
        editor && withoutFeature(servedBy.language(editor.path), serverList.servers, "implementations"),
      noUsages: this.scopes.showsCommit(id),
      serverLanguage: file === undefined ? undefined : servedBy.language(file),
    };
  }

  viewActions(): ViewActions {
    return {
      step: (delta) => this.stepHunk(delta),
      toggleSeen: () => {
        const text = this.views.activeText();
        if (text) return text.toggleCurrent();
        const view = this.views.activeDiff();
        return !!view && this.toggleBySpace(view);
      },
      toggleFlag: () => {
        const text = this.views.activeText();
        if (text) return text.toggleFlagCurrent();
        return this.views.activeDiff()?.toggleFlagCurrent() !== undefined;
      },
      stepFlag: (delta) => this.stepFlagged(delta > 0 ? 1 : -1),
      firstFlagged: () => this.goToFlagged(this.flaggedFrom(undefined, 1)),
      reconfirm: () => {
        const text = this.views.activeText();
        if (text) return text.reconfirmCurrent();
        const done = this.views.activeDiff()?.reconfirmCurrent();
        return done !== undefined;
      },
      comment: () => this.views.activeText()?.commentHere() ?? this.views.activeDiff()?.commentHere() ?? false,
      showCard: () => this.views.activeText()?.showCaretCard() ?? false,
      jumpToSource: () => {
        const caret = this.views.activeDiff()?.caret();
        if (!caret) return false;
        void this.nav.openSource(this.tabs.active, caret.path, caret.line);
        return true;
      },
      stepCommit: (delta) => this.changeset.stepCommit(delta),
      toggleStructure: () => {
        if (!structure.target) return false;
        structure.popup = !structure.popup;
        return true;
      },
      setViewMode: (mode) => void this.views.setViewMode(mode),
      openFind: () => this.openFind(),
      stepFind: (delta) => void this.findControl.step(this.tabs.active, delta),
      closeFind: () => this.findControl.close(this.tabs.active),
      toggleFindOption: (option) => this.findControl.setOptions({ [option]: !this.find.options[option] }),
    };
  }

  /**
   * Opens the find bar in the active tab and puts the cursor in its field; a selection of one line in
   * the focused editor is what it looks for. Answers whether the tab can be searched.
   */
  openFind(): boolean {
    const id = this.tabs.active;
    if (kindOf(id) === "log") return false;
    this.findControl.open(id, focusedEditor(document.activeElement)?.selectedText());
    return true;
  }

  /** Goes to the hunk `delta` away from the current one in the focused tab. */
  stepHunk(delta: number) {
    const text = this.views.activeText();
    if (text) {
      text.step(delta);
      return;
    }
    const view = this.views.activeDiff();
    if (!view) return;
    // By commit, the next hunk after a commit's last one is the next commit's first.
    const last = this.tabs.active === CHANGES && delta > 0 && view.current >= view.stopCount - 1;
    if (last && this.changeset.stepCommit(1)) return;
    void view.goToHunk(view.current + delta);
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
    const text = this.views.activeText();
    if (text && path !== undefined) return { path, hunk: text.current };
    return this.views.activeDiff()?.currentHunk();
  }

  /** The hunk flagged after or before `from` over the files of the view, as they are flagged now. */
  private flaggedFrom(from: HunkPlace | undefined, delta: 1 | -1): HunkPlace | undefined {
    const source = this.scopes.sourceOf(this.tabs.active);
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
      await this.host.arriving().catch(() => undefined);
      if (id === CHANGES) {
        await this.views.diff(CHANGES)?.goToFileHunk(to.path, to.hunk);
      } else {
        // The file's tab at the commit of the tab the step started from, in the mode that tab is in.
        const opened = await this.nav.showFile(id, to.path, this.tabs.modes.get(id));
        if (!opened) return;
        if (opened.mode === "file") this.views.text(opened.id)?.goToHunk(to.hunk);
        else await this.views.diff(opened.id)?.goToFileHunk(to.path, to.hunk);
      }
    })();
    return true;
  }
}
