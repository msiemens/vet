<script lang="ts">
  import type { Snippet } from "svelte";

  let { open = $bindable(false), align = "left", trigger, children }: { open?: boolean; align?: "left" | "right"; trigger: Snippet<[toggle: () => void]>; children: Snippet } = $props();
  let root: HTMLElement;

  const toggle = () => (open = !open);
</script>

<svelte:window
  onpointerdown={(e) => open && !root.contains(e.target as Node) && (open = false)}
  onkeydown={(e) => open && e.key === "Escape" && (open = false)}
/>

<span class="pop" bind:this={root}>
  {@render trigger(toggle)}
  {#if open}
    <div class="sheet" class:right={align === "right"} role="dialog">{@render children()}</div>
  {/if}
</span>

<style>
  .pop { position: relative; display: inline-flex; }
  .sheet { position: absolute; top: calc(100% + 4px); left: 0; z-index: 20; min-width: 220px; background: var(--sheet); border: 1px solid var(--ln); border-radius: 8px; box-shadow: var(--shadow); }
  .sheet.right { left: auto; right: 0; }
</style>
