<script lang="ts">
  // The Structure tool window: every symbol of the file the active tab shows, the changed ones
  // marked as in the Changes tree. A click selects a row (and puts its symbol on show in the Symbol
  // tool), Enter, F4 and a double click jump to it; typing speed-searches.
  import { tick, untrack } from "svelte";
  import type { SymbolInfo } from "../../generated/SymbolInfo";
  import { arrowKey } from "../../lib/foldKeys";
  import { plainKey } from "../../lib/listKeys";
  import { basename } from "../../lib/paths";
  import { changeGate } from "../../lib/ui/changeGate";
  import { aimIndex, stepMatch } from "../../lib/speedSearch";
  import FileIcon from "../../lib/ui/FileIcon.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import { middleClickHides } from "../../lib/ui/middleClickHides";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import TextToggle from "../../lib/ui/TextToggle.svelte";
  import VirtualRows from "../../lib/ui/VirtualRows.svelte";
  import { opened } from "../../state/opened.svelte";
  import { type OutlineLoad, structure } from "../../state/structure.svelte";
  import { holderKeys, structureRows, type StructureRow } from "../../state/structureRows";
  import { keyOf } from "../../state/symbolKey";
  import StructureList from "./StructureList.svelte";
  import StructureNotice from "./StructureNotice.svelte";

  let {
    onjump,
    oneditor,
    onhide,
  }: {
    /** Shows the symbol in the file's tab, at the base side's text for a removed one. */
    onjump: (symbol: SymbolInfo, removed: boolean) => void;
    /** Gives the keyboard focus back to the editor. */
    oneditor: () => void;
    /** Hides the tool window. */
    onhide: () => void;
  } = $props();

  const search = new SpeedSearch();
  let cursor = $state(-1);
  let body = $state<HTMLElement>();
  let list = $state<VirtualRows>();

  const LOADING: OutlineLoad = { state: "loading" };
  const NONE: ReadonlySet<string> = new Set();
  /** The outline and selection of the changeset on view; none while another loads. */
  const held = $derived(opened.session?.structure);
  const target = $derived(held?.target);
  const load = $derived(held?.load ?? LOADING);
  const closed = $derived(held?.closed ?? NONE);
  const outline = $derived(load.state === "done" ? load.outline : undefined);
  const symbols = $derived(outline?.symbols ?? []);
  const changed = $derived(symbols.filter((s) => s.change !== null).length);
  /** The view marks changes, and this file has some. */
  const marked = $derived(changed > 0);
  const rows = $derived(
    outline && !outline.unavailable
      ? structureRows(symbols, {
          sort: structure.sort,
          changedOnly: structure.changedOnly && marked,
          query: search.query,
          closed,
        })
      : [],
  );
  const fileName = $derived(target && basename(target.path));
  const allShut = $derived(closed.size > 0);

  // A search belongs to the file it was typed in.
  const fileChanged = changeGate();
  $effect(() => {
    if (fileChanged(target)) {
      untrack(() => {
        search.clear();
        cursor = -1;
      });
    }
  });

  // The selection moved (a reveal): its row comes into view.
  const revealed = changeGate();
  $effect(() => {
    const id = held?.selected;
    if (id !== undefined && revealed(id, rows)) {
      untrack(() => void tick().then(() => void list?.reveal(rows.findIndex((r) => keyOf(r.symbol.id) === id))));
    }
  });

  const pick = (row: StructureRow) => held?.select(row.symbol, row.change);
  const jump = (row: StructureRow) => onjump(row.symbol, row.change === "removed");

  const nav = new ListNav<StructureRow>({
    element: () => body,
    items: () => rows,
    single: pick,
    double: jump,
    highlight: { get: () => cursor, set: (i) => (cursor = i) },
    reveal: (i) => void list?.reveal(i),
    start: () => rows.findIndex((r) => keyOf(r.symbol.id) === held?.selected),
  });
  $effect(() => () => nav.cancel());

  function click(e: MouseEvent, index: number, chevron: boolean) {
    const row = rows.at(index);
    if (chevron && row?.holds && !search.active) {
      cursor = index;
      body?.focus({ preventScroll: true });
      held?.toggleFold(row.key);
      return;
    }
    nav.click(e, index);
  }

  function foldAll() {
    if (!held) return;
    if (allShut) held.closed.clear();
    else for (const key of holderKeys(symbols)) held.closed.add(key);
  }

  /** The indices of the rows the search matched. */
  const matched = () => rows.flatMap((r, i) => (r.ranges ? [i] : []));

  /** The row Enter acts on while searching: the best match of the query. */
  function aim() {
    cursor = aimIndex(search.query, matched(), (i) => rows[i].name);
    list?.reveal(cursor);
  }

  /** Up and Down while searching go from match to match. */
  function step(by: number) {
    const to = stepMatch(matched(), cursor, by);
    if (to === undefined) return;
    cursor = to;
    list?.reveal(cursor);
  }

  function onkeydown(e: KeyboardEvent) {
    const take = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    if (search.active && (e.key === "ArrowDown" || e.key === "ArrowUp") && plainKey(e)) {
      take();
      step(e.key === "ArrowDown" ? 1 : -1);
      return;
    }
    const row = rows.at(cursor);
    if (!search.active && row && plainKey(e)) {
      if (e.key === "F4") {
        take();
        jump(row);
        return;
      }
      if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        const fold = arrowKey(rows, cursor, e.key === "ArrowLeft" ? "left" : "right", {
          holds: (r) => r.holds,
          open: (r) => r.open,
          leaves: () => true,
        });
        if (fold) {
          take();
          if ("move" in fold) {
            cursor = fold.move;
            list?.reveal(cursor);
          } else held?.toggleFold(fold.toggle.key);
          return;
        }
      }
    }
    if (e.key === "F4" && plainKey(e)) {
      take();
      return;
    }
    if (e.key === "Escape" && !search.active && plainKey(e)) {
      take();
      oneditor();
      return;
    }
    if (search.type(e)) {
      if (search.active) aim();
      else cursor = -1;
      return;
    }
    if (nav.keydown(e)) take();
  }
</script>

<aside aria-label="Structure" onfocusin={() => nav.focusin()}>
  <div class="sh sh-bar" use:middleClickHides={onhide}>
    <span>Structure</span>
    {#if outline && !outline.unavailable}<span class="mono n">{symbols.length}</span>{/if}
    <span class="grow"></span>
    <TextToggle
      on={structure.sort}
      title="Sort alphabetically"
      disabled={!rows.length}
      onclick={() => structure.setSort(!structure.sort)}>A–Z</TextToggle
    >
    <IconButton
      icon="filter"
      label={marked ? "Changed only" : "Changed only (nothing is changed in this file)"}
      active={structure.changedOnly && marked}
      disabled={!marked}
      size="s"
      iconSize={14}
      onclick={() => structure.setChangedOnly(!structure.changedOnly)}
    />
    <IconButton
      icon="locate"
      label="Reveal Symbol in Structure"
      disabled={!outline || !!outline.unavailable}
      size="s"
      iconSize={13}
      onclick={() => held?.revealCaret()}
    />
    <IconButton
      icon={allShut ? "expandAll" : "collapseAll"}
      label={allShut ? "Expand all" : "Collapse all"}
      disabled={!rows.length || search.active}
      size="s"
      iconSize={14}
      onclick={foldAll}
    />
  </div>
  {#if target}
    <div class="strip">
      {#if target.commit}
        <span class="commit"><Icon name="commit" size={13} stroke={1.8} /></span>
      {:else}
        <FileIcon name={fileName ?? ""} />
      {/if}
      <span class="mono path" title={target.path}>{target.path}</span>
      {#if target.commit}<span class="mono at">at {target.commit.slice(0, 7)}</span>{/if}
      {#if outline && !outline.unavailable && target.shown.view.type === "changeset"}
        <span class="sum">{marked ? `${changed} of ${symbols.length} changed` : "not changed"}</span>
      {/if}
    </div>
  {/if}
  <!-- The list takes the keys of its rows, which are not in the tab order. -->
  <div class="body" role="presentation" bind:this={body} {onkeydown} {...nav.attributes} {...search.attributes}>
    {#if search.active}<SpeedSearchField query={search.query} found={rows.length > 0} />{/if}
    {#if rows.length}
      <StructureList bind:list {rows} selected={held?.selected} {cursor} onclick={click} />
    {:else}
      <StructureNotice {load} file={fileName} matches={rows.length} />
    {/if}
  </div>
</aside>

<style>
  aside {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .strip {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: var(--h-row);
    padding: 2px 10px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .commit {
    flex: none;
    display: inline-flex;
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-mono-s);
    color: var(--text-2);
  }
  .at {
    flex: none;
    font-size: var(--fs-mono-s);
  }
  .sum {
    margin-left: auto;
    flex: none;
    white-space: nowrap;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
  }
</style>
