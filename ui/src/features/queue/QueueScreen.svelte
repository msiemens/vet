<script lang="ts">
  import { onMount } from "svelte";
  import type { CheckRun } from "../../generated/CheckRun";
  import type { Thread } from "../../generated/Thread";
  import { plural } from "../../lib/format";
  import { Preference } from "../../lib/preferences.svelte";
  import Avatar from "../../lib/ui/Avatar.svelte";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import EmptyState from "../../lib/ui/EmptyState.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import PanelTabs from "../../lib/ui/PanelTabs.svelte";
  import ProgressBar from "../../lib/ui/ProgressBar.svelte";
  import SearchField from "../../lib/ui/SearchField.svelte";
  import Table from "../../lib/ui/Table.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { type Sort, sortRows } from "../../lib/ui/sorting";
  import { changeset } from "../../state/changeset.svelte";
  import { keyId } from "../../state/changesetKey";
  import { commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { minute } from "../../state/minute.svelte";
  import { queue } from "../../state/queue.svelte";
  import ChangesCell from "./ChangesCell.svelte";
  import ChangesetName from "./ChangesetName.svelte";
  import {
    type QueueEntry,
    type QueueTab,
    TABS,
    authorOf,
    commentCount,
    filterRows,
    labelOf,
    parseTab,
    sourceText,
    statusOrder,
    stepTab,
    statusTitle,
    tabCounts,
    updatedAt,
    updatedText,
  } from "./queueRows";
  import { queueTabCommands, rowMenu } from "./queueCommands";
  import QueuePanel from "./QueuePanel.svelte";
  import ReviewedCell from "./ReviewedCell.svelte";
  import StatusChip from "./StatusChip.svelte";

  let {
    onopen,
    onopenthread,
    onopenlog,
  }: {
    /** Shows the review of the changeset on view. */
    onopen: () => void;
    /** … at a thread. */
    onopenthread: (thread: Thread) => void;
    /** … at the log of a failed check. */
    onopenlog: (run: CheckRun) => void;
  } = $props();

  const COLUMNS: Column<QueueEntry>[] = [
    { id: "changeset", label: "Changeset", flex: true, min: 320, sortBy: (row) => labelOf(row).text },
    { id: "status", label: "Status", width: 130, min: 60, sortBy: statusOrder },
    { id: "author", label: "Author", width: 140, min: 60, sortBy: (row) => authorOf(row)?.name },
    { id: "source", label: "Source", width: 130, min: 60 },
    { id: "changes", label: "Changes", width: 100, min: 60 },
    { id: "reviewed", label: "Reviewed", width: 120, min: 80 },
    { id: "icons", label: "", name: "checks and comments", width: 64 },
    {
      id: "updated",
      label: "Updated",
      width: 70,
      min: 50,
      align: "right",
      sortBy: updatedAt,
      firstSort: "descending",
    },
  ];

  const tabKept = new Preference("app", "queue.tab", parseTab, parseTab(undefined));
  const tab = $derived(tabKept.value);
  let query = $state("");
  let sort = $state<Sort>();

  const rows = $derived(queue.entries);
  const counts = $derived(tabCounts(rows));
  const shown = $derived(sortRows(filterRows(rows, tab, query), COLUMNS, sort));
  /** The repositories whose pull requests could not be listed. */
  const unlisted = $derived(tab === "all" || tab === "prs" ? queue.sources.filter((s) => s.error !== undefined) : []);
  const current = $derived(rows.find((r) => r.onView));
  /** The changeset's own numbers, whatever range or commit the review shows. */
  const own = $derived(queue.onViewFrame);

  function showTab(id: QueueTab) {
    tabKept.value = id;
  }

  /** The panel's link to the lines seen against an older base: the review shows them. */
  async function openBaseChanged() {
    await changeset.showBaseChanged();
    onopen();
  }

  onMount(() => {
    void queue.show();
    const unregister = commands.register(...queueTabCommands({ stepTab: (delta) => showTab(stepTab(tab, delta)) }));
    return () => {
      unregister();
    };
  });

  /** The selected row is the changeset on view: choosing one puts it on view. */
  function select(row: QueueEntry) {
    if (!row.onView) commands.run(`changeset.open:${keyId(row.key)}`);
  }

  /** Puts the changeset on view and shows its review. */
  function openRow(row: QueueEntry) {
    select(row);
    onopen();
  }

  let highlighted = $state(-1);
  let tableBody = $state<HTMLElement>();
  const nav = new ListNav<QueueEntry>({
    element: () => tableBody,
    items: () => shown,
    single: select,
    double: openRow,
    highlight: { get: () => highlighted, set: (i) => (highlighted = i) },
    reveal: (i) => tableBody?.querySelectorAll<HTMLElement>(".row")[i]?.scrollIntoView({ block: "nearest" }),
  });
  $effect(() => () => nav.cancel());

  /** A right click selects the row at once, and opens its menu. */
  function openMenu(e: MouseEvent, row: QueueEntry, index: number) {
    nav.cancel();
    highlighted = index;
    select(row);
    void contextMenus.open(rowMenu, e.clientX, e.clientY, { row: { key: row.key, url: row.pullRequest?.url } });
  }

  function onkeydown(e: KeyboardEvent) {
    if (!nav.keydown(e)) return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<section class="queue" aria-label="Queue">
  <div class="table">
    <div class="bar">
      <PanelTabs
        label="Changesets"
        value={tab}
        onchange={showTab}
        tabs={TABS.map((t) => ({ ...t, count: counts[t.id] }))}
      />
      <span class="grow"></span>
      <SearchField bind:value={query} label="Search changesets" width={260} />
    </div>
    <div class="loading">
      {#if queue.loading}<ProgressBar label="Loading changesets" />{/if}
    </div>

    <Table
      name="queue"
      columns={COLUMNS}
      bind:sort
      bind:body={tableBody}
      {onkeydown}
      {...nav.attributes}
      aria-label="Changesets"
    >
      {#each shown as row, index (keyId(row.key))}
        {@const changes = row.changes}
        {@const author = authorOf(row)}
        {@const comments = commentCount(row)}
        <button
          class="flat pick row"
          class:on={row.onView}
          class:cur={index === highlighted}
          type="button"
          tabindex="-1"
          aria-current={row.onView}
          onclick={(e) => nav.click(e, index)}
          oncontextmenu={(e) => openMenu(e, row, index)}
        >
          <span class="title"><ChangesetName label={labelOf(row)} /></span>
          <span
            ><StatusChip
              status={row.pullRequest?.status}
              title={row.pullRequest && statusTitle(row.pullRequest)}
            /></span
          >
          <span class="author" class:muted={author?.muted}>
            {#if author}
              {#if !author.muted}<Avatar name={author.name} url={author.avatar ?? null} />{/if}
              <span class="name" title={author.name}>{author.name}</span>
            {/if}
          </span>
          <span class="source">{sourceText(row)}</span>
          <span class="delta"><ChangesCell {changes} /></span>
          <span><ReviewedCell reviewed={row.reviewed} /></span>
          <span class="icons">
            <CiStatus state={row.pullRequest?.ci} />
            {#if comments > 0}
              <span
                class="threads"
                title={row.pullRequest ? plural(comments, "comment") : plural(comments, "open thread")}
                ><Icon name="comments" size={13} />{comments}</span
              >
            {/if}
          </span>
          <span class="right when">{updatedText(row, minute.now)}</span>
        </button>
      {:else}
        {#if !unlisted.length}
          <EmptyState text={queue.loading ? "Loading…" : rows.length ? "No matching changesets" : "No changesets"} />
        {/if}
      {/each}
      {#if shown.length && queue.listingPullRequests && (tab === "all" || tab === "prs")}
        <p class="pending">Loading pull requests…</p>
      {/if}
      {#each unlisted as source (source.remote)}
        <p class="unlisted">
          <span class="selectable">{source.remote}: {source.error}</span>
          {#if source.signedOut}<button class="link" type="button" onclick={() => commands.run("app.accounts")}
              >Sign in…</button
            >{/if}
        </p>
      {/each}
    </Table>
  </div>
  <QueuePanel row={current} frame={own} {onopenthread} {onopenlog} onbasechanged={() => void openBaseChanged()} />
</section>

<style>
  .queue {
    flex: 1;
    min-width: 0;
    display: flex;
    background: var(--bg-editor);
  }
  .table {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
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
  .author {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-ctl);
  }
  .author.muted {
    color: var(--text-3);
  }
  .author .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
  .threads {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--accent);
    font-size: var(--fs-mono-s);
  }
  .icons {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .loading {
    flex: none;
    height: var(--h-progress);
  }
  .pending,
  .unlisted {
    margin: 8px 16px;
    color: var(--text-3);
    font-size: var(--fs-ctl);
  }
</style>
