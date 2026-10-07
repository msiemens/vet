<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import Meter from "../../lib/ui/Meter.svelte";
  import type { Reviewed } from "./queueRows";

  let { reviewed, width = 72 }: { reviewed: Reviewed; width?: number } = $props();
</script>

{#if reviewed.kind === "none"}
  <span class="none" title="Not started">—</span>
{:else if reviewed.kind === "started"}
  <span class="none">in progress</span>
{:else if reviewed.kind === "progress"}
  <span class="cell">
    <Meter
      {width}
      segments={[
        { value: reviewed.seen, color: "var(--seen)" },
        { value: reviewed.oldBase, color: "var(--old-base)" },
      ]}
    />
    <span class="mono pct">{Math.round(reviewed.seen + reviewed.oldBase)}%</span>
  </span>
{:else}
  <span class="cell" title={reviewed.percent === undefined ? "Finished with changes unseen" : "Finished"}>
    <span class="done"><Icon name="check" size={12} stroke={2.4} /></span>
    {#if reviewed.percent !== undefined}
      <Meter {width} segments={[{ value: reviewed.percent, color: "var(--seen)" }]} />
      <span class="mono pct">{reviewed.percent}%</span>
    {/if}
  </span>
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
  .none {
    color: var(--text-3);
  }
</style>
