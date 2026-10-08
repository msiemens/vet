<!-- A table: a header naming the columns over the rows its owner draws. Each element of class
     `row` in the body is laid out on the header's grid, and a divider in the header parts each two
     columns. The viewer resizes a column by dragging its divider or with the arrow keys on it, and a
     double click puts it back; the
     widths are kept under `name`. The rest props go to the element holding the rows, `body`.

     Its owner sizes it with `--table-gap` (between columns), `--table-pad` and `--table-pad-end`
     (before the first column and after the last), `--table-row` and `--table-head` (heights),
     `--table-head-color` and `--table-head-size` (the header's text). -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import { readStored, writeStored } from "../storage";
  import {
    type Column,
    gridTemplate,
    handleSide,
    minOf,
    parseWidths,
    resized,
    serializeWidths,
    withWidth,
    withoutWidth,
  } from "./columns";
  import ScrollArea from "./ScrollArea.svelte";

  let {
    name,
    columns,
    scroll = true,
    ruled = false,
    body = $bindable(),
    children,
    ...rest
  }: {
    /** Keeps the column widths under this name. */
    name: string;
    columns: readonly Column[];
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
  const storageKey = `vet.table.${name}`;
  // svelte-ignore state_referenced_locally
  let widths = $state(parseWidths(columns, readStored(storageKey)));
  const template = $derived(gridTemplate(columns, widths));

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

  const keep = () => writeStored(storageKey, serializeWidths(widths));

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
  <div class="head" bind:this={head}>
    {#each columns as column, index (column.id)}
      {@const side = handleSide(columns, index)}
      <span class="cell" class:right={column.align === "right"}>
        <span class="label">{column.label}</span>
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
    height: var(--table-head, 26px);
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
  .table :global(.row[aria-selected="true"]) {
    background: var(--toggle);
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
