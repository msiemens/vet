<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import type { Backend } from "../../state/backend";
  import { changeset } from "../../state/changeset.svelte";
  import ChangeCard from "./ChangeCard.svelte";
  import FileTree from "./FileTree.svelte";
  import type { Mode } from "./editor/mount";
  import type { HunkState } from "./editor/hunkBar";
  import "./editor/editor.css";
  import { mockNonText } from "./mock";
  import { type Hud, ReviewController } from "./ReviewController";
  import type { FileRow } from "./tree";

  let {
    backend,
    hud,
    excluded,
    side,
    onready,
  }: {
    backend: Backend;
    hud: Hud;
    excluded?: { label: string; count: number };
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
  let nonTextStates = $state<HunkState[]>(mockNonText.map(() => "unseen"));

  const nonText = $derived(mockNonText.map((change, i) => ({ change, state: nonTextStates[i] })));
  const file = $derived(changeset.files.find((f) => f.entry.path === (selected ?? changeset.files[0]?.entry.path)));

  onMount(() => {
    const c = new ReviewController(backend, root, {
      ...hud,
      opened: (name, files) => changeset.open(name, files),
      seen: (path, seen, oldBase) => changeset.update(path, seen, oldBase),
    });
    controller = c;
    c.open(true).then((ms) => onready(c, ms), (e) => (changeset.status = (e as Error).message));
  });

  function select(row: FileRow) {
    selected = row.path;
    showCards = !!row.nonText;
    if (row.index !== undefined) void controller?.view?.goToFile(row.index);
  }

  function setMode(next: Mode) {
    mode = next;
    controller?.setMode(next);
  }

  const toggleCard = (i: number) => (nonTextStates[i] = nonTextStates[i] === "seen" ? "unseen" : "seen");
  const hunk = (delta: number) => controller?.view && void controller.view.goToHunk(controller.view.current + delta);
</script>

<FileTree files={changeset.files} {nonText} {selected} {excluded} onselect={select} />

<main>
  <div class="strip">
    {#if file && !showCards}
      <span class="tab"><span>{file.entry.path.slice(file.entry.path.lastIndexOf("/") + 1)}</span></span>
    {:else if showCards}
      <span class="tab">Non-text changes</span>
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
  {#if showCards}
    <div class="cards">
      {#each nonText as { change, state }, i (change.path)}
        <ChangeCard {change} {state} ontoggle={() => toggleCard(i)} />
      {/each}
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
</style>
