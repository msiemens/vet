<script lang="ts">
  import type { Label } from "./queueRows";

  let { label }: { label: Label } = $props();
</script>

{#if label.number}<span class="number">{label.number}</span>{/if}{label.text}

<style>
  .number {
    margin-right: 0.3em;
    color: var(--text-3);
  }
</style>
