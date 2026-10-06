<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, untrack } from "svelte";
  import ContextMenu from "../../lib/ui/ContextMenu.svelte";
  import SideBar from "../../lib/ui/SideBar.svelte";
  import type { Backend } from "../../state/backend";
  import { changeset } from "../../state/changeset.svelte";
  import { threads } from "../../state/threads.svelte";
  import { CommentsPanel } from "../comments";
  import FileTree from "./FileTree.svelte";
  import type { Mode } from "./editor/mount";
  import "./editor/editor.css";
  import "./editor/threadCard.css";
  import { type Hud, ReviewController } from "./ReviewController";
  import { CHANGES, kindOf, pathOf } from "./Tabs.svelte";
  import TabStrip from "./TabStrip.svelte";
  import ViewControls from "./ViewControls.svelte";

  let {
    backend,
    hud,
    leftTool,
    leftOpen,
    rightTool,
    rightOpen,
    side,
    bottom,
    mode,
    onmode,
    onready,
    onfile,
  }: {
    backend: Backend;
    hud: Hud;
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
    /** A panel below the tabs, e.g. the run output. */
    bottom?: Snippet;
    /** The changeset is open; `firstPaintMs` runs from the open request to the first hunk's paint. */
    onready: (controller: ReviewController, firstPaintMs: number) => void;
    /** The file the review shows (the active file tab, else the one selected in the tree) changed. */
    onfile?: (path: string | undefined) => void;
  } = $props();

  let menu = $state<{ path: string; dir: boolean; x: number; y: number }>();
  // The backend is fixed for a mounted screen.
  // svelte-ignore state_referenced_locally
  const controller = new ReviewController(backend, hud, (path, x, y) => (menu = { path, dir: false, x, y }));
  // svelte-ignore state_referenced_locally
  controller.mode = mode;
  const tabs = controller.tabs;
  let selected = $state<string>();

  const revision = $derived(changeset.manifest?.revision);
  const highlighted = $derived(pathOf(tabs.active) ?? selected);
  const fileOf = (path: string) => changeset.file(path);
  $effect(() => onfile?.(highlighted));

  onMount(() => {
    const stopThreads = threads.start(backend);
    controller.open(true).then(
      (ms) => onready(controller, ms),
      (e: unknown) => (changeset.status = e instanceof Error ? e.message : String(e)),
    );
    return () => {
      stopThreads();
      controller.destroy();
    };
  });

  // A settled snapshot moved the changeset to a new revision.
  $effect(() => {
    if (revision === undefined) return;
    untrack(() => {
      const manifest = changeset.currentManifest();
      if (!manifest) return;
      tabs.keepOnly(new Set(manifest.files.map((f) => f.path)));
      controller.update(manifest);
    });
  });

  /** Shows the file in the Changes tab; an unchanged file is only selected. */
  function select(path: string) {
    selected = path;
    if (!fileOf(path)) return;
    tabs.active = CHANGES;
    const view = controller.changes;
    void view.goToFile(view.indexOf(path));
  }

  function runMenu(id: string, { path, dir }: { path: string; dir: boolean }) {
    if (id === "copy") navigator.clipboard.writeText(path).catch(() => (changeset.notice = "Could not copy the path"));
    else if (id === "exclude") void changeset.excludePath(path, dir);
    else if (id === "only") void changeset.reviewOnly(path);
  }
</script>

{#if leftOpen}
  <SideBar side="left" defaultWidth="var(--w-tree)">
    {#if leftTool === "files"}
      <FileTree
        paths={changeset.paths}
        {fileOf}
        {revision}
        list={(path) => changeset.listDirectory(path)}
        selected={highlighted}
        excluded={{ lines: changeset.manifest?.excludedLines ?? 0, items: changeset.manifest?.excludedItems ?? 0 }}
        onselect={select}
        onopen={(path) => void controller.openSource(path)}
        onmenu={(path, dir, x, y) => (menu = { path, dir, x, y })}
      />
    {:else if leftTool === "comments"}
      <CommentsPanel onselect={(thread) => void controller.goToThread(thread)} />
    {:else}
      <p class="soon">This tool window is not available yet.</p>
    {/if}
  </SideBar>
{/if}

<main>
  <div class="strip">
    <TabStrip {tabs} files={changeset.files} />
    <span class="grow"></span>
    {#if kindOf(tabs.active) !== "text"}
      <ViewControls {mode} {onmode} onstep={(delta) => controller.stepHunk(delta)} />
    {/if}
  </div>
  <div class="panes">
    {#each tabs.open as id (id)}
      <div class="pane" class:active={tabs.active === id}>
        <div class="scroll" {@attach (root) => controller.attach(id, root)}></div>
      </div>
    {/each}
  </div>
  {@render bottom?.()}
</main>

{#if rightOpen}
  <SideBar side="right" defaultWidth="var(--w-symbols)">{@render side(rightTool)}</SideBar>
{/if}

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    items={[
      { id: "copy", label: "Copy path" },
      { id: "exclude", label: "Exclude from review" },
      ...(menu.dir ? [{ id: "only", label: "Review only this folder" }] : []),
    ]}
    onselect={(id) => menu && runMenu(id, menu)}
    onclose={() => (menu = undefined)}
  />
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
  .soon {
    margin: 12px 10px;
    color: var(--text-3);
  }
</style>
