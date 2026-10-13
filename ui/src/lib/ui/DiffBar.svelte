<script lang="ts">
  import Meter from "./Meter.svelte";
  import { diffSegments } from "./seenMeter";

  /**
   * A diffstat bar: green and red parts in the ratio of added to removed lines (neutral when there
   * are none). `seen` and `old` are percentages of the changes reviewed; they cover the bar from the
   * left in blue and in the old-base stripes, so a fully reviewed bar is blue.
   */
  let {
    width,
    added,
    removed,
    seen,
    old,
    title,
  }: {
    width: number;
    added: number;
    removed: number;
    seen: number;
    old: number;
    title?: string;
  } = $props();
</script>

<Meter {width} segments={diffSegments(added, removed, seen, old)} {title} />
