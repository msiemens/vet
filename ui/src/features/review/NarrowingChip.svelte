<script lang="ts">
  import TagChip from "../../lib/ui/TagChip.svelte";

  let {
    label,
    onreset,
  }: {
    /** What the review is narrowed to, as the chip words it. */
    label: string;
    /** Ends the narrowing. */
    onreset: () => void;
  } = $props();
</script>

<TagChip
  title="This review is narrowed; the project's review filter is unchanged"
  removeLabel="Reset to the project filter"
  onremove={onreset}
>
  <span class="text">{label}</span>
</TagChip>

<style>
  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
