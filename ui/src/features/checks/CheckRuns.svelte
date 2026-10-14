<!-- The runs of a commit's checks, one row each: state, name, how long it ran and, for a failed one
     with a log, a link to it. -->
<script lang="ts">
  import type { CheckRun } from "../../generated/CheckRun";
  import { ciIcon } from "../../lib/ciStatus";
  import { durationText } from "../../lib/format";
  import CiStatus from "../../lib/ui/CiStatus.svelte";

  let {
    runs,
    onlog,
    markFailed = false,
  }: {
    runs: readonly CheckRun[];
    /** Shows the log of a failed run. */
    onlog: (run: CheckRun) => void;
    /** The names of failed runs take the failure colour. */
    markFailed?: boolean;
  } = $props();
</script>

{#each runs as run (run.id)}
  <div class="run">
    <span class="state">
      {#if ciIcon(run.state)}<CiStatus state={run.state} size={12} />{:else}<span class="none" title={run.state}>–</span
        >{/if}
    </span>
    <span class="name" class:failed={markFailed && run.state === "failure"} title={run.name}>{run.name}</span>
    {#if run.duration !== undefined}<span class="mono duration">{durationText(run.duration)}</span>{/if}
    {#if run.state === "failure" && run.log}
      <button class="link" type="button" onclick={() => onlog(run)}>Log</button>
    {/if}
  </div>
{/each}

<style>
  .run {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    height: 28px;
    font-size: var(--fs-ctl);
  }
  .state {
    display: inline-flex;
    justify-content: center;
    width: 12px;
  }
  .none {
    color: var(--text-3);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .failed {
    color: var(--del-text);
  }
  .duration {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
</style>
