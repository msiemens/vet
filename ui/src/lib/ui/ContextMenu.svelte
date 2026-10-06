<script lang="ts">
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
    items: { id: string; label: string }[];
    onselect: (id: string) => void;
    onclose: () => void;
  } = $props();

  let menu = $state<HTMLElement>();

  // Keep the menu inside the viewport once its size is known.
  const left = $derived(menu ? Math.max(0, Math.min(x, window.innerWidth - menu.offsetWidth)) : x);
  const top = $derived(menu ? Math.max(0, Math.min(y, window.innerHeight - menu.offsetHeight)) : y);

  $effect(() => menu?.querySelector("button")?.focus());
</script>

<svelte:window
  onpointerdown={(e) => !menu?.contains(e.target as Node) && onclose()}
  onkeydown={(e) => e.key === "Escape" && onclose()}
  onblur={onclose}
  onresize={onclose}
/>

<ul class="menu" role="menu" bind:this={menu} style:left="{left}px" style:top="{top}px">
  {#each items as item (item.id)}
    <li role="none">
      <button
        class="flat"
        role="menuitem"
        type="button"
        onclick={() => {
          onselect(item.id);
          onclose();
        }}
      >
        {item.label}
      </button>
    </li>
  {/each}
</ul>

<style>
  .menu {
    position: fixed;
    z-index: 40;
    min-width: 160px;
    margin: 0;
    padding: 4px 0;
    list-style: none;
    background: var(--bg-sheet);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }
  button {
    width: 100%;
    height: var(--h-row);
    display: flex;
    align-items: center;
    padding: 0 12px;
    text-align: left;
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
</style>
