<script lang="ts">
  // A tinted tag that names what a view is narrowed to; with `onremove` it has a button that ends that.
  import type { Snippet } from "svelte";
  import IconButton from "./IconButton.svelte";

  let {
    title,
    mono = false,
    onremove,
    removeLabel = "Remove",
    removeTitle = removeLabel,
    children,
  }: {
    title?: string;
    /** The text is code. */
    mono?: boolean;
    onremove?: () => void;
    /** What the remove button is called, and its tooltip. */
    removeLabel?: string;
    removeTitle?: string;
    children: Snippet;
  } = $props();
</script>

<span class="tag" class:mono class:removable={onremove !== undefined} {title}>
  {@render children()}
  {#if onremove}
    <IconButton icon="close" label={removeLabel} title={removeTitle} size={16} iconSize={10} onclick={onremove} />
  {/if}
</span>

<style>
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    height: 20px;
    padding: 0 7px;
    box-sizing: border-box;
    border: 1px solid color-mix(in srgb, var(--accent) 38%, transparent);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
  .tag.removable {
    padding-right: 2px;
  }
  .tag.mono {
    font: var(--fs-mono-s) var(--font-mono);
  }
  .tag :global(.ib) {
    color: inherit;
  }
</style>
