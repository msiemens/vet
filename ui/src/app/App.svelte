<script lang="ts">
  import { onMount } from "svelte";
  import DevTools from "../dev/DevTools.svelte";
  import { hudListeners } from "../dev/hud.svelte";
  import { Header, StatusFooter, ToolRail, mockFooter, mockHeader } from "../features/chrome";
  import { FilterPopover } from "../features/filter";
  import { ReviewScreen, type ReviewController } from "../features/review";
  import { RunButton } from "../features/run";
  import { SymbolPanel } from "../features/symbols";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import { changeset } from "../state/changeset.svelte";

  let controller = $state.raw<ReviewController>();
  let tools: DevTools;
  let devOpen = $state(false);
  let status = $state("");
  let theme = $state<Theme>(storedTheme());

  const completion = $derived.by(() => {
    const c = changeset.progress;
    return { seenPct: c.seenPct, oldPct: c.oldPct, title: `${c.seen} of ${c.total} changed lines and other changes seen · ${c.old} against an old base` };
  });
  const filteredOut = $derived(changeset.manifest?.filteredOut ?? 0);
  const footer = $derived({ ...mockFooter, snapshot: `#${changeset.manifest?.revision ?? 0}` });

  onMount(() => {
    applyTheme(theme);
    void changeset.connect();
  });

  function ready(c: ReviewController, firstPaintMs: number) {
    controller = c;
    status = `${changeset.manifest?.name}: ${changeset.files.length} files, ${c.view!.totalHunks} hunks; first hunk painted in ${firstPaintMs.toFixed(0)} ms`;
    void tools.runIfRequested();
  }
</script>

<Header info={mockHeader} {completion}>
  {#snippet filter()}<FilterPopover excludedCount={filteredOut} />{/snippet}
  {#snippet run()}<RunButton />{/snippet}
</Header>

<div class="body">
  <ToolRail active="files" />
  {#if changeset.backend}
    <ReviewScreen backend={changeset.backend} hud={hudListeners} onready={ready}>
      {#snippet side()}<SymbolPanel />{/snippet}
    </ReviewScreen>
  {:else}
    <p class="message">{changeset.status}</p>
  {/if}
</div>

<DevTools bind:this={tools} {controller} bind:open={devOpen} bind:status {theme} ontheme={(t) => applyTheme((theme = t))} />

<StatusFooter info={footer}>
  {#snippet end()}<button class="dev" type="button" aria-pressed={devOpen} onclick={() => (devOpen = !devOpen)}>Dev</button>{/snippet}
</StatusFooter>

<style>
  .body { flex: 1; display: flex; min-height: 0; }
  .message { margin: 16px; color: var(--tx2); }
  .dev { border: 0; background: transparent; color: var(--tx3); font-size: 11px; cursor: pointer; padding: 0; }
  .dev[aria-pressed="true"] { color: var(--acc); }
</style>
