<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    open = $bindable(false),
    align = "left",
    above = false,
    trigger,
    children,
  }: {
    open?: boolean;
    align?: "left" | "right";
    /** Opens upwards, for a trigger at the bottom of the window. */
    above?: boolean;
    trigger: Snippet<[toggle: () => void]>;
    children: Snippet;
  } = $props();
  let root: HTMLElement;

  const toggle = () => (open = !open);

  /** Fixed beside the trigger, so a clipping parent (the title bar) cannot cut it off; kept inside the window. */
  function place(sheet: HTMLElement) {
    const anchor = root.getBoundingClientRect();
    const width = sheet.offsetWidth;
    const left = align === "right" ? anchor.right - width : anchor.left;
    if (above) sheet.style.bottom = `${window.innerHeight - anchor.top + 4}px`;
    else sheet.style.top = `${anchor.bottom + 4}px`;
    sheet.style.left = `${Math.max(4, Math.min(left, window.innerWidth - width - 4))}px`;
  }
</script>

<svelte:window
  onpointerdown={(e) => open && !root.contains(e.target as Node) && (open = false)}
  onkeydown={(e) => open && e.key === "Escape" && (open = false)}
  onresize={() => (open = false)}
/>

<span class="pop" bind:this={root}>
  {@render trigger(toggle)}
  {#if open}
    <div class="surface sheet" role="dialog" {@attach place}>{@render children()}</div>
  {/if}
</span>

<style>
  .pop {
    position: relative;
    display: inline-flex;
  }
  .sheet {
    position: fixed;
    z-index: 20;
    min-width: 220px;
  }
</style>
