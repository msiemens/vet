<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import Menu, { type MenuItem } from "./Menu.svelte";
  import Popover from "./Popover.svelte";

  let {
    items,
    onmain,
    onselect,
    label,
    main,
    menuFooter,
  }: {
    items: MenuItem[];
    onmain: () => void;
    onselect: (id: string) => void;
    label: string;
    main: Snippet;
    menuFooter?: Snippet;
  } = $props();
  let open = $state(false);
</script>

<span class="split">
  <button class="ctl main" type="button" aria-label={label} onclick={onmain}>{@render main()}</button>
  <Popover bind:open align="right">
    {#snippet trigger(toggle)}
      <button
        class="ctl caret"
        class:on={open}
        type="button"
        aria-label="{label} menu"
        aria-haspopup="menu"
        aria-expanded={open}
        onclick={toggle}><Icon name="caret" size={10} stroke={2} /></button
      >
    {/snippet}
    <Menu
      {items}
      onselect={(id) => {
        open = false;
        onselect(id);
      }}
    />
    {#if menuFooter}<div class="footer">{@render menuFooter()}</div>{/if}
  </Popover>
</span>

<style>
  .split {
    display: inline-flex;
  }
  /* The caret owns the one border between the parts. */
  .main {
    border-right-width: 0;
    border-radius: var(--radius) 0 0 var(--radius);
  }
  .caret {
    width: 22px;
    justify-content: center;
    padding: 0;
    border-radius: 0 var(--radius) var(--radius) 0;
  }
  .footer {
    border-top: 1px solid var(--line-soft);
    padding: 4px 0;
  }
</style>
