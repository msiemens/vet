<script lang="ts">
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { match } from "../speedSearch";
  import Marked from "./Marked.svelte";
  import Menu from "./Menu.svelte";
  import { type MenuItem, mnemonicTarget } from "./menuItem";
  import { SpeedSearch } from "./speedSearch.svelte";
  import SpeedSearchField from "./SpeedSearchField.svelte";

  let {
    items,
    onselect,
    nested = false,
    onback,
    speedSearch = false,
    mnemonics = false,
  }: {
    items: MenuItem[];
    /** Called with the id of a chosen leaf item. */
    onselect: (id: string) => void;
    /** Typing filters the top-level items by label. */
    speedSearch?: boolean;
    /** The items are the menus of a menu bar: the first letter of one opens it. */
    mnemonics?: boolean;
    /** A submenu: placed beside its parent item and not focused on mount. */
    nested?: boolean;
    /** Left arrow in a submenu. */
    onback?: () => void;
  } = $props();

  let list: HTMLElement;
  let openId = $state<string>();
  const checks = $derived(items.some((i) => i.checked !== undefined));
  const search = new SpeedSearch();
  const found = $derived(
    speedSearch && search.active
      ? items.flatMap((item) => {
          const m = item.separator ? undefined : match(search.query, item.label);
          return m ? [{ item, ranges: m.ranges }] : [];
        })
      : undefined,
  );
  const shown = $derived(found ? found.map((f) => f.item) : items);
  const marks = $derived(new Map(found?.map((f) => [f.item, f.ranges])));

  const buttons = () => [...list.querySelectorAll<HTMLButtonElement>(":scope > li > button:enabled")];

  onMount(() => {
    if (!nested) buttons()[0]?.focus();
  });

  async function open(item: MenuItem, focusFirst: boolean) {
    openId = item.id;
    if (!focusFirst) return;
    await tick();
    list.querySelector<HTMLElement>(`:scope > li[data-id="${CSS.escape(item.id)}"] ul button:enabled`)?.focus();
  }

  function onkeydown(e: KeyboardEvent) {
    // A submenu's keys are its own.
    if (speedSearch && !nested && (e.target as Element).closest("ul") === list && search.type(e)) {
      void tick().then(() => buttons()[0]?.focus());
      return;
    }
    if (mnemonics && !nested && !e.ctrlKey && !e.altKey && !e.metaKey && (e.target as Element).closest("ul") === list) {
      const menu = mnemonicTarget(items, e.key);
      if (menu) {
        e.preventDefault();
        e.stopPropagation();
        void open(menu, true);
        return;
      }
    }
    const all = buttons();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const move = (to: number) => {
      e.preventDefault();
      e.stopPropagation();
      all[(to + all.length) % all.length]?.focus();
    };
    if (e.key === "ArrowDown") move(at + 1);
    else if (e.key === "ArrowUp") move(at < 0 ? -1 : at - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(-1);
    else if (e.key === "ArrowRight") {
      const item = items.find(
        (i) => i.id === (document.activeElement as HTMLElement | null)?.closest("li")?.dataset.id,
      );
      if (item?.children) {
        e.preventDefault();
        e.stopPropagation();
        void open(item, true);
      }
    } else if (e.key === "ArrowLeft" && onback) {
      e.preventDefault();
      e.stopPropagation();
      onback();
    }
  }

  /** Closes the submenu and returns focus to its parent item. */
  function close(item: MenuItem) {
    openId = undefined;
    list.querySelector<HTMLElement>(`:scope > li[data-id="${CSS.escape(item.id)}"] > button`)?.focus();
  }
</script>

<ul
  role="menu"
  class:surface={nested}
  class:nested
  class:scroll={!items.some((i) => i.children)}
  bind:this={list}
  {onkeydown}
  {...search.attributes}
>
  {#if found}<li role="none"><SpeedSearchField query={search.query} found={found.length > 0} /></li>{/if}
  {#each shown as item, i (item.separator ? `-${i}` : item.id)}
    {#if item.separator}
      <li role="separator"></li>
    {:else if item.heading}
      <li role="presentation" class="heading">{item.label}</li>
    {:else}
      <li
        role="none"
        data-id={item.id}
        title={item.reason}
        onpointerenter={() => (openId = item.children && item.enabled !== false ? item.id : undefined)}
      >
        <button
          class="flat"
          class:open={openId === item.id}
          role={item.checked === undefined ? "menuitem" : item.radio ? "menuitemradio" : "menuitemcheckbox"}
          aria-checked={item.checked}
          aria-haspopup={item.children ? "menu" : undefined}
          aria-expanded={item.children ? openId === item.id : undefined}
          disabled={item.enabled === false}
          type="button"
          onclick={() => (item.children ? void open(item, false) : onselect(item.id))}
        >
          {#if checks}<span class="check"
              >{#if item.checked}<Icon name="check" size={12} stroke={2.2} />{/if}</span
            >{/if}
          <span class="label"><Marked text={item.label} ranges={marks.get(item)} /></span>
          {#if item.detail}<span class="mono detail">{item.detail}</span>{/if}
          {#if item.shortcut}<span class="detail">{item.shortcut}</span>{/if}
          {#if item.children}<span class="sub"><Icon name="closed" size={9} /></span>{/if}
        </button>
        {#if item.children && openId === item.id}<Menu
            items={item.children}
            {onselect}
            nested
            onback={() => close(item)}
          />{/if}
      </li>
    {/if}
  {/each}
</ul>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }
  /* A list with submenus cannot scroll: it would clip them. */
  .scroll {
    max-height: min(60vh, 420px);
    overflow: hidden auto;
  }
  li {
    position: relative;
  }
  li[role="separator"] {
    height: 1px;
    margin: 4px 0;
    background: var(--line-soft);
  }
  .heading {
    padding: 6px 12px 2px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    font-weight: 600;
  }
  .nested {
    position: absolute;
    left: calc(100% - 2px);
    top: -5px;
    z-index: 1;
    min-width: 180px;
  }
  button {
    width: 100%;
    height: var(--h-row);
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 12px;
    text-align: left;
    font-size: var(--fs-ctl);
    white-space: nowrap;
  }
  button.open {
    background: var(--hover);
  }
  .label {
    flex: 1;
  }
  .check {
    flex: none;
    width: 12px;
    margin-right: -6px;
    display: inline-flex;
    color: var(--accent);
  }
  .detail {
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .sub {
    display: inline-flex;
    color: var(--text-3);
  }
</style>
