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
  import { session } from "../../state/session.svelte";
  import { symbols } from "../../state/symbols.svelte";
  import { threads } from "../../state/threads.svelte";
  import { CommentsPanel } from "../comments";
  import FileTree from "./FileTree.svelte";
  import "./editor/section.css";
  import "./editor/hunkBar.css";
  import "./editor/changeCard.css";
  import "./editor/highlights.css";
  import "./editor/hover.css";
  import "./editor/threadCard.css";
  import { type Probe, ReviewController } from "./ReviewController";
  import { editorMenu, fileCommands, fileHeaderMenu, tabMenu, treeMenu } from "./reviewCommands";
  import { kindOf, pathOf, tabOf } from "./Tabs.svelte";
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
    /** Unified or side by side; the owner applies a change to the controller. */
    mode: Mode;
    onmode: (mode: Mode) => void;
    /** The left tool window and whether its sidebar is shown. */
    leftTool: string;
    leftOpen: boolean;
    rightTool: string;
    rightOpen: boolean;
    /** The right sidebar's content for a tool. */
    side: Snippet<[tool: string]>;
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
  let selected = $state<string>();

  const revision = $derived(changeset.manifest?.revision);
  const highlighted = $derived(pathOf(tabs.active) ?? selected);
  const fileOf = (path: string) => changeset.file(path);
  // The Symbol panel lists the file the review shows (the active file tab, else the one selected in the tree) first.
  $effect(() => {
    symbols.file = highlighted;
  });

  onMount(() => {
    const stopThreads = threads.start(backend);
    const stopCommands = commands.register(
      ...fileCommands({
        copyText: (text) =>
          void navigator.clipboard.writeText(text).catch(() => (session.notice = "Could not copy to the clipboard")),
        excludePath: (path, dir) => void changeset.excludePath(path, dir),
        reviewOnly: (path) => void changeset.reviewOnly(path),
        closeTab: (kind, path) => tabs.close(tabOf(kind, path)),
        closeOtherTabs: (kind, path) => tabs.closeOthers(tabOf(kind, path)),
      }),
    );
    controller.open(true).then(
      (ms) => onready(controller, ms),
      (e: unknown) => void session.abandon(e),
    );
    return () => {
      stopThreads();
      stopCommands();
      controller.destroy();
      onclose?.();
    };
  });

  // A settled snapshot moved the changeset to a new revision, or another changeset is on view.
  $effect(() => {
    if (revision === undefined) return;
    untrack(() => {
      const manifest = changeset.currentManifest();
      if (!manifest) return;
      controller.update(manifest);
    });
  });

  /** Shows the file in the Changes tab; an unchanged file is only selected. */
  function select(path: string) {
    selected = path;
    void controller.revealInChanges(path);
  }

  /** Inside an editor the app's own menu replaces the webview's, except in a text field. */
  function paneMenu(e: MouseEvent) {
    const target = e.target instanceof Element ? e.target : undefined;
    if (e.defaultPrevented || !target?.closest(".cm-content") || target.closest("input, textarea")) return;
    e.preventDefault();
    openMenu(editorMenu, e.clientX, e.clientY);
  }
</script>

{#if leftOpen}
  <SideBar side="left" defaultWidth="var(--w-tree)">
    {#if leftTool === "comments"}
      <CommentsPanel onselect={(thread) => void controller.goToThread(thread)} />
    {:else}
      <FileTree
        paths={changeset.paths}
        {fileOf}
        {revision}
        list={(path) => changeset.listDirectory(path)}
        selected={highlighted}
        excluded={{ lines: changeset.manifest?.excludedLines ?? 0, items: changeset.manifest?.excludedItems ?? 0 }}
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
      files={changeset.files}
      delta={changeset.delta}
      onmenu={(id, x, y) => openMenu(tabMenu, x, y, { tab: { kind: kindOf(id), path: pathOf(id) } })}
    />
    <span class="grow"></span>
    {#if kindOf(tabs.active) !== "text"}
      <ViewControls {mode} {onmode} onstep={(delta) => controller.stepHunk(delta)} />
    {/if}
  </div>
  <!-- The handler only replaces the webview's menu inside an editor. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panes" oncontextmenu={paneMenu}>
    {#each tabs.open as id (id)}
      <div class="pane" class:active={tabs.active === id}>
        <div class="scroll" {@attach (root) => controller.attach(id, root)}></div>
      </div>
    {/each}
  </div>
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
  .panes {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  /* Every tab keeps its pane laid out, so scroll positions and editors survive switching. */
  .pane {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    visibility: hidden;
  }
  .pane.active {
    visibility: visible;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    overflow-anchor: none;
  }
</style>
