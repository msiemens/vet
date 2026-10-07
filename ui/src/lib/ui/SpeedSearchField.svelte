<script lang="ts">
  import Icon from "./Icon.svelte";

  /** The typed query, shown above the list it filters; `found` is false when nothing matches it. */
  let { query, found = true }: { query: string; found?: boolean } = $props();
</script>

<div class="speed" class:none={!found} role="status" aria-label="Search">
  <Icon name="search" size={12} />
  <span class="query">{query}</span>
</div>

<style>
  .speed {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-row);
    margin: 4px 8px 0;
    padding: 0 8px;
    box-sizing: border-box;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-control);
    font-size: var(--fs-mono-s);
    color: var(--text-3);
    pointer-events: none;
  }
  .query {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
    font-family: var(--font-mono);
    color: var(--text);
  }
  .none .query {
    color: var(--del-text);
  }
</style>
