<script lang="ts">
  import type { LinkState } from "../state/backend";

  let {
    link,
    canRestart,
    onrestart,
  }: {
    link: LinkState;
    canRestart: boolean;
    onrestart: () => Promise<void>;
  } = $props();

  let restarting = $state(false);

  async function restart() {
    restarting = true;
    try {
      await onrestart();
    } finally {
      restarting = false;
    }
  }
</script>

<!-- Connecting again says nothing: only a connection that does not come back for long is told. -->
{#if link === "stalled" || restarting}
  <div class="surface notice" role="status">
    <span>{restarting ? "Restarting the backend…" : "The backend does not answer."}</span>
    {#if canRestart}
      <button class="flat" type="button" disabled={restarting} onclick={() => void restart()}>Restart backend</button>
    {/if}
  </div>
{/if}

<style>
  .notice {
    position: fixed;
    left: 50%;
    bottom: calc(var(--h-footer) + 52px);
    transform: translateX(-50%);
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    font-size: var(--fs-ctl);
    color: var(--text-2);
  }
  button {
    padding: 0 6px;
    border-radius: var(--radius);
  }
</style>
