<script lang="ts">
  import { onMount } from "svelte";
  import type { Thread } from "../../generated/Thread";
  import { readStored, writeStored } from "../../lib/storage";
  import Avatar from "../../lib/ui/Avatar.svelte";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import { ListNav } from "../../lib/ui/listNav";
  import PanelTabs from "../../lib/ui/PanelTabs.svelte";
  import ProgressBar from "../../lib/ui/ProgressBar.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import Table from "../../lib/ui/Table.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { type Sort, sortRows } from "../../lib/ui/sorting";
  import { changeset } from "../../state/changeset.svelte";
  import { keyId } from "../../state/changesetKey";
  import { commands } from "../../state/commands";
  import { contextMenus } from "../../state/contextMenu.svelte";
  import { queue } from "../../state/queue.svelte";
  import { threads } from "../../state/threads.svelte";
  import ChangesCell from "./ChangesCell.svelte";
  import ChangesetName from "./ChangesetName.svelte";
  import { detailGroups } from "./details";
  import {
    type QueueRow,
    TABS,
    authorOf,
    changesOf,
    commentCount,
    filterRows,
    labelOf,
    parsePanel,
    parseTab,
    queueRows,
    reviewedOf,
    sourceText,
    statusOrder,
    statusTitle,
    tabCounts,
    updatedAt,
    updatedText,
  } from "./queueRows";
  import { rowMenu } from "./queueCommands";
  import QueueComments from "./QueueComments.svelte";
  import QueueDetails from "./QueueDetails.svelte";
  import QueueFiles from "./QueueFiles.svelte";
  import ReviewedCell from "./ReviewedCell.svelte";
  import StatusChip from "./StatusChip.svelte";

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

  const COLUMNS: Column<QueueRow>[] = [
    { id: "changeset", label: "Changeset", flex: true, min: 120, sortBy: (row) => labelOf(row).text },
    { id: "status", label: "Status", width: 130, min: 60, sortBy: statusOrder },
    { id: "author", label: "Author", width: 150, min: 60, sortBy: (row) => authorOf(row)?.name },
    { id: "source", label: "Source", width: 150, min: 60 },
    { id: "changes", label: "Changes", width: 110, min: 60 },
    { id: "reviewed", label: "Reviewed", width: 190, min: 80 },
    { id: "icons", label: "", name: "checks and comments", width: 80 },
    {
      id: "updated",
      label: "Updated",
      width: 90,
      min: 50,
      align: "right",
      sortBy: updatedAt,
      firstSort: "descending",
    },
  ];

  const TAB_KEY = "vet.queue.tab";
  const PANEL_KEY = "vet.queue.panel";

  let tab = $state(parseTab(readStored(TAB_KEY)));
  let query = $state("");
  let panel = $state(parsePanel(readStored(PANEL_KEY)));
  let sort = $state<Sort>();

  const rows = $derived(queueRows(queue.local, queue.pullRequests));
  const counts = $derived(tabCounts(rows));
  const shown = $derived(sortRows(filterRows(rows, tab, query), COLUMNS, sort));
  /** The repositories whose pull requests could not be listed. */
  const unlisted = $derived(tab === "all" || tab === "prs" ? queue.sources.filter((s) => s.error !== undefined) : []);
  const onViewId = $derived(queue.onView && keyId(queue.onView));
  const current = $derived(rows.find((r) => keyId(r.key) === onViewId));
  /** The changeset's own numbers, whatever range or commit the review shows. */
  const own = $derived(queue.onViewFrame);

  onMount(() => {
    void queue.loadAll();
    return queue.watch();
  });

  /** What the table knows of a row's lines: only the changeset on view has them. */
  const ownOf = (row: QueueRow) => (keyId(row.key) === onViewId ? own : undefined);

  /** The selected row is the changeset on view: choosing one puts it on view. */
  function select(row: QueueRow) {
    if (keyId(row.key) !== onViewId) commands.run(`changeset.open:${keyId(row.key)}`);
  }

  /** Puts the changeset on view and shows its review. */
  function openRow(row: QueueRow) {
    select(row);
    onopen();
  }

  let highlighted = $state(-1);
  let tableBody = $state<HTMLElement>();
  const nav = new ListNav<QueueRow>({
    element: () => tableBody,
    items: () => shown,
    single: select,
    double: openRow,
    highlight: { get: () => highlighted, set: (i) => (highlighted = i) },
    reveal: (i) => tableBody?.querySelectorAll<HTMLElement>(".row")[i]?.scrollIntoView({ block: "nearest" }),
  });
  $effect(() => () => nav.cancel());

  /** A right click selects the row at once, and opens its menu. */
  function openMenu(e: MouseEvent, row: QueueRow, index: number) {
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

  /** The Details tab's link to the base-changed lines: the review shows them. */
  async function openBaseChanged() {
    await changeset.showBaseChanged();
    onopen();
  }
</script>

<section class="queue" aria-label="Queue">
  <div class="bar">
    <PanelTabs
      label="Changesets"
      value={tab}
      onchange={(id) => {
        tab = id;
        writeStored(TAB_KEY, id);
      }}
      tabs={TABS.map((t) => ({ ...t, count: counts[t.id] }))}
    />
    <span class="grow"></span>
    <label class="search">
      <Icon name="search" size={13} />
      <input class="field" type="text" bind:value={query} aria-label="Search changesets" spellcheck="false" />
    </label>
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
      {@const id = keyId(row.key)}
      {@const numbers = ownOf(row)}
      {@const changes = changesOf(row, numbers?.layout.delta, queue.lines.get(id))}
      {@const author = authorOf(row)}
      {@const comments = commentCount(row)}
      <button
        class="flat row"
        class:on={id === onViewId}
        class:cur={index === highlighted}
        type="button"
        tabindex="-1"
        aria-current={id === onViewId}
        onclick={(e) => nav.click(e, index)}
        oncontextmenu={(e) => openMenu(e, row, index)}
      >
        <span class="title"><ChangesetName label={labelOf(row)} /></span>
        <span
          ><StatusChip status={row.pullRequest?.status} title={row.pullRequest && statusTitle(row.pullRequest)} /></span
        >
        <span class="author" class:muted={author?.muted}>
          {#if author}
            {#if !author.muted}<Avatar name={author.name} url={author.avatar ?? null} />{/if}
            <span class="name" title={author.name}>{author.name}</span>
          {/if}
        </span>
        <span class="source">{sourceText(row)}</span>
        <span class="delta"><ChangesCell {changes} /></span>
        <span><ReviewedCell reviewed={reviewedOf(row.review, numbers?.facts.completion)} /></span>
        <span class="icons">
          <CiStatus state={row.pullRequest?.ci} />
          {#if comments > 0}
            <span
              class="threads"
              title={row.pullRequest
                ? `${comments} ${comments === 1 ? "comment" : "comments"}`
                : `${comments} open ${comments === 1 ? "thread" : "threads"}`}
              ><Icon name="comments" size={13} />{comments}</span
            >
          {/if}
        </span>
        <span class="right when">{updatedText(row)}</span>
      </button>
    {:else}
      {#if !unlisted.length}
        <p class="empty">
          {queue.loading ? "Loading…" : rows.length ? "No matching changesets" : "No changesets"}
        </p>
      {/if}
    {/each}
    {#if shown.length && queue.listingPullRequests && (tab === "all" || tab === "prs")}
      <p class="pending">Loading pull requests…</p>
    {/if}
    {#each unlisted as source (source.remote)}
      <p class="unlisted">
        {source.remote}: {source.error}
        {#if source.signedOut}<button class="link" type="button" onclick={() => commands.run("app.accounts")}
            >Sign in…</button
          >{/if}
      </p>
    {/each}
  </Table>

  <div class="panel">
    <PanelTabs
      label="Selected changeset"
      value={panel}
      onchange={(id) => {
        panel = id;
        writeStored(PANEL_KEY, id);
      }}
      tabs={[
        { id: "details", label: "Details" },
        { id: "files", label: "Files", count: queue.onViewPaths.length },
        { id: "comments", label: "Comments", count: threads.items.length },
      ]}
    />
    <div class="body">
      {#if !current}
        <p class="empty">Select a changeset</p>
      {:else if panel === "details"}
        <ScrollArea>
          <QueueDetails groups={detailGroups(current, own)} onlink={() => void openBaseChanged()} />
        </ScrollArea>
      {:else if panel === "files"}
        <!-- The list scrolls by itself. -->
        <QueueFiles paths={queue.onViewPaths} fileOf={(path) => queue.onViewFile(path)} onopen={onopenfile} />
      {:else}
        <!-- The list scrolls by itself. -->
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
    width: 260px;
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
    padding-left: 26px;
  }
  .row.on {
    background: var(--toggle);
  }
  /* The highlighted row, which Space and Enter act on, while the list has the focus. */
  :global([data-list]:focus-within) .row.cur {
    background: var(--select);
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
  .link {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
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
  }
  .empty {
    margin: 12px 16px;
    color: var(--text-3);
  }
</style>
