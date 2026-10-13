<script lang="ts">
  // The first row of every bottom tool: its name, what it holds, its own controls and the button
  // that hides it.
  import type { Snippet } from "svelte";
  import IconButton from "./IconButton.svelte";

  let {
    title,
    count,
    summary,
    controls,
    onclose,
  }: {
    title: string;
    /** How many things the tool holds. */
    count?: number;
    /** A line of state beside the title. */
    summary?: string;
    /** The tool's own controls, right-aligned; a `flex: 1` element in them takes up the room between. */
    controls?: Snippet;
    /** Hides the tool. */
    onclose: () => void;
  } = $props();
</script>

<header>
  <span class="title">
    {title}
    {#if count !== undefined}<span class="count">{count}</span>{/if}
  </span>
  {#if summary}<span class="summary">{summary}</span>{/if}
  <div class="controls">{@render controls?.()}</div>
  <IconButton icon="minus" label="Hide {title}" size={22} iconSize={14} onclick={onclose} />
</header>

<style>
  header {
    flex: none;
    display: flex;
    align-items: center;
    height: 34px;
    padding: 0 8px 0 14px;
    border-bottom: 1px solid var(--line-soft);
    font-weight: 600;
    color: var(--text-2);
  }
  .title {
    flex: none;
  }
  .count {
    margin-left: 4px;
    font-weight: 400;
    color: var(--text-3);
  }
  .summary {
    flex: none;
    margin-left: 10px;
    font-weight: 400;
  }
  .controls {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    margin: 0 8px 0 10px;
    font-weight: 400;
  }
</style>
