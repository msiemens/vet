<script lang="ts">
  // One hint in the line of keys under a popup: the keys and what they do.
  import type { Snippet } from "svelte";

  let { keys, children }: { keys: string; children: Snippet } = $props();
</script>

<span><b>{keys}</b> {@render children()}</span>

<style>
  b {
    font: 500 var(--fs-mono-s) var(--font-mono);
    color: var(--text-2);
  }
</style>
