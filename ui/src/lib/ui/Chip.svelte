<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    mono = false,
    bold = false,
    onclick,
    title,
    children,
  }: {
    mono?: boolean;
    bold?: boolean;
    onclick?: (e: MouseEvent) => void;
    title?: string;
    children: Snippet;
  } = $props();
</script>

<button class="chip" class:mono class:bold type="button" {title} {onclick}>{@render children()}</button>

<style>
  .chip {
    height: var(--h-ctl);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border: 1px solid var(--ln);
    border-radius: var(--radius);
    background: var(--btn);
    color: var(--tx);
    font-size: var(--fs-ctl);
    cursor: pointer;
    box-sizing: border-box;
    white-space: nowrap;
  }
  .chip:hover {
    border-color: var(--tx3);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .bold {
    font-weight: 600;
  }
</style>
