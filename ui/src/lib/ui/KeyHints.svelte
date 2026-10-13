<script lang="ts">
  // The line of keys under a popup: `KeyHint`s, and whatever else goes beside them.
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="hints">{@render children()}</div>

<style>
  .hints {
    display: flex;
    align-items: center;
    gap: 14px;
    height: var(--h-palette-foot);
    padding: 0 12px;
    border-top: 1px solid var(--line-soft);
    background: var(--bg-panel);
    font-size: 11.5px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    flex: none;
  }
</style>
