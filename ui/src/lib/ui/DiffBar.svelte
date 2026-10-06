<script lang="ts">
  import Meter from "./Meter.svelte";

  /**
   * A diffstat bar: green and red parts in the ratio of added to removed lines (neutral when there
   * are none). `seen` and `old` are percentages of the changes reviewed; they cover the bar from the
   * left in blue and in the old-base colour, so a fully reviewed bar is blue.
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

  const segments = $derived.by(() => {
    const total = added + removed;
    const addEnd = total ? (added / total) * 100 : 0;
    const covered = Math.min(100, seen + old);
    return [
      { value: seen, color: "var(--seen)" },
      { value: old, color: "var(--old-base)" },
      { value: Math.max(0, addEnd - covered), color: "var(--add-text)" },
      { value: total ? Math.min(100 - addEnd, 100 - covered) : 0, color: "var(--del-text)" },
    ];
  });
</script>

<Meter {width} {segments} {title} />
