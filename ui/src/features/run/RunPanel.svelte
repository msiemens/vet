<script lang="ts">
  import { formatBytes } from "../../lib/formatBytes";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Tab from "../../lib/ui/Tab.svelte";
  import { type Run, runs } from "../../state/runs.svelte";
  import { LogView } from "./LogView";

  const run = $derived(runs.current);
  // The view belongs to the log; a state change of the run keeps it.
  const log = $derived(run?.log);

  const label = (r: Run) => runs.target(r.targetId)?.target.label ?? `Run ${r.id}`;
  const dot = (r: Run) => (!r.exit ? "ok" : r.exit.code === 0 ? "done" : "bad");

  function status(r: Run): string {
    if (!r.exit) return r.stopping ? "Stopping…" : "Running";
    return r.exit.code === null ? "Stopped" : `Exited with code ${r.exit.code}`;
  }
</script>

<section aria-label="Run output">
  <div class="strip">
    {#each runs.runs as r (r.id)}
      <Tab selected={r.id === runs.shown} onselect={() => (runs.shown = r.id)}>
        <span class="dot {dot(r)}"></span>{label(r)}
      </Tab>
    {/each}
    <span class="grow"></span>
    {#if run && !run.exit}
      <button class="ctl stop" type="button" disabled={run.stopping} onclick={() => void runs.stop(run.id)}>
        <Icon name="stop" size={10} />Stop
      </button>
    {/if}
    <IconButton icon="close" label="Close run output" size={22} iconSize={12} onclick={() => (runs.panel = false)} />
  </div>

  {#if run}
    <div class="meta">
      <span class="status {dot(run)}">{status(run)}</span>
      {#if run.snapshot && run.changedSinceReview}
        <span class="warn">
          {run.exit ? "Ran" : "Running"} snapshot <span class="mono">{run.snapshot.slice(0, 8)}</span> — {run.changedSinceReview}
          {run.changedSinceReview === 1 ? "file" : "files"} changed since review
        </span>
      {:else if run.snapshot}
        <span>Snapshot <span class="mono">{run.snapshot.slice(0, 8)}</span></span>
      {/if}
    </div>
    {#if run.exit?.droppedBytes}
      <p class="notice">
        Output past the backend's limit was dropped: {formatBytes(run.exit.droppedBytes)} are missing from the end.
      </p>
    {/if}
  {/if}
  <div
    class="log selectable mono"
    {@attach (el) => {
      if (!log) return;
      const view = new LogView(el, log);
      return () => view.destroy();
    }}
  ></div>
</section>

<style>
  section {
    height: var(--h-run);
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--bg-editor);
    border-top: 1px solid var(--line);
  }
  .strip {
    display: flex;
    align-items: center;
    gap: 4px;
    height: var(--h-tab);
    padding-right: 6px;
    flex: none;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--line);
  }
  .grow {
    flex: 1;
  }
  .stop {
    height: 22px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .dot.ok {
    background: var(--ok);
  }
  .dot.done {
    background: var(--text-3);
  }
  .dot.bad {
    background: var(--warn);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: none;
    padding: 4px 12px;
    font-size: var(--fs-ctl);
    color: var(--text-2);
    border-bottom: 1px solid var(--line-soft);
  }
  .meta .mono {
    font-size: var(--fs-mono-s);
  }
  .status.ok {
    color: var(--ok);
  }
  .status.bad,
  .warn {
    color: var(--warn);
  }
  .notice {
    margin: 0;
    padding: 4px 12px;
    flex: none;
    font-size: var(--fs-ctl);
    color: var(--warn);
    background: var(--bg-hunk);
    border-bottom: 1px solid var(--line-soft);
  }
  .log {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 6px 12px;
    font-size: var(--fs-mono);
    line-height: var(--h-line);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .log:empty::before {
    content: "No output yet";
    color: var(--text-3);
  }
  .log:global([data-trimmed])::before {
    content: "… earlier output was trimmed";
    display: block;
    color: var(--text-3);
  }
  .log :global(.err) {
    color: var(--del-text);
  }
</style>
