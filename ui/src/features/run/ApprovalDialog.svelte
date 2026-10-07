<script lang="ts">
  import type { ApprovalFile } from "../../generated/ApprovalFile";
  import type { RunChange } from "../../generated/RunChange";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import type { Approval } from "../../state/runs.svelte";
  import { runs } from "../../state/runs.svelte";

  let { approval }: { approval: Approval } = $props();

  let expanded = $state<Record<string, boolean>>({});
  let busy = $state(false);

  const target = $derived(runs.target(approval.targetId)?.target);
  // The diff replaces the plain list: it pairs a rename into one entry.
  const entries = $derived<{ file?: ApprovalFile; path: string; change: RunChange }[]>(
    approval.files?.map((file) => ({ file, path: file.path, change: file.change })) ?? approval.changed,
  );

  const verb: Record<RunChange, string> = { added: "added", modified: "modified", removed: "removed" };

  async function approve() {
    busy = true;
    try {
      await runs.approve();
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Run check" width="640px" onclose={() => (runs.approval = undefined)}>
  <div class="head">
    <span class="icon"><Icon name="build" size={26} stroke={1.6} /></span>
    <div class="titles">
      <h3>
        {approval.firstRun ? "Run files not approved yet" : "Build files changed since the last run"}
      </h3>
      {#if target}<span class="mono sub">{target.label} · {target.command}</span>{/if}
    </div>
  </div>
  <p class="why">
    {#if approval.firstRun}
      No run was approved in this repository yet. These files decide what the run executes; look at them before running.
    {:else}
      These files decide what the run executes and changed since the last run you approved. Look at them before running.
    {/if}
  </p>

  <div class="files">
    {#each entries as entry, i (entry.path)}
      {@const file = entry.file}
      {@const open = expanded[entry.path] ?? i === 0}
      {@const counts = file?.delta}
      <div class="file">
        <button
          type="button"
          class="flat head-row"
          aria-expanded={open}
          disabled={!file}
          onclick={() => (expanded[entry.path] = !open)}
        >
          <span class="chev"><Icon name={open ? "open" : "closed"} size={10} /></span>
          <span class="path"
            >{#if file?.from}<span class="from">{file.from}</span><span class="arrow">→</span>{/if}{entry.path}</span
          >
          <span class="kind">{file?.from ? "renamed" : verb[entry.change]}</span>
          {#if counts}
            <span class="mono counts"
              ><span class="plus">+{counts.added}</span> <span class="minus">−{counts.removed}</span></span
            >
          {/if}
        </button>
        {#if file && open}
          {@const shown = file.diff}
          {#if !shown}
            <p class="none">No text to compare.</p>
          {:else if !shown.rows.length}
            <p class="none">{file.from ? "Moved without changes to its content." : "No line changes."}</p>
          {:else}
            <div class="diff selectable">
              {#each shown.rows as row, r (r)}
                {#if row.type === "gap"}
                  <div class="dl gap"><span class="n"></span><span class="s"></span><span>⋯</span></div>
                {:else}
                  <div class="dl {row.type}">
                    <span class="n">{row.number}</span>
                    <span class="s">{row.type === "added" ? "+" : row.type === "removed" ? "−" : ""}</span>
                    <span class="t">{row.text}</span>
                  </div>
                {/if}
              {/each}
              {#if shown.truncated}<p class="none">Longer than shown.</p>{/if}
            </div>
          {/if}
        {/if}
      </div>
    {/each}
    {#if approval.failed}<p class="none">The text of these files could not be loaded: {approval.failed}</p>{/if}
  </div>

  {#snippet actions()}
    <button class="ctl" type="button" onclick={() => (runs.approval = undefined)}>Cancel</button>
    <button class="ctl primary" type="button" disabled={busy} onclick={approve}>Run</button>
  {/snippet}
</Dialog>

<style>
  .head {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  .icon {
    display: inline-flex;
    flex: none;
    color: var(--warn);
  }
  .titles {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  h3 {
    margin: 0;
    font-size: var(--fs);
    font-weight: 600;
  }
  .sub {
    font-size: var(--fs-mono-s);
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .why {
    margin: 12px 0;
    color: var(--text-2);
    font-size: var(--fs-ctl);
  }
  .files {
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--bg-editor);
    overflow: hidden;
  }
  .file + .file {
    border-top: 1px solid var(--line-soft);
  }
  .head-row {
    width: 100%;
    height: var(--h-ctl);
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    text-align: left;
    font-size: var(--fs-ctl);
  }
  .chev {
    display: inline-flex;
    color: var(--text-2);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .from {
    color: var(--text-3);
  }
  .arrow {
    margin: 0 6px;
    color: var(--text-3);
  }
  .kind {
    color: var(--text-3);
    font-size: 12px;
  }
  .counts {
    font-size: 11px;
  }
  .plus {
    color: var(--add-text);
  }
  .minus {
    color: var(--del-text);
  }
  .diff {
    overflow-x: auto;
    border-top: 1px solid var(--line-soft);
  }
  .dl {
    display: grid;
    grid-template-columns: 40px 16px minmax(max-content, 1fr);
    font: var(--fs-mono) / var(--h-line) var(--font-mono);
    white-space: pre;
  }
  .n {
    padding-right: 6px;
    text-align: right;
    color: var(--text-3);
  }
  .s {
    text-align: center;
    color: var(--text-3);
  }
  .dl.added {
    background: var(--add-bg);
  }
  .dl.removed {
    background: var(--del-bg);
  }
  .dl.added .s {
    color: var(--add-text);
  }
  .dl.removed .s {
    color: var(--del-text);
  }
  .gap {
    color: var(--text-3);
  }
  .none {
    margin: 0;
    padding: 6px 12px;
    color: var(--text-3);
    font-size: 12px;
  }
</style>
