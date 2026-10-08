<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, untrack } from "svelte";
  import type { MenuSpec } from "../../lib/commandMenu";
  import SideBar from "../../lib/ui/SideBar.svelte";
  import type { Mode } from "../../lib/viewMode";
  import type { Backend } from "../../state/backend";
  import type { Changeset } from "../../state/changeset.svelte";
  import { type CommandContext, commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { platform } from "../../state/platform.svelte";
  import { session } from "../../state/session.svelte";
  import { symbols } from "../../state/symbols.svelte";
  import { threads } from "../../state/threads.svelte";
  import { versions } from "../../state/versions.svelte";
  import { CommentsPanel } from "../comments";
  import FileTree from "./FileTree.svelte";
  import "./editor/section.css";
  import "./editor/hunkBar.css";
  import "./editor/fileChanges.css";
  import "./editor/scrollMarks.css";
  import "./editor/changeCard.css";
  import "./editor/highlights.css";
  import "./editor/hover.css";
  import "./editor/threadCard.css";
  import { focusedEditor } from "./editor/semantic";
  import { type Probe, ReviewController } from "./ReviewController";
  import { editorMenu, fileCommands, fileHeaderMenu, tabMenu, treeMenu } from "./reviewCommands";
  import CommitHeader from "./CommitHeader.svelte";
  import { CHANGES, kindOf, pathOf, showsDiff, subjectOf, tabOf } from "./Tabs.svelte";
  import TabStrip from "./TabStrip.svelte";
  import ViewControls from "./ViewControls.svelte";

  let {
    backend,
    changeset,
    probe,
    leftTool,
    leftOpen,
    rightTool,
    rightOpen,
    side,
    bottom,
    mode,
    onmode,
    onready,
    onclose,
  }: {
    backend: Backend;
    /** What the backend sent about the open changeset. */
    changeset: Changeset;
    /** The dev tools' measurements of the editors, if they are on. */
    probe?: Probe;
    /** Unified or split; the owner applies a change to the controller. */
    mode: Mode;
    onmode: (mode: Mode) => void;
    /** The left tool window and whether its sidebar is shown. */
    leftTool: string;
    leftOpen: boolean;
    rightTool: string;
    rightOpen: boolean;
    /** The right sidebar's content for a tool. */
    side: Snippet<[tool: string]>;
    /** A panel under the editors; `selected` is the file the review is at, and `select` goes to one. */
    bottom?: Snippet<[view: { selected?: string; select: (path: string) => void }]>;
    /** The changeset is open; `firstPaintMs` runs from the open request to the first hunk's paint. */
    onready: (controller: ReviewController, firstPaintMs: number) => void;
    /** The screen is torn down; its controller is gone. */
    onclose?: () => void;
  } = $props();

  /** Opens the context menu `spec` describes at the pointer; `subject` is what it was opened on. */
  const openMenu = (spec: MenuSpec, x: number, y: number, subject?: Partial<CommandContext>) =>
    void contextMenus.open(spec, x, y, subject);

  // The backend and the changeset are fixed for a mounted screen.
  // svelte-ignore state_referenced_locally
  const controller = new ReviewController(
    backend,
    changeset,
    (path, x, y) => openMenu(fileHeaderMenu, x, y, { target: { path, dir: false } }),
    probe,
  );
  // svelte-ignore state_referenced_locally
  controller.mode = mode;
  const tabs = controller.tabs;
  let tree = $state<FileTree>();
  let selected = $state<string>();
  /** The file picked in the tree, until the Changes tab moves on. */
  let picked = $state<string>();

  const revision = $derived(changeset.manifest?.revision);
  /** The chosen target when it already contains the changeset, so there is nothing to show. */
  const containedIn = $derived(changeset.files.length ? undefined : changeset.manifest?.base.containedIn);
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
  // What the active tab of a changed file shows, which the control above the editors switches.
  const fileView = $derived.by(() => {
    const path = pathOf(tabs.active);
    if (path === undefined || !changeset.file(path)) return undefined;
    return kindOf(tabs.active) === "file" ? "diff" : "file";
  });
  const fileOf = (path: string) => changeset.file(path);
  // The Symbol panel lists the file the review shows (the active file tab, else the one selected in the tree) first.
  $effect(() => {
    symbols.file = highlighted;
  });

  onMount(() => {
    const stopThreads = threads.start(backend);
    const stopPlatform = platform.start(backend);
    const stopVersions = versions.start(backend);
    const stopCommands = commands.register(
      ...fileCommands({
        copyText: (text) =>
          void navigator.clipboard.writeText(text).catch(() => (session.notice = "Could not copy to the clipboard")),
        excludePath: (path, dir) => void changeset.excludePath(path, dir),
        reviewOnly: (path) => void changeset.reviewOnly(path),
        closeTab: (kind, path) => tabs.close(tabOf(kind, path)),
        closeOtherTabs: (kind, path) => tabs.closeOthers(tabOf(kind, path)),
        revealFile: () => {
          if (!tree) return false;
          void tree.reveal();
          return true;
        },
      }),
    );
    controller.open(true).then(
      (ms) => onready(controller, ms),
      (e: unknown) => void session.abandon(e),
    );
    return () => {
      stopThreads();
      stopPlatform();
      stopVersions();
      stopCommands();
      controller.destroy();
      onclose?.();
    };
  });

  const range = $derived(changeset.manifest?.interdiff);
  const readOnly = $derived(range?.markable === false);
  // A range shows the threads of its new side: they are read again for each range.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the range a dependency
    changeset.viewKey;
    untrack(() => void threads.reload());
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
      const manifest = changeset.currentManifest();
      if (!manifest) return;
      controller.update(manifest);
    });
  });

  /** Shows the file in the Changes tab; an unchanged file is only selected. */
  function select(path: string) {
    selected = picked = path;
    void controller.revealInChanges(path);
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
      <CommentsPanel onselect={(thread) => void controller.goToThread(thread)} />
    {:else}
      <FileTree
        bind:this={tree}
        tool={leftTool === "all" || leftTool === "commits" ? leftTool : "changes"}
        paths={changeset.paths}
        {fileOf}
        {revision}
        list={(path) => changeset.listDirectory(path)}
        selected={marked}
        excluded={{ lines: changeset.manifest?.excludedLines ?? 0, items: changeset.manifest?.excludedItems ?? 0 }}
        changes={changeset.shownCommit === undefined
          ? changeset.files.length
          : (changeset.manifest?.changesetFiles ?? 0)}
        commits={changeset.commits}
        commit={changeset.commitIndex}
        oncommit={(index) => void changeset.showCommit(index)}
        onselect={select}
        onopen={(path) => void controller.openSource(path)}
        onmenu={(path, dir, x, y) => openMenu(treeMenu(dir), x, y, { target: { path, dir } })}
      />
    {/if}
  </SideBar>
{/if}

<main>
  <div class="strip">
    <TabStrip
      {tabs}
      files={changeset.shownFiles}
      {commit}
      onmenu={(id, x, y) => openMenu(tabMenu, x, y, { tab: { kind: kindOf(id), path: subjectOf(id) } })}
    />
    <span class="grow"></span>
    <ViewControls
      {mode}
      {onmode}
      onstep={(delta) => controller.stepHunk(delta)}
      stepping={showsDiff(tabs.active) || fileView !== undefined}
      file={fileView}
      onfile={() => void controller.toggleFileView()}
    />
  </div>
  {#if range && readOnly}
    <p class="note" role="status">
      Read-only: marks and comments only work in a range that ends at the latest version.
    </p>
  {/if}
  {#if commit && tabs.active === CHANGES}<CommitHeader {commit} />{/if}
  <!-- The handler only replaces the webview's menu inside an editor. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panes" class:readonly={readOnly} oncontextmenu={paneMenu}>
    {#if containedIn}
      <p class="empty">
        Nothing to review: {containedIn} already contains {changeset.manifest?.target.branch ?? "HEAD"}
      </p>
    {/if}
    {#each tabs.open as id (id)}
      <div class="pane" class:active={tabs.active === id}>
        <div class="scroll" {@attach (root) => controller.attach(id, root)}></div>
      </div>
    {/each}
  </div>
  {@render bottom?.({ selected: highlighted, select })}
</main>

{#if rightOpen}
  <SideBar side="right" defaultWidth="var(--w-symbols)">{@render side(rightTool)}</SideBar>
{/if}

<style>
  main {
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
  .note {
    flex: none;
    margin: 0;
    padding: 4px 12px;
    background: color-mix(in srgb, var(--old-base) 14%, var(--bg-panel));
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-ctl);
    color: var(--text-2);
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
