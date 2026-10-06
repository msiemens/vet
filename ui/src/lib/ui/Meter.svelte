<script lang="ts">
  /** A 4px bar of stacked segments; `value` is a percentage, `color` any CSS colour (normally a token). */
  let { width, segments, title }: { width: number; segments: { value: number; color: string }[]; title?: string } =
    $props();
</script>

<span class="meter" style:width="{width}px" {title}>
  {#each segments as s}<span style:width="{s.value}%" style:background={s.color}></span>{/each}
</span>

<style>
  .meter {
    height: 4px;
    border-radius: 2px;
    background: var(--ln2);
    display: flex;
    overflow: hidden;
    flex: none;
  }
</style>
