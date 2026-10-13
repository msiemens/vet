<script lang="ts">
  import type { CommitInfo } from "../../generated/CommitInfo";
  import type { VersionInfo } from "../../generated/VersionInfo";
  import BottomToolHeader from "../../lib/ui/BottomToolHeader.svelte";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import LineDelta from "../../lib/ui/LineDelta.svelte";
  import PathText from "../../lib/ui/PathText.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import Table from "../../lib/ui/Table.svelte";
  import type { Column } from "../../lib/ui/columns";
  import { type Sort, sortRows } from "../../lib/ui/sorting";
  import { checks } from "../../state/checks.svelte";
  import { type Range, chooseRow, defaultRange } from "../../state/timelineRange";
  import { type RangeFile, type TimelineRow, baseRow, timelineRows, timelineSummary } from "./timelineRows";

  let {
    versions,
    base: current,
    range,
    byCommit = false,
    files,
    selected,
    onrange,
    onselect,
    onclose,
  }: {
    /** Oldest first. */
    versions: readonly VersionInfo[];
    /** The base the changeset is compared with now. */
    base?: CommitInfo;
    /** The range the review view shows; none for the whole changeset. */
    range?: Range;
    /** A commit is on view, which choosing a row leaves. */
    byCommit?: boolean;
    /** The files the range changes. */
    files: RangeFile[];
    /** The file the review view is at. */
    selected?: string;
    /** Shows these pushes; none shows the whole changeset. */
    onrange: (range: Range | undefined) => void;
    onselect: (path: string) => void;
    onclose: () => void;
  } = $props();

  const COLUMNS: Column<TimelineRow>[] = [
    { id: "version", label: "Version", width: 56 },
    { id: "time", label: "Time", width: 96, min: 50, sortBy: (row) => row.at, firstSort: "descending" },
    { id: "files", label: "Files", flex: true, min: 80 },
    { id: "ci", label: "", width: 14, fixed: true },
    { id: "lines", label: "Lines", width: 70, align: "right" },
  ];

  let sort = $state<Sort>();
  const shown = $derived({ range, byCommit });
  const rows = $derived(sortRows(timelineRows(versions, shown), COLUMNS, sort));
  const base = $derived(baseRow(versions, current, shown));
  const since = $derived(defaultRange(versions));
  /** The version clicked last, which a shift-click runs the block from. */
  let anchor = $state<number>();

  const time = (at: number) => new Date(at * 1000).toLocaleTimeString(undefined, { hour12: false });

  function choose(n: number, shift: boolean) {
    // A commit on view is replaced by the push, whatever was shown before it.
    const current = byCommit ? undefined : range;
    if (!shift) anchor = n;
    onrange(chooseRow(versions, current, n, shift ? (anchor ?? current?.to) : undefined));
  }
</script>

<section aria-label="Timeline" tabindex="-1" data-escapes-scope>
  <BottomToolHeader
    title="Timeline"
    count={versions.length}
    summary={timelineSummary(shown, versions.at(-1)?.n)}
    {onclose}
  >
    {#snippet controls()}
      <button
        class="ctl"
        type="button"
        title="Show the pushes after the version you last reviewed"
        disabled={!since}
        onclick={() => onrange(since)}>Since my review</button
      >
    {/snippet}
  </BottomToolHeader>
  <div class="body">
    <div class="versions">
      <Table
        name="timeline"
        columns={COLUMNS}
        bind:sort
        --table-pad="14px"
        --table-pad-end="12px"
        --table-row="26px"
        --table-head="24px"
        --table-head-color="var(--text-3)"
        --table-head-size="var(--fs-mono-s)"
      >
        <ul class="rows" aria-label="Versions">
          {#each rows as row (row.n)}
            <li>
              <button
                class="flat row"
                class:shown={row.shown}
                type="button"
                aria-current={row.shown ? "true" : undefined}
                onclick={(e) => choose(row.n, e.shiftKey)}
              >
                <span class="mono n"
                  ><span class="slot"
                    >{#if row.ring}<span class="ring" title="The changes shown are compared with this version"
                      ></span>{/if}</span
                  >v{row.n}</span
                >
                <span class="mono time">{time(row.at)}</span>
                <span class="names">
                  {row.files}
                  {#if row.reviewed}<span class="eye" title="Last reviewed here"><Icon name="eye" size={13} /></span
                    >{/if}
                </span>
                <span class="ci"><CiStatus state={checks.of(row.commit)?.state} size={12} /></span>
                <span class="right">
                  {#if row.lines}<LineDelta {...row.lines} />{:else}<span class="none">–</span>{/if}
                </span>
              </button>
            </li>
          {:else}
            <li class="empty">Versions start when you first mark something seen, acknowledge a change or comment.</li>
          {/each}
          {#if base}
            <li>
              <div class="row base" aria-label="The base the review started from">
                <span class="mono n"
                  ><span class="slot"
                    >{#if base.ring}<span class="ring" title="The changes shown are compared with the base"
                      ></span>{/if}</span
                  >base</span
                >
                <span class="mono time">{time(base.commit.time)}</span>
                <span class="names"
                  ><span class="mono">{base.commit.shortId}</span>
                  <span class="subject">{base.commit.subject}</span></span
                >
                <span></span>
                <span></span>
              </div>
            </li>
          {/if}
        </ul>
      </Table>
    </div>
    <div class="files">
      <ScrollArea>
        <ul class="list" aria-label="Files changed in the range">
          {#each files as file (file.path)}
            <li>
              <button
                class="flat file"
                class:on={file.path === selected}
                type="button"
                onclick={() => onselect(file.path)}
              >
                <span class="path"><PathText path={file.path} /></span>
                {#if file.mode}<span class="mono mode">{file.mode}</span>{/if}
                <LineDelta {...file.delta} />
              </button>
            </li>
          {/each}
        </ul>
      </ScrollArea>
    </div>
  </div>
</section>

<style>
  section {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
    background: var(--bg-panel);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .versions {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
  }
  .rows,
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    font-size: var(--fs-ctl);
  }
  .row.shown {
    box-shadow: inset 3px 0 var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .row.shown:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .row.base {
    display: grid;
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .n {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .slot {
    flex: none;
    width: 9px;
    height: 9px;
  }
  .ring {
    display: block;
    width: 9px;
    height: 9px;
    box-sizing: border-box;
    border: 1.5px solid var(--accent);
    border-radius: 50%;
  }
  .subject {
    color: var(--text-3);
  }
  .right {
    text-align: right;
  }
  .ci {
    display: inline-flex;
    justify-content: center;
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
    display: flex;
    flex-direction: column;
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
    display: flex;
  }
  .mode {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
</style>
