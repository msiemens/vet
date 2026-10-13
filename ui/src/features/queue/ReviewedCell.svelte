<script lang="ts">
  import SeenMeter from "../../lib/ui/SeenMeter.svelte";
  import type { Reviewed } from "./queueRows";

  let { reviewed, width = 56 }: { reviewed: Reviewed; width?: number } = $props();
</script>

{#if reviewed.kind === "none"}
  <span class="none" title="Not started">—</span>
{:else if reviewed.kind === "started"}
  <span class="none">in progress</span>
{:else if reviewed.kind === "progress"}
  <SeenMeter {width} seen={reviewed.seen} old={reviewed.oldBase} pct />
{:else}
  <SeenMeter
    {width}
    done
    pct
    seen={reviewed.percent}
    title={reviewed.percent === undefined ? "Finished with changes unseen" : "Finished"}
  />
{/if}

<style>
  .none {
    color: var(--text-3);
  }
</style>
