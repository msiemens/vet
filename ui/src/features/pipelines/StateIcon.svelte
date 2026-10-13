<script lang="ts">
  // How far a job, step or run has come and how it went: the icon CiStatus draws for the commit's
  // checks, with a phase of its own for what waits and what was stopped.
  import type { CheckPhase } from "../../generated/CheckPhase";
  import type { CiState } from "../../generated/CiState";
  import Icon from "../../lib/ui/Icon.svelte";
  import type { IconName } from "../../lib/ui/icons";
  import { type Look, look, lookTitle } from "./pipelineRows";

  let { state, phase, size = 12 }: { state: CiState; phase: CheckPhase; size?: number } = $props();

  const LOOKS: Record<Look, { icon: IconName; tone: string }> = {
    success: { icon: "check", tone: "ok" },
    failure: { icon: "close", tone: "failing" },
    running: { icon: "timeline", tone: "running" },
    queued: { icon: "timeline", tone: "waiting" },
    cancelled: { icon: "minus", tone: "waiting" },
    skipped: { icon: "minus", tone: "waiting" },
    approval: { icon: "lock", tone: "running" },
  };
  const current = $derived(look(state, phase));
  const shown = $derived(LOOKS[current]);
</script>

<span class="state {shown.tone}" title={lookTitle(current)}><Icon name={shown.icon} {size} stroke={2} /></span>

<style>
  .state {
    display: inline-flex;
    flex: none;
    width: 14px;
    justify-content: center;
  }
  .ok {
    color: var(--add-text);
  }
  .failing {
    color: var(--del-text);
  }
  .running {
    color: var(--old-base);
  }
  .waiting {
    color: var(--text-3);
  }
</style>
