<script lang="ts">
  import { holdWhileUsed } from "../../lib/ui/holdTimer";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { commands } from "../../state/commands";
  import { platform } from "../../state/platform.svelte";
  import { copyText } from "./copy";
  import { notificationCopyText, notificationText } from "./notificationText";

  const cards = $derived(platform.notified.filter((n) => n.card));

  function signIn(id: number) {
    platform.dismiss(id);
    commands.run("app.accounts");
  }
</script>

<div class="cards" aria-live="polite">
  {#each cards as item (item.id)}
    <div
      class="surface card"
      role="status"
      {@attach holdWhileUsed(
        () => platform.holdCard(item.id),
        () => platform.releaseCard(item.id),
      )}
    >
      <div class="top">
        <span class="text selectable">{notificationText(item.notification)}</span>
        <IconButton icon="close" label="Dismiss" size="xs" iconSize={12} onclick={() => platform.dismiss(item.id)} />
      </div>
      <div class="buttons">
        {#if notificationCopyText(item.notification)}
          <button
            class="ctl small"
            type="button"
            onclick={() => void copyText(notificationCopyText(item.notification) ?? "")}
          >
            Copy text
          </button>
        {/if}
        {#if item.notification.type === "syncStopped"}
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
</style>
