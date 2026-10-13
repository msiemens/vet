<script module lang="ts">
  /** An action under the rows. */
  export interface SwitcherAction {
    label: string;
    /** The shortcut it shows at its end. */
    key?: string;
    title?: string;
    disabled?: boolean;
    run: () => void;
  }
</script>

<script lang="ts" generics="T">
  // The popover of a header switcher: a search field (with what filters beside it), the rows to pick
  // from and, under them, actions. The field keeps the focus and moves the highlight over the rows and
  // then the actions; Enter chooses what it is on, and the popover closes first.
  import type { Snippet } from "svelte";
  import { ListNav, revealCurrent } from "./listNav";
  import PickList from "./PickList.svelte";
  import Popover from "./Popover.svelte";
  import SearchField from "./SearchField.svelte";

  let {
    open = $bindable(false),
    label,
    query = $bindable(""),
    items,
    key,
    isOn,
    listLabel,
    empty,
    actions = [],
    trigger,
    filters,
    row,
    onchoose,
    onmenu,
  }: {
    open?: boolean;
    /** What assistive technology calls the search field. */
    label: string;
    /** The search; each opening starts with none. */
    query?: string;
    /** The rows, already narrowed by the query. */
    items: readonly T[];
    key: (item: T) => string;
    /** The row is the current choice. */
    isOn?: (item: T) => boolean;
    listLabel: string;
    /** What the list says when it has no row. */
    empty: string;
    actions?: readonly SwitcherAction[];
    trigger: Snippet<[toggle: () => void]>;
    /** Beside the search field; `reset` puts the highlight back on the first row. */
    filters?: Snippet<[reset: () => void]>;
    row: Snippet<[item: T]>;
    onchoose: (item: T) => void;
    /** A right click on a row. */
    onmenu?: (item: T, e: MouseEvent) => void;
  } = $props();

  let cursor = $state(0);
  let scroller = $state<HTMLElement>();
  let input = $state<HTMLInputElement>();

  // The field takes the focus from the trigger that opened the popover, which `autofocus` leaves where it is.
  $effect(() => input?.focus());

  const stops = $derived(items.length + actions.length);
  const current = $derived(Math.min(cursor, stops - 1));

  // Each opening starts with the whole list.
  $effect(() => {
    if (open) [query, cursor] = ["", 0];
  });

  // The highlight is an index over the rows and then the actions.
  const nav = new ListNav<number>({
    items: () => Array.from({ length: stops }, (_, i) => i),
    single: (index) => {
      open = false;
      if (index < items.length) onchoose(items[index]);
      else {
        const action = actions[index - items.length];
        if (!action.disabled) action.run();
      }
    },
    highlight: { get: () => current, set: (i) => (cursor = i) },
    reveal: () => void revealCurrent(scroller),
    field: true,
  });

  function onkeydown(e: KeyboardEvent) {
    if (nav.keydown(e)) e.preventDefault();
  }
</script>

<Popover bind:open {trigger}>
  <div class="switcher">
    <div class="top">
      <SearchField bind:input bind:value={query} {label} {onkeydown} oninput={() => (cursor = 0)} />
      {@render filters?.(() => (cursor = 0))}
    </div>
    <div class="scroll" bind:this={scroller}>
      <PickList
        {items}
        {key}
        {isOn}
        {empty}
        label={listLabel}
        cursor={current}
        onpick={(_, index, e) => nav.click(e, index)}
        onmenu={onmenu &&
          ((item, index, e) => {
            cursor = index;
            onmenu(item, e);
          })}
        {row}
      />
    </div>
    {#if actions.length}
      <div class="foot">
        {#each actions as action, i (action.label)}
          <button
            class="flat pick"
            class:cur={items.length + i === current}
            type="button"
            tabindex="-1"
            title={action.title}
            disabled={action.disabled}
            onclick={(e) => nav.click(e, items.length + i)}
          >
            {action.label}
            {#if action.key}<span class="key">{action.key}</span>{/if}
          </button>
        {/each}
      </div>
    {/if}
  </div>
</Popover>

<style>
  .switcher {
    width: var(--w-switcher);
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border-bottom: 1px solid var(--line-soft);
  }
  .scroll {
    padding: 4px;
    max-height: min(60vh, 420px);
    overflow: hidden auto;
  }
  .foot {
    display: flex;
    padding: 4px;
    border-top: 1px solid var(--line-soft);
  }
  .foot button {
    flex: 1;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: var(--h-row);
    padding: 0 8px;
    border-radius: var(--radius-sheet);
    font-size: var(--fs-ctl);
  }
  .key {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
</style>
