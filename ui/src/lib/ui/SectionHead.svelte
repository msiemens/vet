<!-- The head of a section of a side panel that folds: a chevron, its title and what it holds. The
     button is the whole row; `aside` puts further controls after it. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Chevron from "./Chevron.svelte";

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
    <Chevron {open} />
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
    gap: var(--gap-tree);
    height: var(--h-ctl);
    padding: 0 4px 0 var(--pad-tree);
    font-size: var(--fs-mono);
    font-weight: 600;
    color: var(--text-2);
    text-align: left;
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
