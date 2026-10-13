<script lang="ts">
  // The places Show Usages or Go to Implementation found, under the caret: usages grouped by file,
  // implementations flat, two lines each. Arrow keys, Page keys, Home and End move,
  // Enter opens, Escape closes, and typing filters.
  import { onDestroy, onMount, tick } from "svelte";
  import type { LocationRow } from "../../generated/LocationRow";
  import type { StatusLetter } from "../../lib/changeLetter";
  import { codeRuns, libraryLabel } from "../../lib/foundText";
  import { LIST } from "../../lib/listKeys";
  import { fullName } from "../../lib/symbolLabel";
  import ChangeLetter from "../../lib/ui/ChangeLetter.svelte";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import GroupHeader from "../../lib/ui/GroupHeader.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import OpenInFindButton from "../../lib/ui/OpenInFindButton.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import QualifiedName from "../../lib/ui/QualifiedName.svelte";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import StatusDot from "../../lib/ui/StatusDot.svelte";
  import { isMac } from "../../state/host";
  import type { PickerView } from "../../state/pickerTypes";
  import { countText, groupRows, matchedRow, opensBelow, symbolLabel, visibleRows } from "./pickerRows";

  let {
    view,
    statusOf,
    onsettings,
  }: {
    view: PickerView;
    /** The letter of a changed file, none for any other. */
    statusOf: (path: string) => StatusLetter | undefined;
    /** Opens Settings at the language servers. */
    onsettings: () => void;
  } = $props();

  /** The most the popup grows to, and how wide it is with the places grouped, listed flat, or without any. */
  const MAX_HEIGHT = 520;
  const WIDTH = { grouped: 720, flat: 560, none: 520 };
  /** How far the Page keys move. */
  const PAGE = 8;

  const search = new SpeedSearch();
  let onlyChangeset = $state(false);
  let cursor = $state(0);
  let root = $state<HTMLElement>();
  let list = $state<HTMLElement>();
  let picked = false;
  let before: Element | null = null;

  const nouns = $derived(view.kind === "usages" ? "usages" : "implementations");
  const results = $derived(view.body.phase === "results" ? view.body : undefined);
  const flat = $derived(view.kind === "implementations");
  const shown = $derived(results ? visibleRows(results.rows, search.query, onlyChangeset) : []);
  const items = $derived(flat ? [] : groupRows(shown, statusOf));
  const hits = $derived(flat ? shown : items.flatMap((i) => (i.kind === "hit" ? [i.row] : [])));
  const count = $derived(results && countText(results.total, results.rows.length));

  // What was typed or toggled starts the list over.
  $effect(() => {
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the reads make them dependencies
    (search.query, onlyChangeset);
    cursor = 0;
  });

  // The side comes from the caret alone, so the popup never crosses it while open: not from
  // "Searching…" to the results, not while typing or toggling the changeset.
  const down = $derived(opensBelow(view.anchor, window.innerHeight, MAX_HEIGHT));

  const geometry = $derived.by(() => {
    const { anchor } = view;
    const below = window.innerHeight - anchor.bottom - 12;
    const above = anchor.top - 12;
    const width = Math.min(WIDTH[!results?.rows.length ? "none" : flat ? "flat" : "grouped"], window.innerWidth - 16);
    return {
      down,
      maxHeight: Math.min(MAX_HEIGHT, down ? below : above),
      width,
      left: Math.max(8, Math.min(anchor.left, window.innerWidth - width - 8)),
      top: anchor.bottom + 4,
      bottom: window.innerHeight - anchor.top + 4,
    };
  });

  onMount(() => {
    before = document.activeElement;
    root?.focus();
  });
  // A popup closed without a pick gives the focus back to the editor it came from.
  onDestroy(() => {
    if (!picked && before instanceof HTMLElement && before.isConnected) before.focus({ preventScroll: true });
  });

  function pick(row: LocationRow) {
    picked = true;
    view.onpick(row);
  }

  async function reveal() {
    await tick();
    list?.querySelector(`[data-at="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }

  // A list that scrolls under a pointer at rest makes the browser repeat the pointer's last move
  // with no movement; only a pointer that moved takes the highlight.
  function hover(e: MouseEvent, at: number) {
    if (e.movementX !== 0 || e.movementY !== 0) cursor = at;
  }

  function keep() {
    picked = true;
    view.onkeep?.();
  }

  function onkeydown(e: KeyboardEvent) {
    if (
      e.key === "Enter" &&
      (e.metaKey || e.ctrlKey) &&
      !e.altKey &&
      !e.shiftKey &&
      view.onkeep &&
      results?.rows.length
    ) {
      e.preventDefault();
      e.stopPropagation();
      keep();
      return;
    }
    if (search.type(e)) return;
    if (e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
    const take = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    const move = (to: number) => {
      take();
      cursor = Math.max(0, Math.min(hits.length - 1, to));
      void reveal();
    };
    switch (e.key) {
      case "Escape":
        take();
        view.onclose();
        break;
      case "ArrowDown":
        move(cursor + 1);
        break;
      case "ArrowUp":
        move(cursor - 1);
        break;
      case "PageDown":
        move(cursor + PAGE);
        break;
      case "PageUp":
        move(cursor - PAGE);
        break;
      case "Home":
        move(0);
        break;
      case "End":
        move(hits.length - 1);
        break;
      case "Enter": {
        take();
        const row = hits.at(cursor);
        if (row) pick(row);
        break;
      }
    }
  }
</script>

<svelte:window
  onpointerdown={(e) => root && !root.contains(e.target as Node) && view.onclose()}
  onresize={() => view.onclose()}
/>

{#snippet code(row: LocationRow)}
  {#each codeRuns(view.kind === "implementations" ? matchedRow(row, view.symbol) : row) as run, i (i)}{#if run.hit}<mark
        class={run.className}>{run.text}</mark
      >{:else if run.className}<span class={run.className}>{run.text}</span>{:else}{run.text}{/if}{/each}
{/snippet}

{#snippet symbol(row: LocationRow)}
  <QualifiedName name={symbolLabel(row)} title={row.within ? fullName(row.within.id) : row.path} />
{/snippet}

{#snippet status(path: string)}
  {@const letter = statusOf(path)}
  {#if letter}<ChangeLetter {letter} />{/if}
{/snippet}

<div
  class="surface picker"
  class:above={!geometry.down}
  role="dialog"
  aria-label={view.kind === "usages" ? `Usages of ${view.symbol}` : `Implementations of ${view.symbol}`}
  tabindex="-1"
  bind:this={root}
  {onkeydown}
  {...{ [LIST]: "" }}
  {...search.attributes}
  style:top={geometry.down ? `${geometry.top}px` : undefined}
  style:bottom={geometry.down ? undefined : `${geometry.bottom}px`}
  style:left="{geometry.left}px"
  style:width="{geometry.width}px"
  style:max-height="{geometry.maxHeight}px"
>
  <div class="head">
    <span class="title">{view.kind === "usages" ? "Usages of" : "Implementations of"} <code>{view.symbol}</code></span>
    {#if count}
      <span class="n">{count.count}</span>
      {#if count.cut}<span class="n">{count.cut}</span>{/if}
    {/if}
    <span class="grow"></span>
    {#if view.onkeep && results?.rows.length}
      <OpenInFindButton keys="{isMac ? '⌘' : 'Ctrl+'}↵" title="Keep this list in the Find window" onclick={keep} />
    {/if}
    {#if view.body.phase !== "stopped"}
      <IconButton
        icon="filter"
        label="Only Changeset Hits"
        size={24}
        iconSize={14}
        active={onlyChangeset}
        onclick={() => (onlyChangeset = !onlyChangeset)}
      />
    {/if}
  </div>
  {#if search.active}<SpeedSearchField query={search.query} found={hits.length > 0} />{/if}

  {#if view.body.phase === "searching"}
    <EmptyState busy title="Searching…" />
  {:else if view.body.phase === "stopped"}
    <EmptyState icon="warning" tone="warn" text={view.body.message}>
      {#snippet actions()}
        <button class="ctl" type="button" onclick={() => view.onretry()}>Restart</button>
        <button
          class="ctl"
          type="button"
          onclick={() => {
            view.onclose();
            onsettings();
          }}>Settings…</button
        >
      {/snippet}
    </EmptyState>
  {:else if !hits.length}
    <EmptyState title={results?.rows.length ? "No matches" : `No ${nouns}`} />
  {:else}
    <div class="list" bind:this={list}>
      {#if flat}
        {#each shown as row, at (`${row.path}:${row.line}:${row.col}`)}
          <button
            class="pick two"
            class:cur={at === cursor}
            data-at={at}
            type="button"
            tabindex="-1"
            onclick={() => pick(row)}
            onmousemove={(e) => hover(e, at)}
          >
            <StatusDot tone={row.change === "hunk" ? "ok" : "none"} />
            <span class="rows">
              <span class="first">
                <span class="sym">{@render symbol(row)}</span>
                <span class="where">
                  {#if row.external}
                    {@const lib = libraryLabel(row.path)}
                    <Icon name="module" size={12} />
                    {#if lib.package}<span class="pkg">{lib.package}</span><span class="sep">›</span>{/if}
                    <PathText path={lib.rest} title={row.path} />
                  {:else}
                    <PathText path={row.path} />
                  {/if}
                  <span class="mono at">:{row.line}</span>
                  {#if !row.external}{@render status(row.path)}{/if}
                </span>
              </span>
              <span class="mono code">{@render code(row)}</span>
            </span>
          </button>
        {/each}
      {:else}
        {#each items as item, i (i)}
          {#if item.kind === "section"}
            <GroupHeader kind="label">{item.label}</GroupHeader>
          {:else if item.kind === "file"}
            <GroupHeader kind="file" count={item.count}>
              {#snippet before()}{#if item.external}<Icon name="module" size={12} />{/if}{/snippet}
              <span class="path"><PathText path={item.path} /></span>
              {#snippet trailing()}{#if item.status}<ChangeLetter letter={item.status} />{/if}{/snippet}
            </GroupHeader>
          {:else}
            <button
              class="pick hit"
              class:cur={item.at === cursor}
              data-at={item.at}
              type="button"
              tabindex="-1"
              onclick={() => pick(item.row)}
              onmousemove={(e) => hover(e, item.at)}
            >
              <StatusDot tone={item.row.change === "hunk" ? "ok" : "none"} />
              <span class="mono line">{item.row.line}</span>
              <span class="within">{@render symbol(item.row)}</span>
              <span class="mono code">{@render code(item.row)}</span>
            </button>
          {/if}
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: fixed;
    z-index: 1000;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    outline: none;
    font-size: var(--fs-ctl);
    background: var(--bg-editor);
    overflow: hidden;
  }
  .head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    box-sizing: border-box;
    height: 33px;
    padding: 0 8px 0 12px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text);
  }
  .title code {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .grow {
    flex: 1;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .path {
    display: flex;
  }
  .hit,
  .two {
    display: flex;
    align-items: center;
    width: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-ctl);
    text-align: left;
    box-sizing: border-box;
    white-space: nowrap;
  }
  .hit {
    height: var(--h-row);
    padding: 0 12px 0 23px;
  }
  .two {
    align-items: flex-start;
    gap: 11px;
    padding: 5px 14px 5px 12px;
  }
  .two + .two {
    border-top: 1px solid var(--line-soft);
  }
  .hit > :global(.dot) {
    margin-right: 8px;
  }
  .two > :global(.dot) {
    margin-top: 5px;
  }
  .line {
    flex: none;
    width: 37px;
    text-align: right;
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .within {
    flex: none;
    display: flex;
    width: 151px;
    margin: 0 7px;
    overflow: hidden;
    font-size: var(--fs);
    color: var(--text-2);
  }
  .code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: pre;
    font-size: var(--fs-mono);
  }
  mark {
    background: var(--find);
    border-radius: 2px;
  }
  .rows {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .two .code {
    flex: none;
    line-height: 15px;
  }
  .first {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    height: 17px;
    line-height: 17px;
  }
  .sym {
    display: flex;
    font-size: var(--fs);
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
  }
  .where {
    flex: 0 10 auto;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
    overflow: hidden;
    font-size: var(--fs-mono-s);
    color: var(--text);
  }
  .where > :global(svg) {
    align-self: center;
  }
  .at,
  .pkg,
  .sep {
    flex: none;
    color: var(--text-3);
  }
</style>
