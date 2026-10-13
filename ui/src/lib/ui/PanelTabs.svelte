<script lang="ts" generics="T extends string">
  let {
    tabs,
    value,
    onchange,
    label,
  }: {
    /** `count` is shown after the label, muted. */
    tabs: { id: T; label: string; count?: number }[];
    value: T;
    onchange: (id: T) => void;
    /** Names the group for assistive technology. */
    label: string;
  } = $props();
</script>

<!-- Underlined text on the panel's own background: the tabs of a panel, as against the editors' tab strip. -->
<div class="tabs" role="tablist" aria-label={label}>
  {#each tabs as tab (tab.id)}
    <button
      class="flat tab"
      class:selected={tab.id === value}
      type="button"
      role="tab"
      aria-selected={tab.id === value}
      onclick={() => onchange(tab.id)}
    >
      {tab.label}{#if tab.count !== undefined}<span class="n">{tab.count}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .tabs {
    display: flex;
    align-items: stretch;
    gap: 4px;
    height: var(--h-panel-tabs);
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 10px;
    border-radius: 0;
    border-bottom: 2px solid transparent;
    color: var(--text-2);
    font-size: var(--fs-ctl);
    box-sizing: border-box;
  }
  .tab.selected {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
</style>
