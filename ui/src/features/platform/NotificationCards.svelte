<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { platform } from "../../state/platform.svelte";
  import { copyText } from "./copy";

  const cards = $derived(platform.notified.filter((n) => n.card));

  function signIn(id: number) {
    platform.dismiss(id);
    platform.openAccounts();
  }
</script>

<div class="cards" aria-live="polite">
  {#each cards as item (item.id)}
    <div class="surface card" role="status">
      <div class="top">
        <span class="text">{item.notification.text}</span>
        <IconButton icon="close" label="Dismiss" size={20} iconSize={12} onclick={() => platform.dismiss(item.id)} />
      </div>
      <div class="buttons">
        {#if item.notification.copyText}
          <button class="ctl small" type="button" onclick={() => void copyText(item.notification.copyText ?? "")}>
            Copy text
          </button>
        {/if}
        {#if item.notification.kind === "syncStopped"}
          <button class="ctl small" type="button" onclick={() => signIn(item.id)}>Sign in…</button>
        {/if}
      </div>
    </div>
  {/each}
</div>

<style>
  .cards {
    position: fixed;
    right: 12px;
    bottom: calc(var(--h-footer) + 12px);
    z-index: 55;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 320px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 8px 8px 12px;
    font-size: var(--fs-ctl);
  }
  .top {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .buttons {
    display: flex;
    gap: 6px;
  }
  .small {
    height: 22px;
  }
</style>
