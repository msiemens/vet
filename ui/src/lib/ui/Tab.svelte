<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let {
    selected,
    title,
    onselect,
    onclose,
    closeLabel = "Close tab",
    onmenu,
    children,
  }: {
    selected: boolean;
    title?: string;
    onselect: () => void;
    /** Absent for a tab that cannot be closed. */
    onclose?: () => void;
    /** What the close button is called, and its tooltip. */
    closeLabel?: string;
    /** Context menu on the tab, at the pointer position. */
    onmenu?: (x: number, y: number) => void;
    children: Snippet;
  } = $props();
</script>

<div
  class="tab"
  class:selected
  role="presentation"
  oncontextmenu={(e) => {
    if (!onmenu) return;
    e.preventDefault();
    onmenu(e.clientX, e.clientY);
  }}
  onmousedown={(e) => {
    // Desktop tabs select on press; the middle button is reserved for closing.
    if (e.button === 0 && !(e.target instanceof Element && e.target.closest(".close"))) onselect();
    if (e.button === 1) e.preventDefault();
  }}
  onauxclick={(e) => e.button === 1 && onclose?.()}
>
  <button class="label" type="button" role="tab" aria-selected={selected} {title} onclick={onselect}>
    {@render children()}
  </button>
  {#if onclose}
    <button class="flat close" type="button" aria-label={closeLabel} title={closeLabel} tabindex="-1" onclick={onclose}>
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
    border-right: 1px solid var(--line-soft);
    border-bottom: 2px solid transparent;
    color: var(--text-2);
  }
  .tab:not(.selected):hover {
    background: var(--hover);
  }
  .tab.selected {
    background: var(--bg-editor);
    border-bottom-color: var(--accent);
    color: var(--text);
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
    opacity: 0;
  }
  .tab:hover .close,
  .tab.selected .close {
    opacity: 1;
  }
</style>
