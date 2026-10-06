<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    selected,
    title,
    onselect,
    onclose,
    children,
  }: {
    selected: boolean;
    title?: string;
    onselect: () => void;
    /** Absent for a tab that cannot be closed. */
    onclose?: () => void;
    children: Snippet;
  } = $props();
</script>

<div
  class="tab"
  class:selected
  role="presentation"
  onmousedown={(e) => e.button === 1 && e.preventDefault()}
  onauxclick={(e) => e.button === 1 && onclose?.()}
>
  <button class="label" type="button" role="tab" aria-selected={selected} {title} onclick={onselect}>
    {@render children()}
  </button>
  {#if onclose}
    <button class="flat close" type="button" aria-label="Close tab" title="Close" tabindex="-1" onclick={onclose}>
      <Icon name="close" size={10} stroke={2} />
    </button>
  {/if}
</div>

<style>
  .tab {
    display: inline-flex;
    align-items: center;
    flex: none;
    height: var(--h-tab);
    box-sizing: border-box;
    border-right: 1px solid var(--ln2);
    border-bottom: 2px solid transparent;
    color: var(--tx2);
  }
  .tab:hover {
    background: var(--hov);
  }
  .tab:active {
    background: var(--press);
  }
  .tab.selected {
    background: var(--ed);
    border-bottom-color: var(--acc);
    color: var(--tx);
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 100%;
    padding: 0 6px 0 12px;
    border: 0;
    background: transparent;
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
  .tab:not(:has(.close)) .label {
    padding-right: 12px;
  }
  .close {
    width: 18px;
    height: 18px;
    margin-right: 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    visibility: hidden;
  }
  .tab:hover .close,
  .tab.selected .close {
    visibility: visible;
  }
</style>
