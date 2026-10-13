<!-- A table of a settings page: a box round a `Table` with ruled rows. The rows are `children`;
     `addRow` is the last row, for adding one, and `below` sits in the box under the table. The rest
     props go to the element holding the rows. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import Table from "../../lib/ui/Table.svelte";
  import type { Column } from "../../lib/ui/columns";
  import type { Sort } from "../../lib/ui/sorting";

  let {
    name,
    columns,
    gap = "8px",
    sort = $bindable(),
    body = $bindable(),
    children,
    addRow,
    below,
    ...rest
  }: {
    /** Keeps the column widths and the sort under this name. */
    name: string;
    columns: readonly Column<never>[];
    /** Between the columns. */
    gap?: string;
    sort?: Sort;
    /** The element holding the rows. */
    body?: HTMLElement;
    children: Snippet;
    addRow?: Snippet;
    below?: Snippet;
  } & HTMLAttributes<HTMLDivElement> = $props();
</script>

<div class="box">
  <Table
    {name}
    {columns}
    bind:sort
    bind:body
    scroll={false}
    ruled
    {...rest}
    --table-gap={gap}
    --table-pad="10px"
    --table-pad-end="6px"
    --table-row="30px"
    --table-head="30px"
    --table-head-color="var(--text-3)"
  >
    {@render children()}
    {@render addRow?.()}
  </Table>
  {@render below?.()}
</div>

<style>
  .box {
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-editor);
    overflow: hidden;
  }
  .box :global(.row:last-child) {
    border-bottom: 0;
  }
</style>
