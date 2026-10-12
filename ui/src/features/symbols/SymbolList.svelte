<script lang="ts">
  // The changed symbols by file, with the current file's group in view.
  import { tick } from "svelte";
  import type { ChangedSymbol } from "../../generated/ChangedSymbol";
  import { ListNav } from "../../lib/ui/listNav";
  import Marked from "../../lib/ui/Marked.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import QualifiedName from "../../lib/ui/QualifiedName.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import { SpeedSearch } from "../../lib/ui/speedSearch.svelte";
  import SpeedSearchField from "../../lib/ui/SpeedSearchField.svelte";
  import { keyOf } from "../../state/symbolKey";
  import { symbols } from "../../state/symbols.svelte";
  import { containerOf, fileEntries, nameOf, rowKeys, searchEntries } from "./symbolList";

  /** A double click or Enter: jump to the symbol's source. */
  let { onjump }: { onjump: (symbol: ChangedSymbol) => void } = $props();

  const changed = $derived(symbols.changed);
  const search = new SpeedSearch();
  const listed = $derived(
    changed?.state === "done"
      ? fileEntries(changed.value).filter((e) => !symbols.currentOnly || e.path === symbols.file)
      : [],
  );
  const searched = $derived(search.active ? searchEntries(listed, search.query) : undefined);
  const entries = $derived(searched?.entries ?? listed);
  const selectedKey = $derived(symbols.selected && keyOf(symbols.selected.symbol.id));
  const SIGN = { added: "+", modified: "~", removed: "−" } as const;

  /** The symbols in the order the rows show them. */
  const flat = $derived(entries.flatMap((e) => e.symbols));
  /** Where the symbols of each file start in `flat`. */
  const starts = $derived(entries.map((_, i) => entries.slice(0, i).reduce((n, e) => n + e.symbols.length, 0)));
  let highlighted = $state(-1);
  let list = $state<HTMLElement>();
  const nav = new ListNav<ChangedSymbol>({
    element: () => list,
    items: () => flat,
    single: (s) => symbols.select(s),
    double: (s) => onjump(s),
    highlight: { get: () => highlighted, set: (i) => (highlighted = i) },
    reveal: (i) =>
      void tick().then(() =>
        list?.querySelector<HTMLElement>(`[data-index="${i}"]`)?.scrollIntoView({ block: "nearest" }),
      ),
  });
  $effect(() => () => nav.cancel());

  function onkeydown(e: KeyboardEvent) {
    if (search.type(e)) {
      highlighted = flat.length ? 0 : -1;
      return;
    }
    if (nav.keydown(e)) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  // The review moved to another file: bring its group into view (the list is not a stream, so this is cheap).
  $effect(() => {
    const path = symbols.file;
    // Also when the list was rebuilt (a new revision, the filter).
    if (!list || path === undefined || !entries.length) return;
    const root = list;
    void tick().then(() => {
      for (const el of root.querySelectorAll<HTMLElement>("[data-path]")) {
        if (el.dataset.path === path) el.scrollIntoView({ block: "nearest" });
      }
    });
  });
</script>

<div class="sh sh-bar">
  <span>Changed symbols</span>
  {#if changed?.state === "done"}<span class="mono n">{changed.value.symbols.length}</span>{/if}
  <span class="grow"></span>
  <button
    class="flat only"
    type="button"
    aria-pressed={symbols.currentOnly}
    disabled={symbols.file === undefined}
    title="List only the symbols of the file the review shows"
    onclick={() => (symbols.currentOnly = !symbols.currentOnly)}>This file</button
  >
</div>
{#if search.active}<SpeedSearchField query={search.query} found={entries.length > 0} />{/if}
<div class="list">
  <ScrollArea bind:viewport={list} {onkeydown} {...nav.attributes} {...search.attributes}>
    {#if !changed || changed.state === "loading"}
      <p class="note">Loading…</p>
    {:else if changed.state === "failed"}
      <p class="note err">{changed.message}</p>
    {:else if !entries.length}
      <p class="note">{changed.value.symbols.length ? "No changed symbols in this file" : "No changed symbols"}</p>
    {:else}
      {#each entries as entry, at (entry.path)}
        <div class="file" class:current={entry.path === symbols.file} data-path={entry.path}>
          <span class="path"><PathText path={entry.path} /></span>
          {#if entry.note}<span class="fnote">{entry.note}</span>{/if}
        </div>
        {@const keys = rowKeys(entry.symbols)}
        {#each entry.symbols as s, i (keys[i])}
          <button
            class="flat row"
            class:cur={starts[at] + i === highlighted}
            type="button"
            tabindex="-1"
            data-index={starts[at] + i}
            aria-pressed={keyOf(s.symbol.id) === selectedKey}
            title="Double-click to show it in the review"
            onclick={(e) => nav.click(e, starts[at] + i)}
          >
            <span class="sign {s.change}" title={s.change}>{SIGN[s.change]}</span>
            <span class="mono name" class:gone={s.change === "removed"}
              ><Marked text={nameOf(s.symbol.id)} ranges={searched?.ranges.get(s)} /></span
            >
            {#if containerOf(s.symbol.id)}
              <span class="mono container"><QualifiedName name={containerOf(s.symbol.id)} /></span>
            {/if}
          </button>
        {/each}
      {/each}
    {/if}
  </ScrollArea>
</div>

<style>
  .only {
    height: 20px;
    padding: 0 6px;
    font-size: 11px;
    font-weight: 400;
  }
  .list {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--line);
  }
  .note {
    padding: 6px 10px;
  }
  .file {
    height: var(--h-row);
    display: flex;
    align-items: center;
    padding: 0 10px;
    overflow: hidden;
    white-space: nowrap;
    font-size: 11px;
  }
  .path {
    flex: 0 1 auto;
    min-width: 0;
    display: flex;
  }
  .fnote {
    flex: none;
    margin-left: 8px;
    font-style: italic;
    color: var(--text-3);
  }
  .file.current {
    font-weight: 600;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: var(--h-row);
    padding: 0 10px 0 16px;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
  }
  /* The highlighted row, which Space and Enter act on, while the list has the focus. */
  .list:focus-within .row.cur {
    background: var(--select);
  }
  .sign {
    width: 10px;
    text-align: center;
    font-weight: 600;
  }
  .added {
    color: var(--add-text);
  }
  .modified {
    color: var(--old-base);
  }
  .removed {
    color: var(--del-text);
  }
  .name {
    font-size: var(--fs-mono-s);
  }
  .gone {
    text-decoration: line-through;
    color: var(--text-2);
  }
  .container {
    min-width: 0;
    display: flex;
    font-size: 11px;
  }
</style>
