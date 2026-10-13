<script lang="ts">
  import Icon from "./Icon.svelte";
  import Meter from "./Meter.svelte";
  import { seenSegments } from "./seenMeter";

  /**
   * How much of the changes is reviewed: a meter of what is seen (blue) and what is seen only against
   * an old base (stripes), in percent. `pct` adds the number after it and `done` a check before it (a
   * finished review; without `seen` the check stands alone); either makes it a cell, and `title` then
   * belongs to the cell.
   */
  let {
    seen,
    old = 0,
    width,
    pct = false,
    done = false,
    title,
  }: {
    seen?: number;
    old?: number;
    width?: number;
    pct?: boolean;
    done?: boolean;
    title?: string;
  } = $props();
</script>

{#if done || pct}
  <span class="cell" {title}>
    {#if done}<span class="done"><Icon name="check" size={12} stroke={2.4} /></span>{/if}
    {#if seen !== undefined}
      <Meter {width} segments={seenSegments(seen, old)} />
      {#if pct}<span class="mono pct">{Math.round(seen)}%</span>{/if}
    {/if}
  </span>
{:else if seen !== undefined}
  <Meter {width} {title} segments={seenSegments(seen, old)} />
{/if}

<style>
  .cell {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .pct {
    min-width: 34px;
    font-size: var(--fs-mono);
  }
  .done {
    display: inline-flex;
    color: var(--ok);
  }
</style>
