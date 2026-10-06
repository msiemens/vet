<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, untrack } from "svelte";
  import type { Backend } from "../../state/backend";
  import { changeset } from "../../state/changeset.svelte";
  import ChangeCard from "./ChangeCard.svelte";
  import FileTree from "./FileTree.svelte";
  import type { Mode } from "./editor/mount";
  import "./editor/editor.css";
  import { type Hud, ReviewController } from "./ReviewController";
  import type { FileRow } from "./tree";

  let {
    backend,
    hud,
    side,
    onready,
  }: {
    backend: Backend;
    hud: Hud;
    /** The right-hand panel. */
    side?: Snippet;
    /** The changeset is open; `firstPaintMs` runs from the open request to the first hunk's paint. */
    onready: (controller: ReviewController, firstPaintMs: number) => void;
  } = $props();

  let root: HTMLElement;
  let controller = $state.raw<ReviewController>();
  let mode = $state<Mode>("unified");
  let selected = $state<string>();
  let showCards = $state(false);

  const revision = $derived(changeset.manifest?.revision);
  const file = $derived(changeset.files.find((f) => f.path === (selected ?? changeset.files[0]?.path)));

  onMount(() => {
    const c = new ReviewController(backend, root, hud);
    controller = c;
    c.open(true).then((ms) => onready(c, ms), (e) => (changeset.status = (e as Error).message));
    return () => c.destroy();
  });

  // A settled snapshot moved the changeset to a new revision.
  $effect(() => {
    const manifest = untrack(() => changeset.manifest);
    if (manifest && revision !== undefined && controller?.view && controller.view.manifest.revision !== revision) controller.show(manifest);
  });

  function select(row: FileRow) {
    selected = row.path;
    showCards = row.file.nonText.length > 0;
    if (!showCards) showText();
  }

  function showText() {
    if (!file) return;
    showCards = false;
    const view = controller?.view;
    const index = view?.indexOf(file.path) ?? -1;
    if (index >= 0) void view!.goToFile(index);
  }

  function setMode(next: Mode) {
    mode = next;
    controller?.setMode(next);
  }

  const hunk = (delta: number) => controller?.view && void controller.view.goToHunk(controller.view.current + delta);
</script>

<FileTree files={changeset.files} {selected} excluded={{ lines: changeset.manifest?.excludedLines ?? 0, items: changeset.manifest?.excludedItems ?? 0 }} onselect={select} />

<main>
  <div class="strip">
    {#if file}
      <span class="tab">{showCards ? "Non-text changes" : file.path.slice(file.path.lastIndexOf("/") + 1)}</span>
    {/if}
    <span class="grow"></span>
    <div class="seg" role="group" aria-label="View">
      <button type="button" class:on={mode === "unified"} onclick={() => setMode("unified")}>Unified</button>
      <button type="button" class:on={mode === "split"} onclick={() => setMode("split")}>Side by side</button>
    </div>
    <div class="seg" role="group" aria-label="Hunk navigation">
      <button type="button" title="Ctrl-Alt-Up" onclick={() => hunk(-1)}>Prev hunk</button>
      <button type="button" title="Ctrl-Alt-Down" onclick={() => hunk(1)}>Next hunk</button>
    </div>
  </div>
  <div class="scroll" hidden={showCards} bind:this={root}></div>
  {#if showCards && file}
    <div class="cards">
      {#each file.nonText as item (item.kind)}
        {@const acknowledged = file.review.acknowledged.includes(item.kind)}
        <ChangeCard path={file.path} oldPath={file.oldPath} kind={item.kind} before={item.before} after={item.after} {acknowledged} ontoggle={() => changeset.acknowledge(file.path, item.kind, !acknowledged)} />
      {/each}
      {#if file.hunks.length}
        <button class="text" type="button" onclick={showText}>Show text changes ({file.hunks.length} hunks)</button>
      {/if}
    </div>
  {/if}
</main>

{#if side}{@render side()}{/if}

<style>
  main { flex: 999 1 520px; min-width: 0; display: flex; flex-direction: column; background: var(--ed); }
  .strip { display: flex; align-items: center; gap: 4px; height: 32px; padding: 0 6px 0 0; background: var(--panel); border-bottom: 1px solid var(--ln); flex: none; }
  .tab { display: inline-flex; align-items: center; height: 32px; padding: 0 12px; box-sizing: border-box; border-right: 1px solid var(--ln2); border-bottom: 2px solid var(--acc); background: var(--ed); font-size: var(--fs-ctl); }
  .grow { flex: 1; }
  .seg { display: inline-flex; margin-right: 6px; }
  .seg button { height: 22px; padding: 0 8px; border: 1px solid var(--ln); background: var(--btn); font-size: 12px; cursor: pointer; }
  .seg button:first-child { border-radius: var(--radius) 0 0 var(--radius); }
  .seg button:last-child { border-left: 0; border-radius: 0 var(--radius) var(--radius) 0; }
  .seg button.on { background: var(--sel); color: var(--acc); }
  .seg button:hover:not(.on) { background: var(--hov); }
  .scroll { flex: 1; overflow-y: auto; min-height: 0; }
  .scroll[hidden] { display: none; }
  .cards { flex: 1; overflow-y: auto; padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  .text { align-self: flex-start; height: var(--h-ctl); padding: 0 10px; border: 1px solid var(--ln); border-radius: var(--radius); background: var(--btn); font-size: var(--fs-ctl); cursor: pointer; }
</style>
