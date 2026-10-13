<script lang="ts" generics="T">
  // A list of rows to pick from, as the switchers and the welcome screen draw them: a button per row
  // that is out of the tab order (the field or the list that owns the keys moves `cursor`), the
  // row's content from the owner. `isOn` marks the row that is the current choice.
  import type { Snippet } from "svelte";

  let {
    items,
    key,
    cursor = -1,
    isOn,
    label,
    empty,
    list = $bindable(),
    onpick,
    onmenu,
    row,
  }: {
    items: readonly T[];
    /** What tells a row from the others across a change of the list. */
    key: (item: T) => string;
    /** The index of the row the keys act on, -1 for none. */
    cursor?: number;
    isOn?: (item: T) => boolean;
    /** What assistive technology calls the list. */
    label: string;
    /** What the list says when it has no row. */
    empty: string;
    list?: HTMLElement;
    onpick: (item: T, index: number, e: MouseEvent) => void;
    /** A right click on a row, which the browser's own menu does not answer. */
    onmenu?: (item: T, index: number, e: MouseEvent) => void;
    row: Snippet<[item: T]>;
  } = $props();
</script>

<ul bind:this={list} role="listbox" aria-label={label}>
  {#each items as item, index (key(item))}
    <li role="option" aria-selected={index === cursor}>
      <button
        class="flat pick"
        class:on={isOn?.(item)}
        class:cur={index === cursor}
        type="button"
        tabindex="-1"
        onclick={(e) => onpick(item, index, e)}
        oncontextmenu={onmenu &&
          ((e) => {
            e.preventDefault();
            onmenu(item, index, e);
          })}
      >
        {@render row(item)}
      </button>
    </li>
  {:else}
    <li class="empty">{empty}</li>
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  button {
    display: block;
    width: 100%;
    box-sizing: border-box;
    border-radius: var(--radius-sheet);
    text-align: left;
  }
  .empty {
    padding: var(--pick-empty-pad, 8px);
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
</style>
