<script lang="ts">
  import LineDelta from "../../lib/ui/LineDelta.svelte";
  import type { Changes } from "./queueRows";

  /** The lines a row changes: "…" while the backend counts them, nothing where there are none. */
  let { changes }: { changes: Changes } = $props();
</script>

{#if changes.kind === "lines"}<LineDelta {...changes.lines} />{:else if changes.kind === "pending"}<span
    class="pending"
    title="Counting the changed lines">…</span
  >{/if}

<style>
  .pending {
    color: var(--text-3);
  }
</style>
