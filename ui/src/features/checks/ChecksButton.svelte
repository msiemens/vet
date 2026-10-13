<script lang="ts">
  import type { CommitChecks } from "../../generated/CommitChecks";
  import { ciIcon } from "../../lib/ciStatus";
  import CiStatus from "../../lib/ui/CiStatus.svelte";
  import SeverityIcon from "../../lib/ui/SeverityIcon.svelte";
  import { badgeCount } from "./checkRows";

  /** The head commit's combined checks in the toolbar; a click shows the Pipelines tool. */
  let {
    checks,
    title,
    stale = false,
    warn = false,
    onopen,
  }: {
    checks: CommitChecks;
    /** What the checks say in words, for the tooltip. */
    title: string;
    /** The platform limits requests, so what is shown may be old. */
    stale?: boolean;
    /** The platform cannot be asked (signed out, token expired): no result to show. */
    warn?: boolean;
    onopen: () => void;
  } = $props();
</script>

{#if warn || ciIcon(checks.state)}
  <button class="ctl status" class:stale type="button" {title} onclick={onopen}>
    {#if warn}
      <SeverityIcon severity="warning" />
    {:else}
      <CiStatus state={checks.state} size={12} {title} />
      <span class="mono">{badgeCount(checks)}</span>
    {/if}
  </button>
{/if}

<style>
  .status {
    margin-right: 6px;
  }
  .stale {
    opacity: 0.65;
  }
</style>
