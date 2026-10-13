<!-- The head of a section of a side panel that folds: a chevron, its title and what it holds. The
     button is the whole row; `aside` puts further controls after it. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    title,
    open,
    count,
    ontoggle,
    before,
    after,
    summary,
    aside,
  }: {
    title: string;
    open: boolean;
    /** What the section holds: a number or a few words. */
    count?: number | string;
    ontoggle: () => void;
    /** In the button, before the title. */
    before?: Snippet;
    /** In the button, after the title. */
    after?: Snippet;
    /** In the button, at its end: what a folded section says of itself. */
    summary?: Snippet;
    /** Controls after the button, not part of it. */
    aside?: Snippet;
  } = $props();
</script>

<div class="head">
  <button class="flat tog" type="button" aria-expanded={open} onclick={ontoggle}>
    <span class="chev"><Icon name={open ? "open" : "closed"} size={10} /></span>
    {@render before?.()}
    {title}
    {@render after?.()}
    {#if count !== undefined && count !== ""}<span class="n">{count}</span>{/if}
    {#if summary}<span class="summary">{@render summary()}</span>{/if}
  </button>
  {@render aside?.()}
</div>

<style>
  .head {
    display: flex;
    align-items: center;
    border-top: 1px solid var(--line-soft);
    padding-right: 6px;
  }
  .tog {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-ctl);
    padding: 0 4px 0 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
    text-align: left;
  }
  .chev {
    flex: none;
    width: var(--w-chev);
    display: inline-flex;
    justify-content: center;
  }
  .n {
    font-weight: 400;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .summary {
    margin-left: auto;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
    white-space: nowrap;
    font-weight: 400;
    color: var(--text-2);
  }
</style>
