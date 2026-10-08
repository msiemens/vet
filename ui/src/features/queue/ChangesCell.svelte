<script lang="ts">
  import type { Changes } from "./queueRows";

  /** The lines a row changes: "…" while the backend counts them, nothing where there are none. */
  let { changes }: { changes: Changes } = $props();
</script>

{#if changes.kind === "lines"}<span class="mono"
    ><span class="add">+{changes.lines.added}</span> <span class="del">−{changes.lines.removed}</span></span
  >{:else if changes.kind === "pending"}<span class="pending" title="Counting the changed lines">…</span>{/if}

<style>
  .add {
    color: var(--add-text);
  }
  .del {
    color: var(--del-text);
  }
  .pending {
    color: var(--text-3);
  }
</style>
