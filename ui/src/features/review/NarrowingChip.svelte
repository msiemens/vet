<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";

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

<span class="chip" title="This review is narrowed; the project's review filter is unchanged">
  <span class="text">{label}</span>
  <button class="flat reset" type="button" aria-label="Reset to the project filter" onclick={onreset}>
    <Icon name="close" size={10} stroke={2} />
  </button>
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    height: 20px;
    padding: 0 2px 0 7px;
    border: 1px solid color-mix(in srgb, var(--accent) 38%, transparent);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    font-size: var(--fs-ctl);
    font-weight: 400;
  }
  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .reset {
    flex: none;
    width: 16px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    color: inherit;
  }
</style>
