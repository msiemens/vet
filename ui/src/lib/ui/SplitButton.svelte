<script module lang="ts">
  export interface MenuItem {
    id: string;
    label: string;
    detail?: string;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";

  let { items, onmain, onselect, label, main, menuFooter }: { items: MenuItem[]; onmain: () => void; onselect: (id: string) => void; label: string; main: Snippet; menuFooter?: Snippet } = $props();
  let open = $state(false);
</script>

<span class="split">
  <button class="main" type="button" aria-label={label} onclick={onmain}>{@render main()}</button>
  <Popover bind:open align="right">
    {#snippet trigger(toggle)}
      <button class="caret" type="button" aria-label="{label} menu" aria-haspopup="menu" aria-expanded={open} onclick={toggle}><Icon name="caret" size={10} stroke={2} /></button>
    {/snippet}
    <ul role="menu">
      {#each items as item (item.id)}
        <li role="none">
          <button role="menuitem" type="button" onclick={() => ((open = false), onselect(item.id))}>
            <span>{item.label}</span>
            {#if item.detail}<span class="mono detail">{item.detail}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
    {#if menuFooter}<div class="footer">{@render menuFooter()}</div>{/if}
  </Popover>
</span>

<style>
  .split { display: inline-flex; }
  button { height: var(--h-ctl); display: inline-flex; align-items: center; gap: 6px; border: 1px solid var(--ln); background: var(--btn); color: var(--tx); font-size: var(--fs-ctl); cursor: pointer; box-sizing: border-box; }
  .main { padding: 0 8px; border-radius: var(--radius) 0 0 var(--radius); }
  .caret { width: 22px; justify-content: center; padding: 0; border-left: 0; border-radius: 0 var(--radius) var(--radius) 0; }
  button:hover { border-color: var(--tx3); }
  ul { list-style: none; margin: 0; padding: 4px 0; }
  li button { width: 100%; height: var(--h-row); border: 0; background: transparent; border-radius: 0; padding: 0 12px; justify-content: space-between; white-space: nowrap; gap: 16px; }
  li button:hover { background: var(--hov); }
  .detail { font-size: var(--fs-mono-s); color: var(--tx3); }
  .footer { border-top: 1px solid var(--ln2); padding: 4px 0; }
</style>
