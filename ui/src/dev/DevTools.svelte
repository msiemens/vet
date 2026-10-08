<script lang="ts">
  import type { ReviewController } from "../features/review";
  import { changeset } from "../state/changeset.svelte";
  import CardGallery from "./CardGallery.svelte";
  import { hud } from "./probe.svelte";

  let {
    controller,
    open = $bindable(false),
  }: {
    controller?: ReviewController;
    open?: boolean;
  } = $props();

  let status = $state("");
  let gallery = $state(false);

  /** The changeset is open in `controller`, its first hunk painted `firstPaintMs` after the open request. */
  export function ready(firstPaintMs: number) {
    status = `${changeset.manifest?.name}: ${changeset.files.length} files, ${controller?.changes.totalHunks} hunks; first hunk painted in ${firstPaintMs.toFixed(0)} ms`;
  }
</script>

{#if open}
  <div class="bar">
    <strong>Dev</strong>
    <button class="ctl" type="button" onclick={() => (gallery = true)}>Change cards</button>
    <span class="status">{status}</span>
    <span
      class="hud mono"
      title="Live: keydown to paint, hover round trip, Ctrl-Alt bait bindings fired (must stay 0 when pressing AltGr character keys)"
    >
      key→paint {hud.keystroke.toFixed(1)} ms · hover {hud.hover.toFixed(1)} ms · collisions {hud.collisions}
    </span>
  </div>
{/if}

{#if gallery}<CardGallery onclose={() => (gallery = false)} />{/if}

<style>
  .bar {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 4px 10px;
    background: var(--bg-hunk);
    border-top: 1px solid var(--line);
    font-size: 12px;
    flex: none;
  }
  .status {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }
  .hud {
    color: var(--text-2);
  }
</style>
