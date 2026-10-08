<script lang="ts">
  import { onMount, untrack, type Snippet } from "svelte";
  import { rowWindow, scrollToRow, windowIndices } from "./rowWindow";

  let {
    count,
    key,
    row,
  }: {
    count: number;
    /** A stable key of row `index`, so rows keep their DOM while the window moves. */
    key: (index: number) => string;
    row: Snippet<[number]>;
  } = $props();

  // Rows beyond the viewport on each side, so a fast scroll does not show blanks before the next frame.
  const OVERSCAN = 8;
  // The scroller's vertical padding, which offsets every row.
  const PAD = 4;

  let el = $state<HTMLElement>();
  // Read from `--h-row` at mount: the rows (styled by the caller) and the spacers must agree.
  let rowHeight = $state(0);
  // Only these change while scrolling, and only when the window moves by a row, not per pixel.
  let start = $state(0);
  let end = $state(0);

  // The window catches up with `count` in an effect, after the rows render with the caller's new keys.
  const indices = $derived(windowIndices({ start, end }, count));

  function update(rows = count) {
    if (!el || !rowHeight) return;
    const w = rowWindow(el.scrollTop - PAD, el.clientHeight, rowHeight, rows, OVERSCAN);
    start = w.start;
    end = w.end;
  }

  /** Scrolls row `index` into view if it is not (a no-op for a row the viewer just clicked). */
  export function reveal(index: number) {
    if (!el || !rowHeight || index < 0 || index >= count) return;
    el.scrollTop = scrollToRow(index, el.scrollTop - PAD, el.clientHeight, rowHeight) + PAD;
  }

  onMount(() => {
    if (!el) return;
    rowHeight = parseFloat(getComputedStyle(el).getPropertyValue("--h-row"));
    update();
    const observer = new ResizeObserver(() => update());
    observer.observe(el);
    return () => observer.disconnect();
  });

  $effect(() => {
    const n = count;
    untrack(() => update(n));
  });
</script>

<div class="rows" bind:this={el} onscroll={() => update()}>
  <div style:height="{start * rowHeight}px"></div>
  {#each indices as index (key(index))}
    {@render row(index)}
  {/each}
  <div style:height="{Math.max(0, count - end) * rowHeight}px"></div>
</div>

<style>
  .rows {
    padding: 4px 0;
    overflow: hidden auto;
    min-height: 0;
  }
</style>
