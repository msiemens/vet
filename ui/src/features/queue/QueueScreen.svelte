<script lang="ts">
  import { onMount } from "svelte";
  import type { ChangesetRow } from "../../generated/ChangesetRow";
  import type { Thread } from "../../generated/Thread";
  import Icon from "../../lib/ui/Icon.svelte";
  import PanelTabs from "../../lib/ui/PanelTabs.svelte";
  import { changeset } from "../../state/changeset.svelte";
  import { keyId } from "../../state/changesetKey";
  import { queue } from "../../state/queue.svelte";
  import { threads } from "../../state/threads.svelte";
  import { detailGroups } from "./details";
  import {
    type QueueTab,
    TABS,
    filterRows,
    reviewedOf,
    showsOwnLines,
    sourceText,
    tabCounts,
    updatedText,
  } from "./queueRows";
  import QueueComments from "./QueueComments.svelte";
  import QueueDetails from "./QueueDetails.svelte";
  import QueueFiles from "./QueueFiles.svelte";
  import ReviewedCell from "./ReviewedCell.svelte";

  let {
    onopen,
    onopenfile,
    onopenthread,
  }: {
    /** Shows the review of the changeset on view. */
    onopen: () => void;
    /** … at a file. */
    onopenfile: (path: string) => void;
    /** … at a thread. */
    onopenthread: (thread: Thread) => void;
  } = $props();

  let tab = $state<QueueTab>("all");
  let query = $state("");
  let panel = $state<"details" | "files" | "comments">("details");

  const counts = $derived(tabCounts(queue.rows));
  const shown = $derived(filterRows(queue.rows, tab, query));
  const manifest = $derived(changeset.manifest);
  const onViewId = $derived(queue.onView && keyId(queue.onView));
  const current = $derived(queue.rows.find((r) => keyId(r.key) === onViewId));
  /** The changeset's own numbers; a range of versions is not them. */
  const own = $derived(manifest && !manifest.interdiff ? manifest : undefined);

  onMount(() => void queue.load());

  /** What the table knows of a row's lines: only the changeset on view has them. */
  const ownOf = (row: ChangesetRow) => (keyId(row.key) === onViewId ? own : undefined);

  /** The selected row is the changeset on view: choosing one puts it on view. */
  function select(row: ChangesetRow) {
    if (keyId(row.key) !== onViewId) void changeset.openChangeset(row.key);
  }

  function move(e: KeyboardEvent, index: number) {
    if (e.key === "Enter") {
      e.preventDefault();
      select(shown[index]);
      onopen();
      return;
    }
    const to = e.key === "ArrowDown" ? index + 1 : e.key === "ArrowUp" ? index - 1 : undefined;
    if (to === undefined || to < 0 || to >= shown.length) return;
    e.preventDefault();
    select(shown[to]);
    (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>(".row")[to]?.focus();
  }
</script>

<section class="queue" aria-label="Queue">
  <div class="bar">
    <PanelTabs
      label="Changesets"
      value={tab}
      onchange={(id) => void (tab = id)}
      tabs={TABS.map((t) => ({ ...t, count: counts[t.id] }))}
    />
    <span class="grow"></span>
    <label class="search">
      <Icon name="search" size={13} />
      <input class="field" type="text" bind:value={query} aria-label="Search changesets" spellcheck="false" />
    </label>
  </div>

  <div class="table">
    <div class="row head">
      <span>Changeset</span>
      <span>Source</span>
      <span>Changes</span>
      <span>Reviewed</span>
      <span></span>
      <span class="right">Updated</span>
    </div>
    <div class="rows" aria-label="Changesets">
      {#each shown as row, index (keyId(row.key))}
        {@const id = keyId(row.key)}
        {@const numbers = ownOf(row)}
        <button
          class="flat row"
          class:on={id === onViewId}
          type="button"
          aria-current={id === onViewId}
          onclick={() => select(row)}
          ondblclick={() => {
            select(row);
            onopen();
          }}
          onkeydown={(e) => move(e, index)}
        >
          <span class="title">{row.title}</span>
          <span class="source">{sourceText(row)}</span>
          <span class="mono delta">
            {#if numbers && showsOwnLines(row)}<span class="add">+{numbers.delta.added}</span>
              <span class="del">−{numbers.delta.removed}</span>{/if}
          </span>
          <span><ReviewedCell reviewed={reviewedOf(row.review, numbers?.completion)} /></span>
          <span class="icons">
            {#if row.openThreads > 0}
              <span class="threads" title="{row.openThreads} open {row.openThreads === 1 ? 'thread' : 'threads'}"
                ><Icon name="comments" size={13} />{row.openThreads}</span
              >
            {/if}
          </span>
          <span class="right when">{updatedText(row)}</span>
        </button>
      {:else}
        <p class="empty">{queue.rows.length ? "No matching changesets" : "No changesets"}</p>
      {/each}
    </div>
  </div>

  <div class="panel">
    <PanelTabs
      label="Selected changeset"
      value={panel}
      onchange={(id) => void (panel = id)}
      tabs={[
        { id: "details", label: "Details" },
        { id: "files", label: "Files", count: changeset.files.length },
        { id: "comments", label: "Comments", count: threads.items.length },
      ]}
    />
    <div class="body">
      {#if !current}
        <p class="empty">Select a changeset</p>
      {:else if panel === "details"}
        <QueueDetails groups={detailGroups(current, manifest)} />
      {:else if panel === "files"}
        <QueueFiles paths={changeset.paths} fileOf={(path) => changeset.file(path)} onopen={onopenfile} />
      {:else}
        <QueueComments threads={threads.items} onopen={onopenthread} />
      {/if}
    </div>
  </div>
</section>

<style>
  .queue {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    padding: 0 8px 0 6px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--line-soft);
  }
  .grow {
    flex: 1;
  }
  .search {
    position: relative;
    display: flex;
    align-items: center;
    width: 220px;
    color: var(--text-3);
  }
  .search :global(svg) {
    position: absolute;
    left: 8px;
    pointer-events: none;
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 22px;
    padding-left: 26px;
  }
  .table {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .rows {
    flex: 1;
    min-height: 0;
    overflow: hidden auto;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) 120px 190px 80px 90px;
    align-items: center;
    column-gap: 12px;
    width: 100%;
    height: var(--h-queue-row);
    padding: 0 16px;
    box-sizing: border-box;
    border-radius: 0;
    text-align: left;
    font-size: var(--fs);
  }
  .row.on {
    background: var(--toggle);
  }
  .head {
    flex: none;
    height: 26px;
    border-bottom: 1px solid var(--line-soft);
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .title,
  .source {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .source {
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .right {
    text-align: right;
  }
  .when {
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .delta {
    font-size: var(--fs-mono-s);
  }
  .add {
    color: var(--add-text);
  }
  .del {
    color: var(--del-text);
  }
  .threads {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--accent);
    font-size: var(--fs-mono-s);
  }
  .panel {
    flex: none;
    height: var(--h-queue-panel);
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-top: 1px solid var(--line);
  }
  .panel > :global(.tabs) {
    flex: none;
    padding: 0 6px;
    border-bottom: 1px solid var(--line-soft);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden auto;
  }
  .empty {
    margin: 12px 16px;
    color: var(--text-3);
  }
</style>
