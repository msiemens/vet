<!-- A table: a header naming the columns over the rows its owner draws. Each element of class
     `row` in the body is laid out on the header's grid, and a divider in the header parts each two
     columns. The viewer resizes a column by dragging its divider or with the arrow keys on it, and a
     double click puts it back. A click on a sortable column's header sets `sort`, by which the owner
     orders its rows (`sortRows`). The widths and the sort are kept as preferences under `name`. The rest props go
     to the element holding the rows, `body`.

     Its owner sizes it with `--table-gap` (between columns), `--table-pad` and `--table-pad-end`
     (before the first column and after the last), `--table-row` and `--table-head` (heights),
     `--table-head-color` and `--table-head-size` (the header's text). -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import { Preference } from "../preferences.svelte";
  import Icon from "./Icon.svelte";
  import {
    type Column,
    type Widths,
    gridTemplate,
    handleSide,
    minOf,
    parseWidths,
    resized,
    withWidth,
    withoutWidth,
  } from "./columns";
  import ScrollArea from "./ScrollArea.svelte";
  import { type Sort, nextSort, parseSort } from "./sorting";

  let {
    name,
    columns,
    sort = $bindable(),
    scroll = true,
    ruled = false,
    body = $bindable(),
    children,
    ...rest
  }: {
    /** Keeps the column widths and the sort under this name. */
    name: string;
    /** Of rows of any kind: the table itself never reads a row. */
    columns: readonly Column<never>[];
    /** The column the rows are sorted by; none for the default order. */
    sort?: Sort;
    /** The rows scroll under the header, with a scroll bar; without it the table is as tall as its rows. */
    scroll?: boolean;
    /** A line under each row. */
    ruled?: boolean;
    /** The element holding the rows. */
    body?: HTMLElement;
    children: Snippet;
  } & HTMLAttributes<HTMLDivElement> = $props();

  const STEP = 16;
  // The name and the columns do not change for a mounted table.
  // svelte-ignore state_referenced_locally
  const widthsKept = new Preference<Widths>("app", `table.${name}`, (value) => parseWidths(columns, value), {});
  // svelte-ignore state_referenced_locally
  const sortKept = new Preference<Sort | undefined>(
    "app",
    `table.${name}.sort`,
    (value) => parseSort(columns, value),
    undefined,
  );
  let widths = $state(widthsKept.value);
  const template = $derived(gridTemplate(columns, widths));
  sort = sortKept.value;

  function sortBy(column: Column<never>) {
    sort = nextSort(sort, column);
    sortKept.value = sort;
  }

  function ariaSort(column: Column<never>) {
    if (!column.sortBy) return undefined;
    return sort?.column === column.id ? sort.direction : "none";
  }

  let head: HTMLElement;
  let drag: { index: number; at: number; start: number; room: number } | undefined;
  let dragging = $state<number>();

  /** The column's width as laid out now, and how much the flexible column can give. */
  function measure(index: number) {
    const cells = head.children;
    const flex = columns.findIndex((c) => c.flex);
    const room = flex < 0 ? Infinity : cells[flex].getBoundingClientRect().width - minOf(columns[flex]);
    return { start: cells[index].getBoundingClientRect().width, room };
  }

  function resize(index: number, start: number, delta: number, room: number) {
    const column = columns[index];
    const side = handleSide(columns, index);
    if (side) widths = withWidth(widths, column.id, resized(column, side, start, delta, room));
  }

  const keep = () => (widthsKept.value = widths);

  function onpointerdown(e: PointerEvent, index: number) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { index, at: e.clientX, ...measure(index) };
    dragging = index;
  }

  function onpointermove(e: PointerEvent) {
    if (drag) resize(drag.index, drag.start, e.clientX - drag.at, drag.room);
  }

  function onpointerup() {
    if (!drag) return;
    drag = undefined;
    dragging = undefined;
    keep();
  }

  function onkeydown(e: KeyboardEvent, index: number) {
    const sign = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!sign) return;
    e.preventDefault();
    const { start, room } = measure(index);
    resize(index, start, sign * STEP, room);
    keep();
  }

  function reset(index: number) {
    widths = withoutWidth(widths, columns[index].id);
    keep();
  }
</script>

<div class="table" class:scroll class:ruled style:--columns={template}>
  <div class="head" role="row" bind:this={head}>
    {#each columns as column, index (column.id)}
      {@const side = handleSide(columns, index)}
      {@const sorted = sort?.column === column.id ? sort.direction : undefined}
      <span class="cell" class:right={column.align === "right"} role="columnheader" aria-sort={ariaSort(column)}>
        {#if column.sortBy}
          <button class="sort" type="button" onclick={() => sortBy(column)}>
            <span class="label">{column.label}</span>
            {#if sorted}<Icon name={sorted === "ascending" ? "up" : "down"} size={11} />{/if}
          </button>
        {:else}
          <span class="label">{column.label}</span>
        {/if}
        {#if index < columns.length - 1 && side !== "end" && handleSide(columns, index + 1) !== "start"}
          <span class="divider"></span>
        {/if}
        {#if side}
          <!-- A focusable separator is a widget (WAI-ARIA window splitter). -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
          <div
            class="edge {side}"
            class:dragging={dragging === index}
            role="separator"
            aria-orientation="vertical"
            aria-label="Width of {column.name ?? column.label}"
            aria-valuenow={widths[column.id] ?? column.width}
            tabindex="0"
            onpointerdown={(e) => onpointerdown(e, index)}
            {onpointermove}
            {onpointerup}
            onkeydown={(e) => onkeydown(e, index)}
            ondblclick={() => reset(index)}
          ></div>
        {/if}
      </span>
    {/each}
  </div>
  {#if scroll}
    <ScrollArea bind:viewport={body} {...rest}>{@render children()}</ScrollArea>
  {:else}
    <div bind:this={body} {...rest}>{@render children()}</div>
  {/if}
</div>

<style>
  .table {
    --gap: var(--table-gap, 12px);
    --pad: var(--table-pad, 16px);
    --pad-end: var(--table-pad-end, 16px);
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .head,
  .table :global(.row) {
    display: grid;
    grid-template-columns: var(--columns);
    align-items: center;
    column-gap: var(--gap);
    padding: 0 var(--pad-end) 0 var(--pad);
    box-sizing: border-box;
  }
  .head {
    flex: none;
    height: var(--table-head, var(--h-ctl));
    border-bottom: 1px solid var(--line-soft);
    color: var(--table-head-color, var(--text-2));
    font-size: var(--table-head-size, var(--fs-ctl));
  }
  /* Over the rows' scroll bar too, so the columns line up. */
  .scroll > .head {
    padding-right: calc(var(--pad-end) + var(--w-scrollbar));
  }
  .table :global(.row) {
    width: 100%;
    height: var(--table-row, var(--h-queue-row));
    border-radius: 0;
    text-align: left;
  }
  .ruled :global(.row) {
    border-bottom: 1px solid var(--line-soft);
  }
  .cell {
    position: relative;
    align-self: stretch;
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .cell.right {
    justify-content: flex-end;
  }
  /* A sortable header is one button over the whole cell, from divider to divider and the header's full
     height (the first and last cell run to the header's edges). The grips lie above it. The sort arrow
     sits at the cell's right edge; in a right-aligned column it goes before the label, so the label
     stays flush right and does not move when the sort changes. */
  .sort {
    position: absolute;
    top: 0;
    bottom: 0;
    left: calc(var(--gap) / -2);
    right: calc(var(--gap) / -2);
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 0;
    padding: 0 calc(var(--gap) / 2);
    border: 0;
    border-radius: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: default;
  }
  .cell:first-child > .sort {
    left: calc(var(--pad) * -1);
    padding-left: var(--pad);
  }
  .cell:last-child > .sort {
    right: calc(var(--pad-end) * -1);
    padding-right: var(--pad-end);
  }
  .cell.right > .sort {
    justify-content: flex-end;
  }
  .cell.right > .sort > :global(svg) {
    order: -1;
  }
  .sort:hover {
    background: var(--hover);
  }
  .sort:active {
    background: var(--press);
  }
  .sort > :global(svg) {
    flex: none;
  }
  .cell:not(.right) > .sort > :global(svg) {
    margin-left: auto;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* A grip in the middle of the gap between two columns, drawn as the divider there. */
  .edge {
    position: absolute;
    z-index: 1;
    top: 0;
    bottom: 0;
    width: var(--w-splitter);
    cursor: col-resize;
    touch-action: none;
  }
  .edge.end {
    right: calc((var(--gap) + var(--w-splitter)) / -2);
  }
  .edge.start {
    left: calc((var(--gap) + var(--w-splitter)) / -2);
  }
  .edge::before,
  .divider {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--line);
  }
  .edge::before {
    content: "";
    left: calc(50% - 0.5px);
  }
  /* Between two columns neither of which this gap resizes. */
  .divider {
    right: calc(var(--gap) / -2 - 0.5px);
  }
  .edge:hover::before,
  .edge.dragging::before,
  .edge:focus-visible::before {
    background: var(--accent);
  }
  .edge:focus-visible {
    outline: none;
  }
</style>
