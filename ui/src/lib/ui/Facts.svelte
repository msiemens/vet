<script lang="ts">
  // Facts about one thing, as a grid of labels and values: write the rows as `dt` and `dd` pairs. A
  // label is right-aligned in a column of one width (`--w-facts-label`, which the settings sheets'
  // form labels share); a `dt` left empty puts its `dd` under the values (a problem, a hint, the
  // buttons), and `dt.top` keeps a label at the top of a value that runs over several lines.
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();
</script>

<dl>{@render children()}</dl>

<style>
  dl {
    display: grid;
    grid-template-columns: var(--w-facts-label) 1fr;
    gap: 6px 12px;
    margin: 0;
    align-items: center;
  }
  dl :global(dt) {
    text-align: right;
    color: var(--text-2);
  }
  dl :global(dt.top) {
    align-self: start;
  }
  dl :global(dd) {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
