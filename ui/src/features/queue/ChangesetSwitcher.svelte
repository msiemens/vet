<script lang="ts">
  import { tick, untrack } from "svelte";
  import type { ChangesetKey } from "../../generated/ChangesetKey";
  import Chip from "../../lib/ui/Chip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import { changeset } from "../../state/changeset.svelte";
  import { keyId } from "../../state/changesetKey";
  import { commands } from "../../state/commands";
  import { queue } from "../../state/queue.svelte";
  import ChangesCell from "./ChangesCell.svelte";
  import ChangesetName from "./ChangesetName.svelte";
  import {
    type Label,
    type QueueTab,
    SWITCHER_FILTERS,
    changesOf,
    commentCount,
    filterRows,
    labelOf,
    queueRows,
    reviewedOf,
    sourceText,
    updatedText,
  } from "./queueRows";
  import ReviewedCell from "./ReviewedCell.svelte";

  let {
    label,
    draft = false,
    title,
    open = $bindable(false),
    onpick,
    onqueue,
    onopenpullrequest,
  }: {
    /** What the changeset on view is called; unknown until the manifest arrives. */
    label?: Label;
    /** The changeset on view is a draft pull request. */
    draft?: boolean;
    title?: string;
    open?: boolean;
    onpick: (key: ChangesetKey) => void;
    /** Shows the queue. */
    onqueue: () => void;
    /** Asks for a pull request by its number. */
    onopenpullrequest: () => void;
  } = $props();

  let query = $state("");
  let filter = $state<QueueTab>("all");
  let cursor = $state(0);
  let list = $state<HTMLElement>();

  const rows = $derived(queueRows(queue.local, queue.pullRequests));
  const shown = $derived(filterRows(rows, filter, query));
  const onViewId = $derived(queue.onView && keyId(queue.onView));
  const own = $derived(changeset.manifest && !changeset.manifest.interdiff ? changeset.manifest : undefined);

  // The changesets are listed afresh each time the popover opens.
  $effect(() => {
    if (!open) return;
    [query, filter, cursor] = ["", "all", 0];
    untrack(() => void queue.loadAll());
  });

  const focus = (node: HTMLElement) => node.focus();

  async function move(by: number) {
    cursor = Math.max(0, Math.min(cursor + by, shown.length - 1));
    await tick();
    list?.querySelector(".cursor")?.scrollIntoView({ block: "nearest" });
  }

  function pick(index: number) {
    const row = shown.at(index);
    if (!row) return;
    open = false;
    onpick(row.key);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      void move(e.key === "ArrowDown" ? 1 : -1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(Math.min(cursor, shown.length - 1));
    }
  }
</script>

<Popover bind:open>
  {#snippet trigger(toggle)}
    <Chip active={open} {title} onclick={toggle}>
      <Icon name="branch" />{#if label}<span class="current"><ChangesetName {label} /></span>{:else}…{/if}
      {#if draft}<span class="badge">Draft</span>{/if}
    </Chip>
  {/snippet}
  <div class="switcher">
    <div class="top">
      <label class="search">
        <Icon name="search" size={13} />
        <input
          class="field"
          type="text"
          use:focus
          bind:value={query}
          oninput={() => (cursor = 0)}
          {onkeydown}
          aria-label="Search changesets"
          spellcheck="false"
        />
      </label>
      <Segmented
        small
        label="Show"
        options={SWITCHER_FILTERS}
        value={filter}
        onchange={(id) => {
          filter = id;
          cursor = 0;
        }}
      />
    </div>
    <ul bind:this={list} aria-label="Changesets">
      {#each shown as row, index (keyId(row.key))}
        {@const id = keyId(row.key)}
        {@const numbers = id === onViewId ? own : undefined}
        {@const changes = changesOf(row, numbers?.delta, queue.lines.get(id))}
        {@const comments = commentCount(row)}
        <li>
          <button
            class="flat row"
            class:on={id === onViewId}
            class:cursor={index === Math.min(cursor, shown.length - 1)}
            type="button"
            tabindex="-1"
            onclick={() => pick(index)}
          >
            <span class="line">
              <span class="name"><ChangesetName label={labelOf(row)} /></span>
              <ReviewedCell reviewed={reviewedOf(row.review, numbers?.completion)} width={48} />
            </span>
            <span class="line sub">
              <span class="detail">
                {sourceText(row)}
                <ChangesCell {changes} />
                {#if comments > 0}<span class="threads"><Icon name="comments" size={11} />{comments}</span>{/if}
              </span>
              <span>{updatedText(row)}</span>
            </span>
          </button>
        </li>
      {:else}
        <li class="empty">{rows.length ? "No matching changesets" : "No changesets"}</li>
      {/each}
    </ul>
    <div class="foot">
      <button
        class="flat"
        type="button"
        onclick={() => {
          open = false;
          onqueue();
        }}>Open queue <span class="key">{commands.binding("view.queue")?.label}</span></button
      >
      <button
        class="flat"
        type="button"
        onclick={() => {
          open = false;
          onopenpullrequest();
        }}>Open pull request… <span class="key">{commands.binding("changeset.openPullRequest")?.label}</span></button
      >
    </div>
  </div>
</Popover>

<style>
  .switcher {
    width: var(--w-switcher);
    display: flex;
    flex-direction: column;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px;
    border-bottom: 1px solid var(--line-soft);
  }
  .search {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
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
  ul {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: min(60vh, 420px);
    overflow: hidden auto;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    padding: 4px 8px;
    box-sizing: border-box;
    border-radius: var(--radius);
    text-align: left;
  }
  .row.on {
    background: var(--toggle);
  }
  .row.cursor:not(.on) {
    background: var(--hover);
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-ctl);
  }
  .sub {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .detail {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .threads {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--accent);
  }
  .empty {
    padding: 8px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
  .foot {
    display: flex;
    padding: 4px;
    border-top: 1px solid var(--line-soft);
  }
  .foot button {
    flex: 1;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: var(--h-row);
    padding: 0 8px;
    border-radius: var(--radius);
    font-size: var(--fs-ctl);
  }
  .current {
    display: block;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .key {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
</style>
