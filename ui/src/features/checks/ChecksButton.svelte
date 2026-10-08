<script lang="ts">
  import type { CheckRun } from "../../generated/CheckRun";
  import type { CommitChecks } from "../../generated/CommitChecks";
  import { ciIcon } from "../../lib/ciStatus";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { errorText } from "../../lib/errorText";
  import { openUrl } from "../../state/host";
  import { session } from "../../state/session.svelte";
  import { badgeCount, durationText } from "./checkRows";

  /** The head commit's combined checks in the toolbar; a click lists the runs. */
  let {
    checks,
    shortId,
    onlog,
  }: {
    checks: CommitChecks;
    /** The commit's short id, as the popover names it. */
    shortId: string;
    /** Shows the log of a failed run. */
    onlog: (run: CheckRun) => void;
  } = $props();

  let open = $state(false);
  const look = $derived(ciIcon(checks.state));
  const title = $derived(look && `${look.title}: ${checks.runs.length} ${checks.runs.length === 1 ? "run" : "runs"}`);

  async function openInBrowser(url: string) {
    open = false;
    try {
      await openUrl(url);
    } catch (error) {
      session.notice = `Could not open the browser: ${errorText(error)}`;
    }
  }
</script>

{#if look}
  <Popover bind:open align="right">
    {#snippet trigger(toggle)}
      <button class="ctl status" type="button" aria-expanded={open} {title} onclick={toggle}>
        <CiStatus state={checks.state} size={12} {title} />
        <span class="mono">{badgeCount(checks)}</span>
      </button>
    {/snippet}
    <div class="runs">
      <div class="head">
        <span class="mono commit">{shortId}</span>
        {#if checks.url}
          {@const url = checks.url}
          <button class="link" type="button" onclick={() => void openInBrowser(url)}>Open in browser</button>
        {/if}
      </div>
      {#each checks.runs as run (run.id)}
        <div class="run">
          <span class="state">
            {#if ciIcon(run.state)}<CiStatus state={run.state} size={12} />{:else}<span class="none" title={run.state}
                >–</span
              >{/if}
          </span>
          <span class="name" title={run.name}>{run.name}</span>
          {#if run.duration !== undefined}<span class="mono duration">{durationText(run.duration)}</span>{/if}
          {#if run.state === "failure" && run.log}
            <button
              class="link"
              type="button"
              onclick={() => {
                open = false;
                onlog(run);
              }}>Log</button
            >
          {/if}
        </div>
      {/each}
    </div>
  </Popover>
{/if}

<style>
  .status {
    margin-right: 6px;
  }
  .runs {
    min-width: 300px;
    max-width: 440px;
    padding: 4px 0;
    font-size: var(--fs-ctl);
  }
  .head,
  .run {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    height: 28px;
  }
  .head {
    justify-content: space-between;
    color: var(--text-3);
  }
  .commit {
    font-size: var(--fs-mono-s);
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
  .duration {
    color: var(--text-3);
    font-size: var(--fs-mono-s);
  }
  .link {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
