<script lang="ts">
  // The changed symbols by file, with the current file's group in view.
  import { tick } from "svelte";
  import { containerOf, fileEntries, keyOf, nameOf } from "./symbolList";
  import { symbols } from "./symbols.svelte";

  let list = $state<HTMLElement>();

  const changed = $derived(symbols.changed);
  const entries = $derived(
    changed?.state === "done"
      ? fileEntries(changed.value).filter((e) => !symbols.currentOnly || e.path === symbols.file)
      : [],
  );
  const selectedKey = $derived(symbols.selected && keyOf(symbols.selected.symbol.id));
  const SIGN = { added: "+", modified: "~", removed: "−" } as const;

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

<div class="bar">
  <span class="title">Changed symbols</span>
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
<div class="list" bind:this={list}>
  {#if !changed || changed.state === "loading"}
    <p class="note">Loading…</p>
  {:else if changed.state === "failed"}
    <p class="note err">{changed.message}</p>
  {:else if !entries.length}
    <p class="note">{changed.value.symbols.length ? "No changed symbols in this file" : "No changed symbols"}</p>
  {:else}
    {#each entries as entry (entry.path)}
      <div class="file" class:current={entry.path === symbols.file} data-path={entry.path} title={entry.path}>
        <span class="path">{entry.path}</span>
        {#if entry.note}<span class="fnote">{entry.note}</span>{/if}
      </div>
      {#each entry.symbols as s (keyOf(s.symbol.id) + s.change)}
        <button
          class="flat row"
          type="button"
          aria-pressed={keyOf(s.symbol.id) === selectedKey}
          onclick={() => symbols.select(s)}
        >
          <span class="sign {s.change}" title={s.change}>{SIGN[s.change]}</span>
          <span class="mono name" class:gone={s.change === "removed"}>{nameOf(s.symbol.id)}</span>
          <span class="mono container">{containerOf(s.symbol.id)}</span>
        </button>
      {/each}
    {/each}
  {/if}
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-ctl);
    padding: 0 6px 0 10px;
    flex: none;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
    border-bottom: 1px solid var(--line-soft);
  }
  .n {
    font-weight: 400;
    color: var(--text-3);
  }
  .grow {
    flex: 1;
  }
  .only {
    height: 20px;
    padding: 0 6px;
    font-size: 11px;
    font-weight: 400;
  }
  .list {
    overflow: hidden auto;
    flex: 1;
    min-height: 0;
    border-bottom: 1px solid var(--line);
  }
  .note {
    margin: 0;
    padding: 6px 10px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
  .err {
    color: var(--del-text);
    white-space: pre-wrap;
  }
  .file {
    height: var(--h-row);
    display: flex;
    align-items: center;
    padding: 0 10px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 11px;
    color: var(--text-3);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fnote {
    flex: none;
    margin-left: 8px;
    font-style: italic;
  }
  .file.current {
    color: var(--text);
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
    font-size: 11px;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
