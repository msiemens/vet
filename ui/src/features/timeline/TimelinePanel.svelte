<script lang="ts">
  import type { LineDelta } from "../../generated/LineDelta";
  import type { VersionInfo } from "../../generated/VersionInfo";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { type Range, rowRange } from "../../state/timelineRange";
  import { type RangeFile, timelineRows } from "./timelineRows";

  let {
    versions,
    range,
    files,
    selected,
    onrange,
    onselect,
    onclose,
  }: {
    /** Oldest first. */
    versions: readonly VersionInfo[];
    /** The range the review view shows. */
    range?: Range;
    /** The files the range changes. */
    files: RangeFile[];
    /** The file the review view is at. */
    selected?: string;
    onrange: (range: Range | undefined) => void;
    onselect: (path: string) => void;
    onclose: () => void;
  } = $props();

  const rows = $derived(timelineRows(versions, range));
  /** The version clicked last, which a shift-click runs the range from. */
  let anchor = $state<number>();

  const time = (at: number) => new Date(at * 1000).toLocaleTimeString(undefined, { hour12: false });

  function choose(n: number, shift: boolean) {
    const chosen = rowRange(versions, n, shift ? (anchor ?? range?.to) : undefined);
    if (!chosen) return;
    if (!shift) anchor = n;
    onrange(chosen);
  }
</script>

{#snippet counts({ added, removed }: LineDelta)}
  <span class="mono delta"><span class="add">+{added}</span> <span class="del">−{removed}</span></span>
{/snippet}

<section aria-label="Timeline">
  <header>
    <span class="title">Timeline <span class="count">{versions.length}</span></span>
    <span class="grow"></span>
    <IconButton icon="minus" label="Collapse the timeline" size={22} iconSize={14} onclick={onclose} />
  </header>
  <div class="body">
    <div class="table">
      <div class="row head">
        <span>Version</span><span>Time</span><span>Files</span><span class="right">Lines</span>
      </div>
      <ul class="rows" aria-label="Versions, newest first">
        {#each rows as row (row.n)}
          <li>
            <button
              class="flat row"
              class:edge={row.band === "end"}
              class:inside={row.band === "inside"}
              type="button"
              aria-current={row.band ? "true" : undefined}
              onclick={(e) => choose(row.n, e.shiftKey)}
            >
              <span class="mono n">v{row.n}</span>
              <span class="mono time">{time(row.at)}</span>
              <span class="names">
                {#if row.rebasedOnto !== undefined}
                  <span class="rebased"
                    ><Icon name="branch" size={13} /> rebased onto <span class="mono">{row.rebasedOnto}</span></span
                  >
                {:else}
                  {row.files}
                {/if}
                {#if row.reviewed}<span class="eye" title="Last reviewed here"><Icon name="eye" size={13} /></span>{/if}
              </span>
              <span class="right">
                {#if row.lines}{@render counts(row.lines)}{:else}<span class="none">–</span>{/if}
              </span>
            </button>
          </li>
        {:else}
          <li class="empty">No versions yet</li>
        {/each}
      </ul>
    </div>
    <ul class="files" aria-label="Files changed in the range">
      {#each files as file (file.path)}
        <li>
          <button class="flat file" class:on={file.path === selected} type="button" onclick={() => onselect(file.path)}>
            <span class="path">{file.path}</span>
            {#if file.mode}<span class="mono mode">{file.mode}</span>{/if}
            {@render counts(file.delta)}
          </button>
        </li>
      {/each}
    </ul>
  </div>
</section>

<style>
  section {
    flex: none;
    height: var(--h-timeline);
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--bg-panel);
  }
  header {
    flex: none;
    display: flex;
    align-items: center;
    height: 34px;
    padding: 0 8px 0 14px;
    border-bottom: 1px solid var(--line-soft);
    font-weight: 600;
    color: var(--text-2);
  }
  .count {
    margin-left: 4px;
    font-weight: 400;
    color: var(--text-3);
  }
  .grow {
    flex: 1;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .table {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
  }
  .rows {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }
  .row {
    display: grid;
    grid-template-columns: 56px 96px minmax(0, 1fr) auto;
    align-items: center;
    column-gap: 12px;
    width: 100%;
    height: 26px;
    padding: 0 12px 0 14px;
    box-sizing: border-box;
    text-align: left;
    font-size: var(--fs-ctl);
    border-radius: 0;
  }
  .head {
    flex: none;
    height: 24px;
    color: var(--text-3);
    border-bottom: 1px solid var(--line-soft);
    font-size: var(--fs-mono-s);
  }
  .row.edge,
  .row.inside {
    box-shadow: inset 3px 0 var(--accent);
  }
  .row.inside {
    background: color-mix(in srgb, var(--accent) 9%, transparent);
  }
  .row.edge {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .right {
    text-align: right;
  }
  .names {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .rebased {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
  }
  .eye {
    display: inline-flex;
    color: var(--accent);
  }
  .none {
    color: var(--text-3);
  }
  .empty {
    padding: 12px 14px;
    color: var(--text-3);
  }
  .files {
    flex: none;
    width: var(--w-timeline-files);
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
    border-left: 1px solid var(--line);
  }
  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 26px;
    padding: 0 12px;
    box-sizing: border-box;
    text-align: left;
    font-size: var(--fs-ctl);
    border-radius: 0;
  }
  .file.on {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .mode {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .delta {
    font-size: 11.5px;
  }
  .add {
    color: var(--add-text);
  }
  .del {
    color: var(--del-text);
  }
</style>
