<script lang="ts">
  import { untrack } from "svelte";
  import { readStored, writeStored } from "../../lib/storage";
  import Chip from "../../lib/ui/Chip.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import SwitcherPopover from "../../lib/ui/SwitcherPopover.svelte";
  import { keyId } from "../../state/changesetKey";
  import { commands } from "../../state/commands";
  import { queue } from "../../state/queue.svelte";
  import ChangesCell from "./ChangesCell.svelte";
  import ChangesetName from "./ChangesetName.svelte";
  import {
    type Label,
    SWITCHER_FILTERS,
    changesOf,
    commentCount,
    filterRows,
    labelOf,
    parseTab,
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
  }: {
    /** What the changeset on view is called; unknown until the manifest arrives. */
    label?: Label;
    /** The changeset on view is a draft pull request. */
    draft?: boolean;
    title?: string;
    open?: boolean;
  } = $props();

  const FILTER_KEY = "vet.switcher.filter";

  let query = $state("");
  let filter = $state(parseTab(readStored(FILTER_KEY)));

  const rows = $derived(queueRows(queue.local, queue.pullRequests));
  const shown = $derived(filterRows(rows, filter, query));
  const onViewId = $derived(queue.onView && keyId(queue.onView));
  const own = $derived(queue.onViewFrame);

  // The changesets are listed afresh each time the popover opens.
  $effect(() => {
    if (!open) return;
    untrack(() => void queue.loadAll());
    return queue.watch();
  });

  /** An action under the rows that runs a command. */
  const action = (id: string, text: string) => ({
    label: text,
    key: commands.binding(id)?.label,
    title: commands.hint(id),
    disabled: !commands.isEnabled(id),
    run: () => commands.run(id),
  });
</script>

<SwitcherPopover
  bind:open
  bind:query
  label="Search changesets"
  items={shown}
  key={(row) => keyId(row.key)}
  isOn={(row) => keyId(row.key) === onViewId}
  listLabel="Changesets"
  empty={rows.length ? "No matching changesets" : "No changesets"}
  actions={[action("view.queue", "Open queue"), action("changeset.openPullRequest", "Open pull request…")]}
  onchoose={(row) => commands.run(`changeset.open:${keyId(row.key)}`)}
>
  {#snippet trigger(toggle)}
    <Chip active={open} {title} onclick={toggle}>
      <Icon name="branch" />{#if label}<span class="current"><ChangesetName {label} /></span>{:else}…{/if}
      {#if draft}<span class="badge">Draft</span>{/if}
    </Chip>
  {/snippet}
  {#snippet filters(reset)}
    <Segmented
      small
      label="Show"
      options={SWITCHER_FILTERS}
      value={filter}
      onchange={(id) => {
        filter = id;
        writeStored(FILTER_KEY, id);
        reset();
      }}
    />
  {/snippet}
  {#snippet row(changeset)}
    {@const id = keyId(changeset.key)}
    {@const numbers = id === onViewId ? own : undefined}
    {@const changes = changesOf(changeset, numbers?.layout.delta, queue.lines.get(id))}
    {@const comments = commentCount(changeset)}
    <span class="row">
      <span class="line">
        <span class="name"><ChangesetName label={labelOf(changeset)} /></span>
        <ReviewedCell reviewed={reviewedOf(changeset.review, numbers?.facts.completion)} width={48} />
      </span>
      <span class="line sub">
        <span class="detail">
          {sourceText(changeset)}
          <ChangesCell {changes} />
          {#if comments > 0}<span class="threads"><Icon name="comments" size={11} />{comments}</span>{/if}
        </span>
        <span>{updatedText(changeset)}</span>
      </span>
    </span>
  {/snippet}
</SwitcherPopover>

<style>
  .row {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 4px 8px;
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
  .current {
    display: block;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
