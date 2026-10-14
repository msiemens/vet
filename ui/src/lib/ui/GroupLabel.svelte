<!-- The line that names a group of rows in a list, with the count of what is in it. It is no row: nothing
     folds or is chosen on it. A group of tree rows that folds is a `TreeRow` with `heading`. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    count,
    children,
    trailing,
  }: {
    count?: number | string;
    /** What the group is called. */
    children: Snippet;
    /** After the count. */
    trailing?: Snippet;
  } = $props();
</script>

<div class="label">
  {@render children()}
  {#if count !== undefined}<span class="n">{count}</span>{/if}
  {@render trailing?.()}
</div>

<style>
  .label {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px 3px;
    font-size: var(--fs-mono-s);
    font-weight: 600;
    color: var(--text-3);
  }
</style>
