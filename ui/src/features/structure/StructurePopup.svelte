<script lang="ts">
  // The structure of the active file in a popup over the editor: the same list as the window, with
  // a filter field on top. A click or Enter jumps to the symbol and closes it; Escape closes it.
  import { onMount, tick } from "svelte";
  import type { SymbolInfo } from "../../generated/SymbolInfo";
  import { LIST } from "../../lib/listKeys";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import { structure } from "../../state/structure.svelte";
  import { structureRows, type StructureRow } from "../../state/structureRows";
  import { keyOf } from "../../state/symbolKey";
  import StructureList from "./StructureList.svelte";
  import StructureNotice from "./StructureNotice.svelte";

  let {
    onjump,
    oneditor,
    onservers,
  }: {
    /** Shows the symbol in the file's tab, at the base side's text for a removed one. */
    onjump: (symbol: SymbolInfo, removed: boolean) => void;
    /** Gives the keyboard focus back to the editor. */
    oneditor: () => void;
    /** Opens Settings at the language servers. */
    onservers: () => void;
  } = $props();

  /** How far the Page keys move. */
  const PAGE = 8;

  let query = $state("");
  let cursor = $state(0);
  let root = $state<HTMLElement>();
  let input = $state<HTMLInputElement>();
  let list = $state<VirtualRows>();

  const target = $derived(structure.target);
  const outline = $derived(structure.load.state === "done" ? structure.load.outline : undefined);
  const symbols = $derived(outline?.symbols ?? []);
  const marked = $derived(symbols.some((s) => s.change !== null));
  const rows = $derived(
    outline && !outline.unavailable
      ? structureRows(symbols, {
          sort: structure.sort,
          changedOnly: structure.changedOnly && marked,
          query,
          closed: new Set(),
        })
      : [],
  );
  const fileName = $derived(target?.path.split("/").pop());

  // The popup opens on the symbol the cursor is in; what is typed or toggled starts the list over.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the reads make them dependencies
    [query, structure.sort, structure.changedOnly];
    cursor = Math.max(0, query ? rows.findIndex((r) => !r.muted) : rows.findIndex(selected));
  });

  const selected = (r: StructureRow) => keyOf(r.symbol.id) === structure.selected;

  onMount(() => input?.focus());

  function close() {
    structure.popup = false;
  }

  function jump(row: StructureRow | undefined) {
    if (!row) return;
    close();
    onjump(row.symbol, row.change === "removed");
  }

  function servers() {
    close();
    onservers();
  }

  async function move(to: number) {
    cursor = Math.max(0, Math.min(rows.length - 1, to));
    await tick();
    list?.reveal(cursor);
  }

  // A list that scrolls under a pointer at rest makes the browser repeat the pointer's last move
  // with no movement; only a pointer that moved takes the highlight.
  function hover(e: MouseEvent, at: number) {
    if (e.movementX !== 0 || e.movementY !== 0) cursor = at;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
    const take = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    switch (e.key) {
      case "Escape":
        take();
        close();
        oneditor();
        break;
      case "ArrowDown":
        take();
        void move(cursor + 1);
        break;
      case "ArrowUp":
        take();
        void move(cursor - 1);
        break;
      case "PageDown":
        take();
        void move(cursor + PAGE);
        break;
      case "PageUp":
        take();
        void move(cursor - PAGE);
        break;
      case "Enter":
      case "F4":
        take();
        jump(rows.at(cursor));
        break;
    }
  }
</script>

<svelte:window onpointerdown={(e) => root && !root.contains(e.target as Node) && close()} />

<div
  class="surface pop"
  role="dialog"
  aria-label="Structure of {fileName ?? 'the file'}"
  tabindex="-1"
  bind:this={root}
  {onkeydown}
  {...{ [LIST]: "" }}
>
  <div class="head">
    <span class="title">Structure of <code>{fileName}</code></span>
    {#if outline && !outline.unavailable}<span class="mono n">{symbols.length}</span>{/if}
    <span class="grow"></span>
    <button
      class="flat txtbtn mono"
      type="button"
      aria-pressed={structure.sort}
      title="Sort alphabetically"
      onclick={() => structure.setSort(!structure.sort)}>A–Z</button
    >
    <IconButton
      icon="filter"
      label={marked ? "Changed only" : "Changed only (nothing is changed in this file)"}
      active={structure.changedOnly && marked}
      disabled={!marked}
      size={22}
      iconSize={14}
      onclick={() => structure.setChangedOnly(!structure.changedOnly)}
    />
  </div>
  <div class="search">
    <Icon name="search" size={12} />
    <input
      bind:this={input}
      bind:value={query}
      type="text"
      placeholder="Filter symbols"
      autocomplete="off"
      spellcheck="false"
      aria-label="Filter symbols"
    />
  </div>
  <div class="list">
    {#if rows.length}
      <StructureList
        bind:list
        {rows}
        selected={undefined}
        {cursor}
        onclick={(_, i) => jump(rows.at(i))}
        onhover={hover}
      />
    {:else}
      <StructureNotice load={structure.load} file={fileName} matches={rows.length} onsettings={servers} />
    {/if}
  </div>
  <div class="foot">
    <span><kbd>↑</kbd><kbd>↓</kbd> move</span>
    <span><kbd>Enter</kbd> jump</span>
    <span><kbd>Esc</kbd> close</span>
  </div>
</div>

<style>
  .pop {
    position: absolute;
    z-index: 20;
    top: 44px;
    left: 50%;
    transform: translateX(-50%);
    width: 420px;
    max-width: calc(100% - 16px);
    max-height: 440px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    outline: none;
    background: var(--bg-editor);
    font-size: var(--fs-ctl);
    color: var(--text);
  }
  .head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    box-sizing: border-box;
    height: 33px;
    padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--line-soft);
  }
  .title code {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .n {
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .txtbtn {
    height: 22px;
    flex: none;
    padding: 0 6px;
    border-radius: var(--radius);
    color: var(--text-2);
    font-size: 10.5px;
    font-weight: 500;
    line-height: 1;
  }
  .search {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-3);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font: var(--fs-mono-s) var(--font-mono);
  }
  .list {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 12px;
    border-top: 1px solid var(--line-soft);
    color: var(--text-3);
    font-size: 11.5px;
  }
  .foot span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  kbd {
    font: 11px var(--font-mono);
    padding: 1px 5px;
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 4px;
    background: var(--bg-control);
    color: var(--text);
  }
</style>
