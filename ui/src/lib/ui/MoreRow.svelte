<script lang="ts">
  // The line "N more" under a list that shows only its first rows; it shows the rest. `depth` indents
  // it with the rows it belongs to, under the names of a tree.
  let { count, depth = 0, onclick }: { count: number; depth?: number; onclick: () => void } = $props();
</script>

<div class="more" style:padding-left="calc(var(--inset-tree-name) + {depth} * var(--w-indent))">
  <button class="link" type="button" {onclick}>{count} more</button>
</div>

<style>
  .more {
    display: flex;
    align-items: center;
    min-height: var(--h-row);
    padding-right: 10px;
    box-sizing: border-box;
    font-size: var(--fs-ctl);
  }
</style>
