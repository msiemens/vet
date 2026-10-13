<!-- The head of a popup at the caret: what it lists, how much, and its controls at the end. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    children,
    meta,
    controls,
  }: {
    /** What it lists; a name in `<code>` is set in the code font. */
    children: Snippet;
    /** Counts after it. */
    meta?: Snippet;
    /** Buttons at the end. */
    controls?: Snippet;
  } = $props();
</script>

<div class="head">
  <span class="title">{@render children()}</span>
  {@render meta?.()}
  <span class="grow"></span>
  {@render controls?.()}
</div>

<style>
  .head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    box-sizing: border-box;
    height: 33px;
    padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text);
  }
  .title :global(code) {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
</style>
