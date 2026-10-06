<script lang="ts">
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import type { MenuItem } from "./menuItem";

  let {
    items,
    onselect,
    nested = false,
    onback,
  }: {
    items: MenuItem[];
    /** Called with the id of a chosen leaf item. */
    onselect: (id: string) => void;
    /** A submenu: placed beside its parent item and not focused on mount. */
    nested?: boolean;
    /** Left arrow in a submenu. */
    onback?: () => void;
  } = $props();

  let list: HTMLElement;
  let openId = $state<string>();
  const checks = $derived(items.some((i) => i.checked !== undefined));

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

<ul role="menu" class:nested class:scroll={!items.some((i) => i.children)} bind:this={list} {onkeydown}>
  {#each items as item, i (item.separator ? `-${i}` : item.id)}
    {#if item.separator}
      <li role="separator"></li>
    {:else}
      <li
        role="none"
        data-id={item.id}
        onpointerenter={() => (openId = item.children && item.enabled !== false ? item.id : undefined)}
      >
        <button
          class="flat"
          class:open={openId === item.id}
          role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
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
          <span class="label">{item.label}</span>
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
  .nested {
    position: absolute;
    left: calc(100% - 2px);
    top: -5px;
    z-index: 1;
    min-width: 180px;
    background: var(--bg-sheet);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: var(--shadow);
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
