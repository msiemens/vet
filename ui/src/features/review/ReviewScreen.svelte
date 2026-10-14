<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, untrack } from "svelte";
  import type { MenuSpec } from "../../lib/commandMenu";
  import { repoName } from "../../lib/paths";
  import Banner from "../../lib/ui/Banner.svelte";
  import SideBar from "../../lib/ui/SideBar.svelte";
  import type { SoftWrap } from "../../lib/softWrap";
  import type { Backend } from "../../state/backend";
  import type { Changeset } from "../../state/changeset.svelte";
  import { keyId } from "../../state/changesetKey";
  import type { CommandContext } from "../../state/commands";
  import { commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { minute } from "../../state/minute.svelte";
  import { platform } from "../../state/platform.svelte";
  import { session } from "../../state/session.svelte";
  import { structure } from "../../state/structure.svelte";
  import { symbols } from "../../state/symbols.svelte";
  import { threads } from "../../state/threads.svelte";
  import { versions } from "../../state/versions.svelte";
  import { CommentsPanel } from "../comments";
  import { copyText } from "../platform";
  import { StructurePopup } from "../structure";
  import FileTree from "./FileTree.svelte";
  import "./editor/section.css";
  import "./editor/hunkBar.css";
  import "./editor/fileChanges.css";
  import "./editor/scrollMarks.css";
  import "./editor/find.css";
  import "./editor/changeCard.css";
  import "./editor/highlights.css";
  import "./editor/hover.css";
  import "./editor/threadCard.css";
  import { focusedEditor } from "./editor/semantic";
  import { ReviewController } from "./ReviewController";
  import { provideActions } from "./reviewActions";
  import { editorMenu, fileHeaderMenu, tabMenu, treeMenu } from "./reviewCommands";
  import CommitHeader from "./CommitHeader.svelte";
  import Dock from "./Dock.svelte";
  import EmptyChanges from "./EmptyChanges.svelte";
  import { type Action, type EmptyView, emptyView, pickRange } from "./emptyChanges";
  import { narrowingLabel } from "./narrowing";
  import { exposeRendered } from "./devHook";
  import type { SymbolRow } from "./symbolRows";
  import CommitStrip from "./CommitStrip.svelte";
  import FindBar from "./FindBar.svelte";
  import Toast from "../../lib/ui/Toast.svelte";
  import { CHANGES, ON_SCREEN, commitOf, kindOf, pathOf } from "./tabModel";
  import TabStrip from "./TabStrip.svelte";
  import ViewControls from "./ViewControls.svelte";

  let {
    backend,
    changeset,
    leftTool,
    leftOpen,
    rightTool,
    rightOpen,
    side,
    bottomOpen = false,
    bottom,
    softWrap,
    onfilter,
    ontool,
    scope,
    onready,
    onclose,
  }: {
    backend: Backend;
    /** What the backend sent about the open changeset. */
    changeset: Changeset;
    /** Which files wrap their long lines; the owner applies a change to the controller. */
    softWrap: SoftWrap;
    /** Opens Settings at the review filter. */
    onfilter: () => void;
    /** Shows a tool window. */
    ontool: (tool: "changes" | "all" | "comments" | "timeline") => void;
    /** What the changes shown are of, which names the first tab. */
    scope: string;
    /** The left tool window and whether its sidebar is shown. */
    leftTool: string;
    leftOpen: boolean;
    rightTool: string;
    rightOpen: boolean;
    /** The right sidebar's content for a tool. */
    side: Snippet<[tool: string]>;
    /** A bottom tool is shown: `bottom` renders its panel in the dock, which fills the dock's height. */
    bottomOpen?: boolean;
    /** A panel under the editors; `selected` is the file the review is at, and `select` goes to one. */
    bottom?: Snippet<[view: { selected?: string; select: (path: string) => void }]>;
    /** The changeset is open in the controller. */
    onready: (controller: ReviewController) => void;
    /** The screen is torn down; its controller is gone. */
    onclose?: () => void;
  } = $props();

  /** Opens the context menu `spec` describes at the pointer; `subject` is what it was opened on. */
  const openMenu = (spec: MenuSpec, x: number, y: number, subject?: Partial<CommandContext>) =>
    void contextMenus.open(spec, x, y, subject);

  // The backend and the changeset are fixed for a mounted screen.
  // svelte-ignore state_referenced_locally
  const controller = new ReviewController(backend, changeset, (path, x, y) =>
    openMenu(fileHeaderMenu, x, y, { target: { path, dir: false } }),
  );
  // svelte-ignore state_referenced_locally
  controller.softWrap = softWrap;
  const tabs = controller.tabs;
  let tree = $state<FileTree>();
  let selected = $state<string>();
  /** The file picked in the tree, until the Changes tab moves on. */
  let picked = $state<string>();

  /** What the loading note names. */
  const project = $derived(session.repo === undefined ? "the changeset" : repoName(session.repo));
  const revision = $derived(changeset.layout?.shown.revision);
  /** The Changes tab lists no file, and says why instead. */
  const empty = $derived.by((): EmptyView | undefined => {
    const layout = changeset.layout;
    const facts = changeset.facts;
    if (changeset.loading || !layout || !facts || changeset.shownFiles.length || changeset.baseChanged)
      return undefined;
    const view = layout.shown.view.type;
    return emptyView({
      key: facts.target.changeset,
      target: facts.target,
      view: view === "commit" ? "commit" : view === "interdiff" ? "range" : "changeset",
      base: layout.base,
      changesetBase: facts.base,
      review: facts.review,
      excluded: layout.excluded,
      filter: facts.filter,
      narrowing: facts.narrowing,
      repo: session.repo,
      versions: versions.items,
      interdiff: layout.interdiff,
      commits: changeset.commits,
      commitIndex: changeset.shownCommit,
      pullRequest: platform.info,
      threads: { total: threads.items.length, open: threads.items.filter((t) => t.status === "open").length },
      now: minute.now,
    });
  });
  const narrowed = $derived(
    changeset.facts?.narrowing && narrowingLabel(changeset.facts.filter, changeset.facts.narrowing),
  );
  /** What the Changes tool says under its header while it lists no file. */
  const none = $derived(
    changeset.layout?.excluded.length
      ? "No files to review"
      : changeset.shownCommit === undefined
        ? "No changed files"
        : "No files in this commit",
  );
  /** The tree's group of excluded files is open. */
  let excludedOpen = $state(false);
  // Another changeset has other excluded files.
  let excludedOf: string | undefined;
  $effect(() => {
    const id = changeset.facts && keyId(changeset.facts.target.changeset);
    if (id === excludedOf) return;
    excludedOf = id;
    untrack(() => (excludedOpen = false));
  });

  function act(action: Action) {
    switch (action.id) {
      case "browse":
        return ontool("all");
      case "timeline":
        return ontool("timeline");
      case "comments":
        return ontool("comments");
      case "other":
        return void commands.run("changeset.switch");
      case "provider":
        return void commands.run("changeset.openInBrowser");
      case "back":
        return void changeset.showWhole();
      case "next":
        return void changeset.stepCommit(1);
      case "step":
        return void (action.n !== undefined && changeset.showCommit(action.n));
      case "filter":
        return onfilter();
      case "show":
        excludedOpen = true;
        return ontool("changes");
      case "narrow":
        return void changeset.narrow(action.filter);
      case "reset":
        return void changeset.narrow(undefined);
      case "pick":
        return void (action.n !== undefined && changeset.setRange(pickRange(versions.items, action.n)));
      case "openFile":
        return void (action.path !== undefined && controller.openSource(ON_SCREEN, action.path));
    }
  }
  /** The commit on view, while the changeset is reviewed by commit. */
  const commit = $derived(changeset.shownCommit === undefined ? undefined : changeset.commits[changeset.shownCommit]);
  const highlighted = $derived(pathOf(tabs.active) ?? selected);
  // The tree marks the file the viewer is in: a file tab's, else the one picked in the tree, else
  // the file at the top of the Changes tab, which follows its scrolling.
  const marked = $derived(pathOf(tabs.active) ?? picked ?? tabs.changesFile);
  // A pick in the tree gives way once the Changes tab moves on.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the file a dependency
    tabs.changesFile;
    untrack(() => (picked = undefined));
  });
  // The active tab has hunks to step through: the Changes tab, or a changed file's.
  const stepping = $derived(kindOf(tabs.active) === "changes" || controller.fileChanged(tabs.active));
  /** The commit the active file tab stays at. */
  const pinned = $derived(commitOf(tabs.active));
  const fileOf = (path: string) => changeset.reviewed(path);
  // The symbols of the file the review shows (the active file tab, else the one selected in the tree) are found first.
  $effect(() => {
    symbols.file = highlighted;
  });
  // The Structure tool lists the symbols of the file the active tab shows, in the view the tab reads.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the layout a dependency
    changeset.layout;
    structure.show(controller.structureTarget());
  });

  onMount(() => {
    const stopThreads = threads.start(backend);
    const stopPlatform = platform.start(backend);
    const stopVersions = versions.start(backend);
    const stopHook = exposeRendered(() => controller.rendered());
    const stopCommands = provideActions({
      copyText: (text) => void copyText(text),
      excludePath: (path, dir) => void changeset.excludePath(path, dir),
      reviewOnly: (path) => void changeset.reviewOnly(path),
      setFolderSeen: (path, seen) => {
        const shown = changeset.currentShown();
        if (shown) void changeset.setDirectorySeen(shown, path, seen);
      },
      closeTab: (tab) => tabs.close(tab),
      closeOtherTabs: (tab) => tabs.closeOthers(tab),
      stepTab: (delta) => tabs.step(delta),
      revealFile: () => {
        if (!tree) return false;
        void tree.reveal();
        return true;
      },
    });
    controller.open(true).then(
      (opened) => opened && onready(controller),
      (e: unknown) => void session.abandon(e),
    );
    return () => {
      stopThreads();
      stopPlatform();
      stopVersions();
      stopCommands();
      stopHook();
      controller.destroy();
      onclose?.();
    };
  });

  const range = $derived(changeset.layout?.interdiff);
  const readOnly = $derived(range?.markable === false);
  // A range shows the threads of its new side: they are read again for each range.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the range a dependency
    changeset.viewKey;
    untrack(() => void threads.reload());
  });

  // The open tabs, the active one or a view mode changed: the tabs are remembered.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the reads make the tabs a dependency
    [tabs.open.length, tabs.active, ...tabs.open.map((id) => tabs.modes.get(id))];
    untrack(() => {
      controller.scheduleSave();
      controller.pruneScopes();
      controller.syncFind();
    });
  });

  // A settled snapshot moved the changeset to a new revision, another changeset is on view, or
  // the review view shows another range.
  $effect(() => {
    if (revision === undefined) return;
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the range a dependency
    changeset.viewKey;
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the file filter a dependency
    changeset.baseChanged;
    untrack(() => {
      const layout = changeset.currentLayout();
      const facts = changeset.currentFacts();
      if (layout && facts) controller.update({ layout, facts });
    });
  });

  /** Shows the file in the Changes tab; an unchanged file is only selected. */
  function select(path: string) {
    selected = picked = path;
    void controller.revealInChanges(path);
  }

  function openSymbol(row: SymbolRow) {
    if (row.change) void controller.jumpToSymbol({ symbol: row.symbol, change: row.change });
  }

  /** Inside an editor the app's own menu replaces the webview's, except in a text field. It acts on the editor clicked. */
  function paneMenu(e: MouseEvent) {
    const target = e.target instanceof Element ? e.target : undefined;
    const content = target?.closest(".cm-content");
    if (e.defaultPrevented || !content || target?.closest("input, textarea")) return;
    e.preventDefault();
    openMenu(editorMenu, e.clientX, e.clientY, { editor: focusedEditor(content) });
  }
</script>

{#if leftOpen}
  <SideBar side="left" defaultWidth="var(--w-tree)">
    {#if leftTool === "comments"}
      <CommentsPanel
        onselect={(thread) => void controller.goToThread(thread)}
        card={(thread, open, toggle) => controller.threadCard(thread, open, toggle)}
      />
    {:else if changeset.loading}
      <p class="empty">Loading {project}…</p>
    {:else}
      <FileTree
        bind:this={tree}
        tool={leftTool === "all" || leftTool === "commits" ? leftTool : "changes"}
        paths={changeset.paths}
        {fileOf}
        {revision}
        list={(path) => changeset.listDirectory(path)}
        selected={marked}
        excluded={{
          lines: changeset.layout?.excludedLines ?? 0,
          items: changeset.layout?.excludedItems ?? 0,
          groups: changeset.layout?.excluded ?? [],
        }}
        bind:excludedOpen
        {narrowed}
        onreset={() => void changeset.narrow(undefined)}
        {none}
        changes={changeset.shownCommit === undefined ? changeset.paths.length : (changeset.facts?.files ?? 0)}
        commits={changeset.commits}
        commit={changeset.commitIndex}
        oncommit={(index) => void changeset.showCommit(index)}
        onselect={select}
        onopen={(path) => void controller.openSource(ON_SCREEN, path)}
        onopensymbol={openSymbol}
        onmenu={(path, dir, x, y) => openMenu(treeMenu(dir), x, y, { target: { path, dir } })}
      />
    {/if}
  </SideBar>
{/if}

<main>
  <div class="strip">
    {#if !changeset.loading}
      <TabStrip
        {tabs}
        files={changeset.shownFiles}
        {scope}
        {commit}
        unitOf={(id) => controller.commitUnit(id)}
        onmenu={(id, x, y) => openMenu(tabMenu, x, y, { tab: id })}
      />
    {/if}
    <span class="grow"></span>
    {#if !changeset.loading}
      <ViewControls
        mode={tabs.modes.get(tabs.active)}
        {stepping}
        file={kindOf(tabs.active) === "file"}
        empty={!!empty && tabs.active === CHANGES}
      />
    {/if}
  </div>
  {#if range && readOnly}
    <Banner message="Read-only: marks and comments are made on the latest push or All changes." />
  {/if}
  {#if pinned !== undefined}<CommitStrip id={pinned} commit={controller.commitUnit(tabs.active)} />{/if}
  {#if commit && tabs.active === CHANGES}<CommitHeader {commit} />{/if}
  {#if controller.find.open.has(tabs.active)}
    <FindBar
      find={controller.find}
      fileView={tabs.modes.get(tabs.active) === "file"}
      unchanged={kindOf(tabs.active) === "file" && !controller.fileChanged(tabs.active)}
      onoption={(patch) => controller.setFindOptions(patch)}
      onstep={(delta) => void controller.stepFind(delta)}
      onclose={() => controller.closeFind()}
    />
  {/if}
  {#if controller.find.wrapped}
    {#key controller.find.wrapped.n}
      <Toast
        message={controller.find.wrapped.text}
        duration={2500}
        onclose={() => (controller.find.wrapped = undefined)}
      />
    {/key}
  {/if}
  <!-- The handler only replaces the webview's menu inside an editor. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panes" class:readonly={readOnly} class:loading={changeset.loading} oncontextmenu={paneMenu}>
    {#if changeset.loading}
      <p class="empty">Loading {project}…</p>
    {:else if empty && tabs.active === CHANGES}
      <EmptyChanges view={empty} onact={act} />
    {/if}
    {#each tabs.open as id (id)}
      {@const path = pathOf(id)}
      {@const text = tabs.modes.get(id) === "file"}
      <!-- A file's tab has a pane for its diff, while the changeset changes the file, and one for its text. -->
      {#if controller.hasDiffPane(id)}
        <div class="pane" class:active={tabs.active === id && !text}>
          <div class="scroll" {@attach (root) => controller.attach(id, root)}></div>
        </div>
      {/if}
      {#if path !== undefined}
        <div class="pane" class:active={tabs.active === id && text}>
          <div class="scroll" {@attach (root) => controller.attachText(id, root)}></div>
        </div>
      {/if}
    {/each}
  </div>
  {#if bottomOpen}
    <Dock>{@render bottom?.({ selected: highlighted, select })}</Dock>
  {/if}
  {#if structure.popup}
    <StructurePopup
      onjump={(symbol, removed) => void controller.jumpToOutlineSymbol(symbol, removed)}
      oneditor={() => controller.focusEditor()}
    />
  {/if}
</main>

{#if rightOpen}
  <SideBar side="right" defaultWidth="var(--w-symbols)">{@render side(rightTool)}</SideBar>
{/if}

<style>
  main {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
  }
  .strip {
    display: flex;
    align-items: center;
    height: var(--h-tab);
    background: var(--bg-panel);
    border-bottom: 1px solid var(--line);
    flex: none;
  }
  .grow {
    flex: 1;
  }
  /* The editors stack their layers up to z-index 1000; isolated, those stay inside the panes and
     the header's popovers (z-index 20) lie above them. */
  .panes {
    position: relative;
    isolation: isolate;
    flex: 1;
    min-height: 0;
  }
  .empty {
    position: relative;
    z-index: 1;
    margin: 12px 16px;
    color: var(--text-3);
  }
  /* Every tab keeps its pane laid out, so scroll positions and editors survive switching. The view
     puts its scroll bar's gutter beside `.scroll`. */
  .pane {
    position: absolute;
    inset: 0;
    display: flex;
  }
  /* The views of the previous changeset stay laid out, unseen, until the new manifest replaces them. */
  .panes.loading .pane {
    visibility: hidden;
  }
  /* Not `visibility: visible` on the active pane: that would show it through a hidden ancestor. */
  .pane:not(.active) {
    visibility: hidden;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    min-width: 0;
    overflow-anchor: none;
  }
</style>
