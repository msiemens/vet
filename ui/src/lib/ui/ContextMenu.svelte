<script lang="ts">
  import Menu from "./Menu.svelte";
  import type { MenuItem } from "./menuItem";

  let {
    x,
    y,
    items,
    onselect,
    onclose,
  }: {
    /** Viewport position of the pointer. */
    x: number;
    y: number;
    items: MenuItem[];
    onselect: (id: string) => void;
    onclose: () => void;
  } = $props();

  let sheet = $state<HTMLElement>();

  // Keep the menu inside the viewport once its size is known.
  const left = $derived(sheet ? Math.max(0, Math.min(x, window.innerWidth - sheet.offsetWidth)) : x);
  const top = $derived(sheet ? Math.max(0, Math.min(y, window.innerHeight - sheet.offsetHeight)) : y);
</script>

<svelte:window
  onpointerdown={(e) => !sheet?.contains(e.target as Node) && onclose()}
  onkeydown={(e) => e.key === "Escape" && onclose()}
  onblur={onclose}
  onresize={onclose}
/>

<div class="surface sheet" bind:this={sheet} style:left="{left}px" style:top="{top}px">
  <Menu
    {items}
    onselect={(id) => {
      onselect(id);
      onclose();
    }}
  />
</div>

<style>
  .sheet {
    position: fixed;
    z-index: 40;
    min-width: 160px;
  }
</style>
