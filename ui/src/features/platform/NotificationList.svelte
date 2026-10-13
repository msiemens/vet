<script lang="ts">
  import { commands } from "../../state/commands";
  import { platform } from "../../state/platform.svelte";
  import { shortTime } from "../../lib/shortTime";
  import { copyText } from "./copy";

  let { onclose }: { onclose: () => void } = $props();
</script>

<div class="surface list" role="dialog" aria-label="Notifications">
  <div class="head">
    <span>Notifications</span>
    <button class="flat" type="button" onclick={onclose}>Close</button>
  </div>
  <ul>
    {#each platform.notified as item (item.id)}
      <li>
        <span class="text selectable">{item.notification.text}</span>
        <span class="time">{shortTime(item.at)}</span>
        {#if item.notification.copyText}
          <button class="flat" type="button" onclick={() => void copyText(item.notification.copyText ?? "")}>
            Copy text
          </button>
        {/if}
        {#if item.notification.kind === "syncStopped"}
          <button class="flat" type="button" onclick={() => commands.run("app.accounts")}>Sign in…</button>
        {/if}
      </li>
    {/each}
  </ul>
</div>

<style>
  .list {
    position: absolute;
    right: 0;
    bottom: calc(var(--h-footer) - 6px);
    z-index: 60;
    width: 360px;
    max-height: 50vh;
    overflow: auto;
    font-size: var(--fs-ctl);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line-soft);
    font-weight: 600;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--line-soft);
  }
  .text {
    flex: 1 0 100%;
  }
  .time {
    color: var(--text-3);
  }
</style>
