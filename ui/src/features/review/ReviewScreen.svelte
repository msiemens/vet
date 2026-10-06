<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, untrack } from "svelte";
  import ContextMenu from "../../lib/ui/ContextMenu.svelte";
  import SideBar from "../../lib/ui/SideBar.svelte";
  import type { Backend } from "../../state/backend";
  import { changeset } from "../../state/changeset.svelte";
  import ChangeCard from "./ChangeCard.svelte";
  import FileTree from "./FileTree.svelte";
  import type { Mode } from "./editor/mount";
  import "./editor/editor.css";
  import { type Hud, ReviewController } from "./ReviewController";
  import { CHANGES, pathOf } from "./Tabs.svelte";
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
    mode,
    onmode,
    onready,
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
    /** The changeset is open; `firstPaintMs` runs from the open request to the first hunk's paint. */
    onready: (controller: ReviewController, firstPaintMs: number) => void;
  } = $props();

  let menu = $state<{ path: string; x: number; y: number }>();
  // The backend is fixed for a mounted screen.
  // svelte-ignore state_referenced_locally
  const controller = new ReviewController(backend, hud, (path, x, y) => (menu = { path, x, y }));
  // svelte-ignore state_referenced_locally
  controller.mode = mode;
  const tabs = controller.tabs;
  let selected = $state<string>();

  const revision = $derived(changeset.manifest?.revision);
  const highlighted = $derived(pathOf(tabs.active) ?? selected);
  const fileOf = (path: string) => changeset.file(path);

  onMount(() => {
    controller.open(true).then(
      (ms) => onready(controller, ms),
      (e: unknown) => (changeset.status = e instanceof Error ? e.message : String(e)),
    );
    return () => controller.destroy();
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

  /** Shows the file in the Changes tab; a file with only non-text changes has nothing there, so it opens its own tab. */
  function select(path: string) {
    selected = path;
    if (!fileOf(path)?.hunks.length) {
      void controller.openFile(path);
      return;
    }
    tabs.active = CHANGES;
    const view = controller.changes;
    void view.goToFile(view.indexOf(path));
  }

  function copyPath(path: string) {
    navigator.clipboard.writeText(path).catch(() => (changeset.notice = "Could not copy the path"));
  }
</script>

{#if leftOpen}
  <SideBar side="left" defaultWidth="var(--w-tree)">
    {#if leftTool === "files"}
      <FileTree
        paths={changeset.paths}
        {fileOf}
        selected={highlighted}
        excluded={{ lines: changeset.manifest?.excludedLines ?? 0, items: changeset.manifest?.excludedItems ?? 0 }}
        onselect={select}
        onopen={(path) => void controller.openFile(path)}
        onmenu={(path, x, y) => (menu = { path, x, y })}
      />
    {:else}
      <p class="soon">This tool window is not available yet.</p>
    {/if}
  </SideBar>
{/if}

<main>
  <div class="strip">
    <TabStrip {tabs} files={changeset.files} />
    <span class="grow"></span>
    <ViewControls {mode} {onmode} onstep={(delta) => controller.stepHunk(delta)} />
  </div>
  <div class="panes">
    {#each tabs.open as id (id)}
      {@const path = pathOf(id)}
      {@const file = path === undefined ? undefined : fileOf(path)}
      <div class="pane" class:active={tabs.active === id}>
        {#if file?.nonText.length}
          <div class="cards">
            {#each file.nonText as item (item.kind)}
              {@const acknowledged = file.review.acknowledged.includes(item.kind)}
              <ChangeCard
                path={file.path}
                oldPath={file.oldPath}
                kind={item.kind}
                before={item.before}
                after={item.after}
                {acknowledged}
                ontoggle={() => changeset.acknowledge(file.path, item.kind, !acknowledged)}
              />
            {/each}
          </div>
        {/if}
        <div class="scroll" {@attach (root) => controller.attach(id, root)}></div>
      </div>
    {/each}
  </div>
</main>

{#if rightOpen}
  <SideBar side="right" defaultWidth="var(--w-symbols)">{@render side(rightTool)}</SideBar>
{/if}

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    items={[{ id: "copy", label: "Copy path" }]}
    onselect={() => menu && copyPath(menu.path)}
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
  .cards {
    flex: none;
    max-height: 40%;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    border-bottom: 1px solid var(--line);
  }
  .soon {
    margin: 12px 10px;
    color: var(--text-3);
  }
</style>
