<!-- A table of a settings page in the design of the Language servers page: a box round a `Table` with
     ruled rows. The rows are `children`; `addRow` is the last row, for adding one, and `below` sits in the box
     under the table. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Table from "../../lib/ui/Table.svelte";
  import type { Column } from "../../lib/ui/columns";

  let {
    name,
    label,
    columns,
    children,
    addRow,
    below,
  }: {
    /** Keeps the column widths under this name. */
    name: string;
    label: string;
    columns: readonly Column<never>[];
    children: Snippet;
    addRow?: Snippet;
    below?: Snippet;
  } = $props();
</script>

<div class="box">
  <Table
    {name}
    {columns}
    scroll={false}
    ruled
    aria-label={label}
    --table-gap="8px"
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
