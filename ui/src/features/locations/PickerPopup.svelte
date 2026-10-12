<script lang="ts">
  // The places Show Usages or Go to Implementation found, under the caret: grouped by file when
  // there are many, two lines each when there are few. Arrow keys, Page keys, Home and End move,
  // Enter opens, Escape closes, and typing filters.
  import { onDestroy, onMount, tick } from "svelte";
  import type { LocationRow } from "../../generated/LocationRow";
  import { LIST } from "../../lib/listKeys";
  import { highlightRuns } from "../../lib/speedSearch";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import type { PickerView } from "../../state/pickerTypes";
  import { FLAT_UP_TO, countText, groupRows, libraryLabel, visibleRows } from "./pickerRows";

  let {
    view,
    statusOf,
    onsettings,
  }: {
    view: PickerView;
    /** `M`, `A`, `D` or `R` for a changed file, none for any other. */
    statusOf: (path: string) => string | undefined;
    /** Opens Settings at the language servers. */
    onsettings: () => void;
  } = $props();

  const STATUS: Record<string, string> = {
    M: "var(--old-base)",
    A: "var(--add-text)",
    D: "var(--del-text)",
    R: "var(--text-2)",
  };
  /** The most the popup grows to, and how wide it is. */
  const MAX_HEIGHT = 520;
  const WIDTH = 720;
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
  const flat = $derived(!!results && results.rows.length <= FLAT_UP_TO);
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

  const geometry = $derived.by(() => {
    const { anchor } = view;
    const below = window.innerHeight - anchor.bottom - 12;
    const above = anchor.top - 12;
    const rows = flat ? shown.length * 46 : items.length * 24;
    const want = Math.min(MAX_HEIGHT, 56 + rows + (search.active ? 32 : 0) + (results ? 0 : 56));
    const down = below >= want || below >= above;
    const width = Math.min(WIDTH, window.innerWidth - 16);
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

  function onkeydown(e: KeyboardEvent) {
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

  /** What a place is in: its symbol, else its file. */
  const symbolOf = (row: LocationRow) => row.within?.name.join("::") ?? row.path.slice(row.path.lastIndexOf("/") + 1);
</script>

<svelte:window
  onpointerdown={(e) => root && !root.contains(e.target as Node) && view.onclose()}
  onresize={() => view.onclose()}
/>

{#snippet code(row: LocationRow)}
  {#each highlightRuns(row.text, [[row.matchStart, row.matchEnd]]) as run, i (i)}{#if run.hit}<mark>{run.text}</mark
      >{:else}{run.text}{/if}{/each}
{/snippet}

{#snippet status(path: string)}
  {@const letter = statusOf(path)}
  {#if letter}<span class="mono letter" style:color={STATUS[letter]}>{letter}</span>{/if}
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
      <span class="mono count">{count.count}</span>
      {#if count.cut}<span class="mono cut">· {count.cut}</span>{/if}
    {/if}
    <span class="grow"></span>
    {#if view.body.phase !== "stopped"}
      <IconButton
        icon="review"
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
    <div class="state"><span class="spinner" aria-hidden="true"></span>Searching…</div>
  {:else if view.body.phase === "stopped"}
    <div class="state stopped">
      <span class="warn"><Icon name="warning" size={16} /></span>
      <span class="message">{view.body.message}</span>
      <span class="grow"></span>
      <button class="ctl" type="button" onclick={() => view.onretry()}>Restart</button>
      <button
        class="ctl"
        type="button"
        onclick={() => {
          view.onclose();
          onsettings();
        }}>Settings…</button
      >
    </div>
  {:else if !hits.length}
    <div class="state">{results?.rows.length ? "No matches" : `No ${nouns}`}</div>
  {:else}
    <div class="list" bind:this={list}>
      {#if flat}
        {#each shown as row, at (`${row.path}:${row.line}:${row.col}`)}
          <button
            class="two"
            class:cur={at === cursor}
            data-at={at}
            type="button"
            tabindex="-1"
            onclick={() => pick(row)}
            onmousemove={() => (cursor = at)}
          >
            <span class="dot" class:on={row.change === "hunk"}></span>
            <span class="rows">
              <span class="first">
                <span class="sym">{symbolOf(row)}</span>
                <span class="where">
                  {#if row.external}
                    {@const lib = libraryLabel(row.path)}
                    <Icon name="module" size={12} />
                    {#if lib.package}<span>{lib.package}</span><span class="sep">›</span>{/if}
                    <PathText path={lib.rest} suffix=":{row.line}" title={row.path} />
                  {:else}
                    <PathText path={row.path} suffix=":{row.line}" />
                    {@render status(row.path)}
                  {/if}
                </span>
              </span>
              <span class="mono code">{@render code(row)}</span>
            </span>
          </button>
        {/each}
      {:else}
        {#each items as item, i (i)}
          {#if item.kind === "section"}
            <div class="section">{item.label}</div>
          {:else if item.kind === "file"}
            <div class="file">
              {#if item.external}<Icon name="module" size={12} />{/if}
              <PathText path={item.path} />
              {#if item.status}<span class="mono letter" style:color={STATUS[item.status]}>{item.status}</span>{/if}
              <span class="grow"></span>
              <span class="mono n">{item.count}</span>
            </div>
          {:else}
            <button
              class="hit"
              class:cur={item.at === cursor}
              data-at={item.at}
              type="button"
              tabindex="-1"
              onclick={() => pick(item.row)}
              onmousemove={() => {
                cursor = item.at;
              }}
            >
              <span class="dot" class:on={item.row.change === "hunk"}></span>
              <span class="mono line">{item.row.line}</span>
              <span class="within">{item.row.within?.name.join("::") ?? ""}</span>
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
    height: 36px;
    padding: 0 8px 0 12px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text);
  }
  .title code {
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .count,
  .cut {
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .grow {
    flex: 1;
  }
  .state {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 56px;
    padding: 0 12px;
    color: var(--text-2);
  }
  .stopped {
    justify-content: flex-start;
    color: var(--text);
  }
  .warn {
    display: inline-flex;
    color: var(--warn);
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--track);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 4px;
  }
  .section {
    padding: 10px 12px 2px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: var(--text-3);
  }
  .file {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-row);
    padding: 0 12px;
    color: var(--text-2);
    white-space: nowrap;
  }
  .letter {
    flex: none;
    font-size: 11px;
  }
  .n {
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .hit,
  .two {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 0 12px;
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
  }
  .two {
    padding-top: 5px;
    padding-bottom: 5px;
    align-items: center;
  }
  .cur {
    background: var(--select);
  }
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }
  .dot.on {
    background: var(--add-text);
  }
  .line {
    flex: none;
    width: 36px;
    text-align: right;
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .within {
    flex: none;
    width: 170px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-2);
  }
  .code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--fs-mono);
  }
  mark {
    background: var(--find);
    color: inherit;
    border-radius: 2px;
  }
  .rows {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .first {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .sym {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .where {
    flex: none;
    max-width: 55%;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
  .sep {
    color: var(--text-3);
  }
  .two .code {
    flex: none;
  }
</style>
